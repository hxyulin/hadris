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
fn writes_and_reads_back_creation_timestamp() {
    // 2001-09-09 01:46:40 UTC — distinct from modify/access so we can tell them
    // apart on read-back.
    let metadata = InputMetadata {
        created: Some(1_000_000_000),
        modified: Some(946_684_800),
        accessed: Some(946_684_801),
        ..InputMetadata::default()
    };
    let image = write(
        vec![InputEntry::file("data.txt", b"data".to_vec()).with_metadata(metadata)],
        RripOptions::default(),
    );
    let entry = image
        .root_dir()
        .iter(&image)
        .entries()
        .filter_map(Result::ok)
        .find(|entry| entry.matches_name("data.txt"))
        .unwrap();
    let ts = entry
        .rrip
        .as_ref()
        .unwrap()
        .timestamps
        .as_ref()
        .expect("TF timestamps present");
    let creation = ts.creation.as_ref().expect("creation timestamp emitted");
    assert_eq!(creation.year, 2001, "creation year round-trips");
    // Modify and access must still be present and distinct from creation.
    assert!(ts.modify.is_some());
    assert!(ts.access.is_some());
    assert_ne!(creation.year, ts.modify.as_ref().unwrap().year);
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

fn nested_directory(depth: usize) -> InputEntry {
    let mut children = vec![InputEntry::file("leaf.txt", b"deep".to_vec())];
    for level in (1..=depth).rev() {
        children = vec![InputEntry::directory(format!("level{level}"), children)];
    }
    children.pop().unwrap()
}

#[test]
fn relocates_a_ninth_level_directory_and_preserves_the_rrip_view() {
    let image = write(vec![nested_directory(9)], RripOptions::default());
    let mut directory = image.root_dir().dir_ref();
    let mut saw_child_link = false;

    for level in 1..=9 {
        let entries: Vec<_> = image
            .open_dir(directory)
            .entries()
            .filter_map(Result::ok)
            .collect();
        let names: Vec<_> = entries
            .iter()
            .map(|entry| entry.display_name().into_owned())
            .collect();
        let entry = entries
            .into_iter()
            .find(|entry| entry.matches_name(&format!("level{level}")))
            .unwrap_or_else(|| panic!("missing logical level {level}; found {names:?}"));
        saw_child_link |= entry.rrip.as_ref().unwrap().child_link.is_some();
        directory = entry.as_dir_ref(&image).unwrap();
    }
    assert!(saw_child_link);

    assert!(
        image
            .open_dir(directory)
            .entries()
            .filter_map(Result::ok)
            .any(|entry| entry.matches_name("leaf.txt"))
    );
}

#[test]
fn relocation_directory_name_does_not_collide_with_user_input() {
    let image = write(
        vec![
            InputEntry::directory("RR_MOVED", Vec::new()),
            nested_directory(9),
        ],
        RripOptions::default(),
    );
    let names: Vec<_> = image
        .root_dir()
        .iter(&image)
        .entries()
        .filter_map(Result::ok)
        .filter(|entry| !entry.is_special())
        .map(|entry| entry.display_name().into_owned())
        .collect();
    assert!(names.contains(&"RR_MOVED".to_string()));
    assert!(names.contains(&"RR_MOVED_1".to_string()));
}

#[test]
fn rejects_deep_directories_when_relocation_is_disabled() {
    let rrip = RripOptions {
        relocate_deep_dirs: false,
        ..RripOptions::default()
    };
    let tree = InputTree::new(PathSeparator::ForwardSlash, vec![nested_directory(9)]);
    let error = IsoImageWriter::create(Cursor::new(Vec::new()), tree, options(rrip)).unwrap_err();
    assert!(error.to_string().contains("relocation is disabled"));
}

#[test]
fn relocates_paths_that_exceed_the_iso_path_length_limit() {
    let names: Vec<_> = (0..5)
        .map(|index| format!("{index}_{}", "x".repeat(58)))
        .collect();
    let mut children = vec![InputEntry::file("leaf.txt", Vec::new())];
    for name in names.iter().rev() {
        children = vec![InputEntry::directory(name.clone(), children)];
    }
    let image = write(children, RripOptions::default());
    let mut directory = image.root_dir().dir_ref();
    let mut saw_child_link = false;
    for name in names {
        let entry = image
            .open_dir(directory)
            .entries()
            .filter_map(Result::ok)
            .find(|entry| entry.matches_name(&name))
            .unwrap();
        saw_child_link |= entry.rrip.as_ref().unwrap().child_link.is_some();
        directory = entry.as_dir_ref(&image).unwrap();
    }
    assert!(saw_child_link);
}
