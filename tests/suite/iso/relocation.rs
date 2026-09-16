use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;

use hadris_iso::read::PathSeparator;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{InputEntry, InputTree, IsoImageWriter};
use hadris_tests::harness::command::{require_or_skip, run_command};
use hadris_tests::harness::tree::{EntryData, snapshot_host};

#[derive(Clone, Debug)]
enum Collision {
    None,
    File(&'static str),
    Directory(&'static str),
    Directories(&'static [&'static str]),
    DirectoryWithNames(&'static str, &'static [&'static str]),
}

fn rock_ridge_options(lowercase: bool) -> IsoFormatOptions {
    let mut features = CreationFeatures::rock_ridge();
    if lowercase {
        features.filenames = BaseIsoLevel::Level1 {
            supports_lowercase: true,
            supports_rrip: true,
        };
    }
    IsoFormatOptions {
        features,
        volume_name: "RELOCATION".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        strict_charset: false,
    }
}

fn nested_entries(names: &[String], leaf: &[u8]) -> Vec<InputEntry> {
    let mut entries = vec![InputEntry::file("leaf.txt", leaf.to_vec())];
    for name in names.iter().rev() {
        entries = vec![InputEntry::directory(name.clone(), entries)];
    }
    entries
}

fn expected_nested(names: &[String], leaf: &[u8]) -> BTreeMap<String, EntryData> {
    let mut expected = BTreeMap::new();
    let mut path = String::new();
    for name in names {
        path.push('/');
        path.push_str(name);
        expected.insert(path.clone(), EntryData::Directory);
    }
    expected.insert(format!("{path}/leaf.txt"), EntryData::File(leaf.to_vec()));
    expected
}

fn apply_collision(
    collision: &Collision,
    entries: &mut Vec<InputEntry>,
    expected: &mut BTreeMap<String, EntryData>,
) {
    match collision {
        Collision::None => {}
        Collision::File(name) => {
            entries.push(InputEntry::file(*name, b"user".to_vec()));
            expected.insert(format!("/{name}"), EntryData::File(b"user".to_vec()));
        }
        Collision::Directory(name) => {
            entries.push(InputEntry::directory(
                *name,
                vec![InputEntry::file("user.txt", b"user".to_vec())],
            ));
            expected.insert(format!("/{name}"), EntryData::Directory);
            expected.insert(
                format!("/{name}/user.txt"),
                EntryData::File(b"user".to_vec()),
            );
        }
        Collision::Directories(names) => {
            for name in *names {
                entries.push(InputEntry::directory(
                    *name,
                    vec![InputEntry::file(format!("{name}.txt"), b"user".to_vec())],
                ));
                expected.insert(format!("/{name}"), EntryData::Directory);
                expected.insert(
                    format!("/{name}/{name}.txt"),
                    EntryData::File(b"user".to_vec()),
                );
            }
        }
        Collision::DirectoryWithNames(name, extra) => {
            let mut children = vec![InputEntry::file("user.txt", b"user".to_vec())];
            expected.insert(format!("/{name}"), EntryData::Directory);
            expected.insert(
                format!("/{name}/user.txt"),
                EntryData::File(b"user".to_vec()),
            );
            for extra_name in *extra {
                children.push(InputEntry::file(*extra_name, b"taken".to_vec()));
                expected.insert(
                    format!("/{name}/{extra_name}"),
                    EntryData::File(b"taken".to_vec()),
                );
            }
            entries.push(InputEntry::directory(*name, children));
        }
    }
}

fn extract_with_bsdtar(
    entries: Vec<InputEntry>,
    options: IsoFormatOptions,
) -> BTreeMap<String, EntryData> {
    let image = IsoImageWriter::create(
        Cursor::new(Vec::new()),
        InputTree::new(PathSeparator::ForwardSlash, entries),
        options,
    )
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let iso = temp.path().join("image.iso");
    let extracted = temp.path().join("extracted");
    fs::write(&iso, image.into_inner()).unwrap();
    fs::create_dir(&extracted).unwrap();
    run_command(
        "bsdtar",
        vec![
            "-xf".into(),
            iso.into_os_string(),
            "-C".into(),
            extracted.clone().into_os_string(),
        ],
    )
    .unwrap();
    snapshot_host(&extracted).unwrap()
}

#[test]
fn bsdtar_extracts_relocated_trees() {
    if !require_or_skip("bsdtar", "--version") {
        return;
    }
    let paths = [
        (1..=9).map(|i| format!("level{i}")).collect::<Vec<_>>(),
        (1..=15).map(|i| format!("level{i}")).collect(),
        (0..5).map(|i| format!("{i}_{}", "x".repeat(58))).collect(),
    ];
    let collisions = [
        Collision::None,
        Collision::File("rr_moved"),
        Collision::File(".rr_moved"),
        Collision::Directory("rr_moved"),
        Collision::Directory(".rr_moved"),
        Collision::Directories(&["rr_moved", ".rr_moved"]),
        Collision::DirectoryWithNames("rr_moved", &["RRD000001"]),
    ];
    for names in &paths {
        for collision in &collisions {
            let mut expected = expected_nested(names, b"deep");
            let mut entries = nested_entries(names, b"deep");
            apply_collision(collision, &mut entries, &mut expected);
            assert_eq!(
                extract_with_bsdtar(entries, rock_ridge_options(false)),
                expected,
                "path components: {names:?}, collision: {collision:?}"
            );
        }
    }
}

#[test]
fn bsdtar_extracts_user_rr_moved_with_lowercase_iso_names() {
    if !require_or_skip("bsdtar", "--version") {
        return;
    }
    let names: Vec<_> = (1..=9).map(|i| format!("level{i}")).collect();
    let mut expected = expected_nested(&names, b"deep");
    let mut entries = nested_entries(&names, b"deep");
    apply_collision(
        &Collision::Directory("rr_moved"),
        &mut entries,
        &mut expected,
    );
    assert_eq!(
        extract_with_bsdtar(entries, rock_ridge_options(true)),
        expected
    );
}

#[test]
fn bsdtar_extracts_deep_user_tree_inside_rr_moved() {
    if !require_or_skip("bsdtar", "--version") {
        return;
    }
    let names: Vec<_> = (1..=9).map(|i| format!("level{i}")).collect();
    let mut expected = expected_nested(&names, b"deep");
    let mut entries = nested_entries(&names, b"deep");
    let mut nested = vec![InputEntry::file("inside.txt", b"inside".to_vec())];
    let mut nested_path = String::from("/rr_moved");
    expected.insert(nested_path.clone(), EntryData::Directory);
    for level in 1..=8 {
        nested_path.push_str(&format!("/d{level}"));
        expected.insert(nested_path.clone(), EntryData::Directory);
        nested = vec![InputEntry::directory(format!("d{level}"), nested)];
    }
    expected.insert(
        format!("{nested_path}/inside.txt"),
        EntryData::File(b"inside".to_vec()),
    );
    expected.insert(
        "/rr_moved/user.txt".to_string(),
        EntryData::File(b"user".to_vec()),
    );
    entries.push(InputEntry::directory(
        "rr_moved",
        vec![
            InputEntry::file("user.txt", b"user".to_vec()),
            nested.pop().unwrap(),
        ],
    ));
    assert_eq!(
        extract_with_bsdtar(entries, rock_ridge_options(false)),
        expected
    );
}
