use std::collections::BTreeSet;
use std::io::Cursor;

use hadris_iso::directory::{DirectoryRecord, DirectoryRecordHeader};
use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{InputEntry, InputMetadata, InputTree, IsoImageWriter};

fn options(rrip: RripOptions) -> IsoFormatOptions {
    IsoFormatOptions {
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

fn root_names(image: &IsoImage<Cursor<Vec<u8>>>) -> Vec<String> {
    image
        .root_dir()
        .iter(image)
        .entries()
        .filter_map(Result::ok)
        .filter(|entry| !entry.is_special())
        .map(|entry| entry.display_name().into_owned())
        .collect()
}

fn directory_names(image: &IsoImage<Cursor<Vec<u8>>>, name: &str) -> Vec<String> {
    let entry = image
        .root_dir()
        .iter(image)
        .entries()
        .filter_map(Result::ok)
        .find(|entry| entry.matches_name(name))
        .unwrap();
    let directory = entry.as_dir_ref(image).unwrap();
    image
        .open_dir(directory)
        .entries()
        .filter_map(Result::ok)
        .filter(|entry| !entry.is_special())
        .map(|entry| entry.display_name().into_owned())
        .collect()
}

fn assert_logical_leaf(image: &IsoImage<Cursor<Vec<u8>>>, depth: usize) {
    let mut directory = image.root_dir().dir_ref();
    let mut saw_child_link = false;
    for level in 1..=depth {
        let entry = image
            .open_dir(directory)
            .entries()
            .filter_map(Result::ok)
            .find(|entry| entry.matches_name(&format!("level{level}")))
            .unwrap();
        if entry.rrip.as_ref().unwrap().child_link.is_some() {
            saw_child_link = true;
            assert!(!entry.record.is_directory());
            assert!(entry.is_directory());
        }
        directory = entry.as_dir_ref(image).unwrap();
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

fn record_has_re(record: &DirectoryRecord) -> bool {
    let su = record.system_use();
    let mut offset = 0;
    while offset + 4 <= su.len() {
        let length = su[offset + 2] as usize;
        if length < 4 || offset + length > su.len() {
            break;
        }
        if &su[offset..offset + 2] == b"RE" {
            return true;
        }
        offset += length;
    }
    false
}

fn container_has_relocated_child(image: &IsoImage<Cursor<Vec<u8>>>, name: &str) -> bool {
    let entry = image
        .root_dir()
        .iter(image)
        .entries()
        .filter_map(Result::ok)
        .find(|entry| entry.matches_name(name))
        .unwrap();
    let parent_extent = entry.record.header().extent.read();
    let directory = entry.as_dir_ref(image).unwrap();
    let mut saw_re = false;
    for record in image
        .open_dir(directory)
        .raw_entries()
        .filter_map(Result::ok)
    {
        if record_has_re(&record) {
            assert!(
                record.header().extent.read() > parent_extent,
                "relocated directory extent must follow its container"
            );
            saw_re = true;
        }
    }
    saw_re
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
        if entry.rrip.as_ref().unwrap().child_link.is_some() {
            saw_child_link = true;
            assert!(!entry.record.is_directory());
            assert!(entry.is_directory());
        }
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
fn relocation_reuses_user_rr_moved_directory() {
    let metadata = InputMetadata {
        mode: Some(0o700),
        uid: Some(5),
        gid: Some(6),
        modified: Some(946_684_800),
        ..InputMetadata::default()
    };
    let image = write(
        vec![
            InputEntry::directory(
                "rr_moved",
                vec![InputEntry::file("user.txt", b"user".to_vec())],
            )
            .with_metadata(metadata),
            nested_directory(9),
        ],
        RripOptions::default(),
    );
    let names = root_names(&image);
    assert!(names.contains(&"rr_moved".to_string()));
    assert!(!names.contains(&".rr_moved".to_string()));
    assert_eq!(
        directory_names(&image, "rr_moved"),
        vec!["user.txt".to_string()]
    );
    assert_logical_leaf(&image, 9);
    assert!(container_has_relocated_child(&image, "rr_moved"));

    let rr_moved = image
        .root_dir()
        .iter(&image)
        .entries()
        .filter_map(Result::ok)
        .find(|entry| entry.matches_name("rr_moved"))
        .unwrap();
    let px = rr_moved
        .rrip
        .as_ref()
        .unwrap()
        .posix_attributes
        .as_ref()
        .unwrap();
    assert_eq!(px.file_mode.read(), 0o040700);
    assert_eq!(px.file_uid.read(), 5);
    assert_eq!(px.file_gid.read(), 6);
}

#[test]
fn relocation_uses_dot_name_when_a_file_occupies_rr_moved() {
    let image = write(
        vec![
            InputEntry::file("rr_moved", b"user".to_vec()),
            nested_directory(9),
        ],
        RripOptions::default(),
    );
    let names = root_names(&image);
    assert!(names.contains(&"rr_moved".to_string()));
    assert!(names.contains(&".rr_moved".to_string()));
    assert_logical_leaf(&image, 9);
}

#[test]
fn relocation_reuses_iso_first_directory_when_both_names_exist() {
    let image = write(
        vec![
            InputEntry::directory(
                "rr_moved",
                vec![InputEntry::file("plain.txt", b"plain".to_vec())],
            ),
            InputEntry::directory(
                ".rr_moved",
                vec![InputEntry::file("dot.txt", b"dot".to_vec())],
            ),
            nested_directory(15),
        ],
        RripOptions::default(),
    );
    let names = root_names(&image);
    assert!(names.contains(&"rr_moved".to_string()));
    assert!(names.contains(&".rr_moved".to_string()));
    assert_eq!(
        directory_names(&image, "rr_moved"),
        vec!["plain.txt".to_string()]
    );
    assert_eq!(
        directory_names(&image, ".rr_moved"),
        vec!["dot.txt".to_string()]
    );
    assert_logical_leaf(&image, 15);
    assert!(container_has_relocated_child(&image, "rr_moved"));
    assert!(!container_has_relocated_child(&image, ".rr_moved"));
}

#[test]
fn relocation_avoids_physical_name_collisions_in_user_container() {
    let image = write(
        vec![
            InputEntry::directory(
                "rr_moved",
                vec![
                    InputEntry::file("RRD000001", b"taken".to_vec()),
                    InputEntry::file("user.txt", b"user".to_vec()),
                ],
            ),
            nested_directory(9),
        ],
        RripOptions::default(),
    );
    let names = directory_names(&image, "rr_moved");
    assert!(names.contains(&"RRD000001".to_string()));
    assert!(names.contains(&"user.txt".to_string()));
    assert_logical_leaf(&image, 9);
    assert!(container_has_relocated_child(&image, "rr_moved"));
}

#[test]
fn relocation_preserves_user_tree_inside_reused_container() {
    let mut nested = vec![InputEntry::file("inside.txt", b"inside".to_vec())];
    for level in (1..=8).rev() {
        nested = vec![InputEntry::directory(format!("d{level}"), nested)];
    }
    let image = write(
        vec![
            InputEntry::directory(
                "rr_moved",
                vec![
                    InputEntry::file("user.txt", b"user".to_vec()),
                    nested.pop().unwrap(),
                ],
            ),
            nested_directory(9),
        ],
        RripOptions::default(),
    );
    assert!(directory_names(&image, "rr_moved").contains(&"user.txt".to_string()));
    assert_logical_leaf(&image, 9);

    let rr_moved = image
        .root_dir()
        .iter(&image)
        .entries()
        .filter_map(Result::ok)
        .find(|entry| entry.matches_name("rr_moved"))
        .unwrap()
        .as_dir_ref(&image)
        .unwrap();
    let mut directory = rr_moved;
    for level in 1..=8 {
        let entry = image
            .open_dir(directory)
            .entries()
            .filter_map(Result::ok)
            .find(|entry| entry.matches_name(&format!("d{level}")))
            .unwrap();
        directory = entry.as_dir_ref(&image).unwrap();
    }
    assert!(
        image
            .open_dir(directory)
            .entries()
            .filter_map(Result::ok)
            .any(|entry| entry.matches_name("inside.txt"))
    );
}

#[test]
fn lowercase_creates_dot_container_ahead_of_user_rr_moved() {
    let mut features = CreationFeatures::rock_ridge();
    features.filenames = BaseIsoLevel::Level1 {
        supports_lowercase: true,
        supports_rrip: true,
    };
    let tree = InputTree::new(
        PathSeparator::ForwardSlash,
        vec![
            InputEntry::directory(
                "rr_moved",
                vec![InputEntry::file("user.txt", b"user".to_vec())],
            ),
            nested_directory(9),
        ],
    );
    let data = IsoImageWriter::create(
        Cursor::new(vec![0; 4 * 1024 * 1024]),
        tree,
        IsoFormatOptions {
            features,
            ..options(RripOptions::default())
        },
    )
    .unwrap();
    let image = IsoImage::open(data).unwrap();
    let names = root_names(&image);
    assert!(names.contains(&"rr_moved".to_string()));
    assert!(names.contains(&".rr_moved".to_string()));
    assert_eq!(
        directory_names(&image, "rr_moved"),
        vec!["user.txt".to_string()]
    );
    assert_logical_leaf(&image, 9);
    assert!(container_has_relocated_child(&image, ".rr_moved"));
}

#[test]
fn relocation_path_table_parents_relocated_dirs_under_container() {
    let image = write(
        vec![
            InputEntry::directory(
                "rr_moved",
                vec![InputEntry::file("user.txt", b"user".to_vec())],
            ),
            nested_directory(9),
        ],
        RripOptions::default(),
    );
    let entries: Vec<_> = image
        .path_table()
        .entries(&image)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let names: Vec<_> = entries
        .iter()
        .map(|entry| String::from_utf8_lossy(entry.name.as_bytes()).into_owned())
        .collect();
    let rr_moved = names
        .iter()
        .position(|name| name == "RR_MOVED")
        .expect("path table should include the reused container");
    let relocated = names
        .iter()
        .position(|name| name.starts_with("RRD"))
        .expect("path table should include relocated directories");
    assert_eq!(entries[relocated].parent_index as usize, rr_moved + 1);
    for (index, entry) in entries.iter().enumerate().skip(1) {
        assert!(
            (1..=index).contains(&(entry.parent_index as usize)),
            "invalid parent index {index}: {}",
            entry.parent_index
        );
    }
    let container_number = (rr_moved + 1) as u16;
    let child_ids: Vec<_> = entries
        .iter()
        .filter(|entry| entry.parent_index == container_number)
        .map(|entry| entry.name.as_bytes().to_vec())
        .collect();
    let unique = child_ids.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        child_ids.len(),
        unique.len(),
        "relocated path-table children should have unique ISO identifiers"
    );
}

#[test]
fn relocation_rejects_two_occupied_names() {
    let tree = InputTree::new(
        PathSeparator::ForwardSlash,
        vec![
            InputEntry::file("rr_moved", b"user".to_vec()),
            InputEntry::file(".rr_moved", b"user".to_vec()),
            nested_directory(9),
        ],
    );
    let error = IsoImageWriter::create(
        Cursor::new(Vec::new()),
        tree,
        options(RripOptions::default()),
    )
    .unwrap_err();
    assert!(
        matches!(&error, hadris_iso::write::IsoCreationError::Io(inner) if inner.kind() == hadris_io::ErrorKind::InvalidInput)
    );
    assert!(
        error
            .to_string()
            .contains("available rr_moved or .rr_moved directory")
    );
}

#[test]
fn shallow_trees_allow_both_relocation_names() {
    write(
        vec![
            InputEntry::directory("rr_moved", Vec::new()),
            InputEntry::directory(".rr_moved", Vec::new()),
        ],
        RripOptions::default(),
    );
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
        if entry.rrip.as_ref().unwrap().child_link.is_some() {
            saw_child_link = true;
            assert!(!entry.record.is_directory());
            assert!(entry.is_directory());
        }
        directory = entry.as_dir_ref(&image).unwrap();
    }
    assert!(saw_child_link);
}

#[test]
fn patches_dot_entry_for_every_directory_when_the_parent_listing_spans_multiple_sectors() {
    let children: Vec<_> = (0..50)
        .map(|i| {
            InputEntry::directory(
                format!("dir{i:02}"),
                vec![InputEntry::file("f.txt", b"x".to_vec())],
            )
        })
        .collect();
    let tree = InputTree::new(PathSeparator::ForwardSlash, children);
    let cursor = IsoImageWriter::create(
        Cursor::new(vec![0u8; 4 * 1024 * 1024]),
        tree,
        options(RripOptions::default()),
    )
    .unwrap();
    let raw = cursor.get_ref().clone();
    let image = IsoImage::open(cursor).unwrap();

    let mut checked = 0;
    for entry in image
        .root_dir()
        .iter(&image)
        .entries()
        .filter_map(Result::ok)
        .filter(|entry| !entry.is_special())
    {
        let dir_ref = entry.as_dir_ref(&image).unwrap();
        let sector = dir_ref.extent.0 * 2048;
        let dot = DirectoryRecordHeader::from_bytes(
            &raw[sector..sector + size_of::<DirectoryRecordHeader>()],
        );
        assert_eq!(
            dot.extent.read() as usize,
            dir_ref.extent.0,
            "{:?}'s '.' entry should self-reference its own extent",
            entry.display_name()
        );
        assert_eq!(
            dot.data_len.read() as usize,
            dir_ref.size,
            "{:?}'s '.' entry should self-reference its own size",
            entry.display_name()
        );
        checked += 1;
    }
    assert_eq!(checked, 50);
}
