#![cfg(all(feature = "std", feature = "sync", feature = "write"))]

use std::io::Cursor;

use hadris_iso::read::PathSeparator;
use hadris_iso::sync::read::IsoImage;
use hadris_iso::sync::write::options::{CreationFeatures, FormatOptions};
use hadris_iso::sync::write::{InputEntry, InputTree, IsoImageWriter};

fn options() -> FormatOptions {
    FormatOptions {
        volume_name: "FLOOR_TEST".to_owned(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        features: CreationFeatures::default(),
        path_separator: PathSeparator::ForwardSlash,
        strict_charset: false,
    }
}

fn input() -> InputTree {
    InputTree::new(
        PathSeparator::ForwardSlash,
        vec![InputEntry::file("PAYLOAD.BIN", vec![0x5a; 4096])],
    )
}

#[test]
fn default_create_retains_compact_allocation() {
    let output =
        IsoImageWriter::create(Cursor::new(vec![0_u8; 2 * 1024 * 1024]), input(), options())
            .unwrap();
    let image = IsoImage::open(Cursor::new(output.into_inner())).unwrap();
    let entry = image.find_path("PAYLOAD.BIN").unwrap().unwrap();
    assert!(entry.header().extent.read() < 400);
    assert_eq!(image.read_file(&entry).unwrap(), vec![0x5a; 4096]);
}

#[test]
fn allocation_floor_is_honored() {
    let output = IsoImageWriter::create_with_allocation_floor(
        Cursor::new(vec![0_u8; 2 * 1024 * 1024]),
        input(),
        options(),
        Some(400),
    )
    .unwrap();
    let image = IsoImage::open(Cursor::new(output.into_inner())).unwrap();
    let entry = image.find_path("PAYLOAD.BIN").unwrap().unwrap();
    assert_eq!(entry.header().extent.read(), 400);
    assert_eq!(image.read_file(&entry).unwrap(), vec![0x5a; 4096]);
}
