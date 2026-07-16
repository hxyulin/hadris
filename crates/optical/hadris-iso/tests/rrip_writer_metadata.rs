use std::io::Cursor;

use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::{CreationFeatures, FormatOptions};
use hadris_iso::write::{InputEntry, InputMetadata, InputTree, IsoImageWriter};

fn options(rrip: RripOptions) -> FormatOptions {
    FormatOptions {
        volume_name: "RRIP_TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        strict_charset: false,
        features: CreationFeatures {
            rock_ridge: Some(rrip),
            ..CreationFeatures::rock_ridge()
        },
    }
}

fn write(entries: Vec<InputEntry>, rrip: RripOptions) -> IsoImage<Cursor<Vec<u8>>> {
    let tree = InputTree::new(PathSeparator::ForwardSlash, entries);
    let data =
        IsoImageWriter::create(Cursor::new(vec![0; 4 * 1024 * 1024]), tree, options(rrip)).unwrap();
    IsoImage::open(data).unwrap()
}

#[test]
fn writes_posix_metadata_and_special_entries() {
    let metadata = InputMetadata {
        mode: Some(0o640),
        uid: Some(1000),
        gid: Some(1001),
        modified: Some(946_684_800),
        accessed: Some(946_684_801),
        ..InputMetadata::default()
    };
    let image = write(
        vec![
            InputEntry::file("data.txt", b"data".to_vec()).with_metadata(metadata),
            InputEntry::symlink("latest", "data.txt"),
            InputEntry::character_device("ttyS0", 4, 64),
        ],
        RripOptions::default(),
    );
    let entries: Vec<_> = image
        .root_dir()
        .iter(&image)
        .entries()
        .filter_map(Result::ok)
        .filter(|entry| !entry.is_special())
        .collect();

    let file = entries
        .iter()
        .find(|entry| entry.matches_name("data.txt"))
        .unwrap();
    let rrip = file.rrip.as_ref().unwrap();
    let px = rrip.posix_attributes.as_ref().unwrap();
    assert_eq!(px.file_mode.read(), 0o100640);
    assert_eq!(px.file_uid.read(), 1000);
    assert_eq!(px.file_gid.read(), 1001);
    assert!(rrip.timestamps.is_some());

    let symlink = entries
        .iter()
        .find(|entry| entry.matches_name("latest"))
        .unwrap();
    assert_eq!(
        symlink.rrip.as_ref().unwrap().symlink_target.as_deref(),
        Some("data.txt")
    );
    assert_eq!(
        symlink
            .rrip
            .as_ref()
            .unwrap()
            .posix_attributes
            .as_ref()
            .unwrap()
            .file_mode
            .read()
            & 0o170000,
        0o120000
    );

    let device = entries
        .iter()
        .find(|entry| entry.matches_name("ttyS0"))
        .unwrap();
    let rrip = device.rrip.as_ref().unwrap();
    let pn = rrip.device_number.as_ref().unwrap();
    assert_eq!(pn.dev_high.read(), 4);
    assert_eq!(pn.dev_low.read(), 64);
}

#[test]
fn disabled_preservation_uses_stable_defaults() {
    let metadata = InputMetadata {
        mode: Some(0o600),
        uid: Some(42),
        gid: Some(43),
        modified: Some(946_684_800),
        accessed: Some(946_684_800),
        ..InputMetadata::default()
    };
    let rrip = RripOptions {
        preserve_permissions: false,
        preserve_ownership: false,
        preserve_timestamps: false,
        ..RripOptions::default()
    };
    let image = write(
        vec![InputEntry::file("data.txt", Vec::new()).with_metadata(metadata)],
        rrip,
    );
    let entry = image
        .root_dir()
        .iter(&image)
        .entries()
        .filter_map(Result::ok)
        .find(|entry| entry.matches_name("data.txt"))
        .unwrap();
    let rrip = entry.rrip.as_ref().unwrap();
    let px = rrip.posix_attributes.as_ref().unwrap();
    assert_eq!(px.file_mode.read(), 0o100644);
    assert_eq!(px.file_uid.read(), 0);
    assert_eq!(px.file_gid.read(), 0);
    assert!(rrip.timestamps.is_none());
}

#[test]
fn rejects_special_entries_when_the_matching_option_is_disabled() {
    let rrip = RripOptions {
        preserve_symlinks: false,
        ..RripOptions::default()
    };
    let tree = InputTree::new(
        PathSeparator::ForwardSlash,
        vec![InputEntry::symlink("link", "target")],
    );
    let error = IsoImageWriter::create(Cursor::new(Vec::new()), tree, options(rrip)).unwrap_err();
    assert!(error.to_string().contains("preserve_symlinks"));
}
