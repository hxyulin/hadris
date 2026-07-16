#![cfg(all(feature = "std", feature = "sync"))]

use std::io::{Cursor, Seek, SeekFrom};

use hadris_cd::{CdOptions, CdWriter, Directory, FileEntry, FileTree};
use hadris_iso::sync::read::IsoImage;
use hadris_optical::detect::sync::detect;
use hadris_udf::dir::UdfDirEntry;
use hadris_udf::sync::UdfFs;

const VOLUME_ID: &str = "BRIDGE_TEST";

fn fixture() -> (FileTree, Vec<u8>) {
    let large: Vec<u8> = (0..5000).map(|index| (index % 251) as u8).collect();
    let mut tree = FileTree::new();
    tree.add_file(FileEntry::from_buffer("EMPTY.TXT", Vec::new()));
    let mut docs = Directory::new("DOCS");
    docs.add_file(FileEntry::from_buffer("LARGE.BIN", large.clone()));
    let mut nested = Directory::new("NESTED");
    nested.add_file(FileEntry::from_buffer(
        "NOTE.TXT",
        b"qualified through both namespaces".to_vec(),
    ));
    docs.add_subdir(nested);
    tree.add_dir(docs);
    (tree, large)
}

fn create(options: CdOptions) -> Vec<u8> {
    let (tree, _) = fixture();
    CdWriter::create(Cursor::new(vec![0_u8; 4 * 1024 * 1024]), tree, options)
        .unwrap()
        .into_inner()
}

fn udf_entry<'a>(
    mut entries: impl Iterator<Item = &'a UdfDirEntry>,
    name: &str,
) -> &'a UdfDirEntry {
    entries
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("missing UDF entry {name}"))
}

fn verify_iso(bytes: &[u8], large: &[u8]) {
    let image = IsoImage::open(Cursor::new(bytes)).expect("open ISO namespace");
    let pvd = image.read_pvd().expect("read ISO PVD");
    assert_eq!(pvd.volume_identifier.to_str().trim(), VOLUME_ID);

    let empty = image.find_path("EMPTY.TXT").unwrap().expect("empty file");
    assert_eq!(empty.total_size(), 0);
    assert_eq!(image.read_file(&empty).unwrap(), b"");

    let large_entry = image
        .find_path("DOCS/LARGE.BIN")
        .unwrap()
        .expect("large file");
    assert_eq!(large_entry.total_size(), large.len() as u64);
    assert_eq!(image.read_file(&large_entry).unwrap(), large);

    let note = image
        .find_path("DOCS/NESTED/NOTE.TXT")
        .unwrap()
        .expect("nested file");
    assert_eq!(
        image.read_file(&note).unwrap(),
        b"qualified through both namespaces"
    );
    assert_eq!(image.into_inner().get_ref().len(), bytes.len());
}

fn verify_udf(bytes: &[u8], large: &[u8]) {
    let volume = UdfFs::open(Cursor::new(bytes)).expect("open UDF namespace");
    assert_eq!(volume.info().volume_id.trim_end_matches('\0'), VOLUME_ID);
    let root = volume.root_dir().expect("read UDF root");

    let empty = udf_entry(root.entries(), "EMPTY.TXT");
    assert_eq!(volume.read_file(empty).unwrap(), b"");

    let docs = udf_entry(root.entries(), "DOCS");
    let docs = volume.read_directory(&docs.icb).expect("read DOCS");
    let large_entry = udf_entry(docs.entries(), "LARGE.BIN");
    assert_eq!(volume.read_file(large_entry).unwrap(), large);

    let nested = udf_entry(docs.entries(), "NESTED");
    let nested = volume.read_directory(&nested.icb).expect("read NESTED");
    let note = udf_entry(nested.entries(), "NOTE.TXT");
    assert_eq!(
        volume.read_file(note).unwrap(),
        b"qualified through both namespaces"
    );
    assert_eq!(volume.into_inner().get_ref().len(), bytes.len());
}

#[test]
fn bridge_reopens_through_iso_and_udf_and_recovers_source() {
    let (tree, large) = fixture();
    let cursor = Cursor::new(vec![0_u8; 4 * 1024 * 1024]);
    let output = CdWriter::new(cursor, CdOptions::default().volume_id(VOLUME_ID))
        .finish(tree)
        .expect("create bridge");
    let bytes = output.into_inner();

    verify_iso(&bytes, &large);
    verify_udf(&bytes, &large);

    let mut source = Cursor::new(bytes.as_slice());
    source.seek(SeekFrom::Start(1234)).unwrap();
    let formats = detect(&mut source).unwrap().expect("detect bridge");
    assert!(formats.is_bridge());
    assert_eq!(source.stream_position().unwrap(), 1234);
}

#[test]
fn detects_and_reopens_iso_only_image() {
    let (_, large) = fixture();
    let bytes = create(CdOptions::default().volume_id(VOLUME_ID).iso_only());
    let formats = detect(&mut Cursor::new(bytes.as_slice()))
        .unwrap()
        .expect("detect ISO");
    assert!(formats.has_iso9660());
    assert!(formats.udf().is_none());
    verify_iso(&bytes, &large);
}

#[test]
fn detects_and_reopens_udf_only_image() {
    let (_, large) = fixture();
    let bytes = create(CdOptions::default().volume_id(VOLUME_ID).udf_only());
    let formats = detect(&mut Cursor::new(bytes.as_slice()))
        .unwrap()
        .expect("detect UDF");
    assert!(!formats.has_iso9660());
    assert!(formats.udf().is_some());
    verify_udf(&bytes, &large);
}
