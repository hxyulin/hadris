//! A directory record that does not parse ends the directory listing instead of
//! being reported again and again: the iterator used to stay at the same offset after
//! an error, so `entries()` yielded the same `Err` forever.

use std::io::Cursor;
use std::sync::Arc;

use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::write::options::{CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};

fn write_bytes(files: Vec<IsoFile>) -> Vec<u8> {
    let options = IsoFormatOptions {
        volume_name: "BROKEN".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::default(),
        strict_charset: false,
    };
    let input = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files,
    };
    let mut buffer = Cursor::new(vec![0u8; 2 * 1024 * 1024]);
    IsoImageWriter::create(&mut buffer, input, options).expect("write ISO");
    buffer.into_inner()
}

fn file(name: &str) -> IsoFile {
    IsoFile::File {
        name: Arc::new(name.to_string()),
        contents: name.as_bytes().to_vec(),
    }
}

/// Byte offset of the `index`-th record of the root directory (0 and 1 are `.` and
/// `..`).
fn root_record_offset(image: &[u8], index: usize) -> usize {
    let pvd = 16 * 2048;
    let root = &image[pvd + 156..pvd + 190];
    let extent = u32::from_le_bytes([root[2], root[3], root[4], root[5]]) as usize;
    let mut offset = extent * 2048;
    for _ in 0..index {
        offset += image[offset] as usize;
    }
    offset
}

#[test]
fn a_record_that_does_not_parse_ends_the_directory_instead_of_repeating() {
    let mut bytes = write_bytes(vec![file("A.TXT"), file("B.TXT"), file("C.TXT")]);
    // Make the first file's record inconsistent: its big-endian extent no longer
    // matches the little-endian one, which the parser rejects.
    let broken = root_record_offset(&bytes, 2);
    bytes[broken + 6] ^= 0xFF;

    let image = IsoImage::open(Cursor::new(bytes)).expect("open ISO");
    let root = image.root_dir();
    let results: Vec<_> = image.open_dir(root.dir_ref()).entries().take(64).collect();

    assert!(
        results.len() <= 3,
        "the iterator kept going after the bad record: {} results",
        results.len()
    );
    assert!(
        results.last().is_some_and(|r| r.is_err()),
        "the bad record was not reported"
    );
    assert!(
        results[..results.len() - 1].iter().all(|r| r.is_ok()),
        "the records before the bad one are fine"
    );
}
