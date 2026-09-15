use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;

use hadris_iso::read::PathSeparator;
use hadris_iso::write::options::{CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{InputEntry, InputTree, IsoImageWriter};
use hadris_tests::harness::command::{require_or_skip, run_command};
use hadris_tests::harness::tree::{EntryData, snapshot_host};

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
    for names in paths {
        for collision in [None, Some(("rr_moved", false)), Some((".rr_moved", true))] {
            let mut expected = BTreeMap::new();
            let mut path = String::new();
            for name in &names {
                path.push('/');
                path.push_str(name);
                expected.insert(path.clone(), EntryData::Directory);
            }
            expected.insert(
                format!("{path}/leaf.txt"),
                EntryData::File(b"deep".to_vec()),
            );
            let mut entries = vec![InputEntry::file("leaf.txt", b"deep".to_vec())];
            for name in names.iter().rev() {
                entries = vec![InputEntry::directory(name.clone(), entries)];
            }
            if let Some((name, directory)) = collision {
                if directory {
                    entries.push(InputEntry::directory(
                        name,
                        vec![InputEntry::file("user.txt", b"user".to_vec())],
                    ));
                    expected.insert(format!("/{name}"), EntryData::Directory);
                    expected.insert(
                        format!("/{name}/user.txt"),
                        EntryData::File(b"user".to_vec()),
                    );
                } else {
                    entries.push(InputEntry::file(name, b"user".to_vec()));
                    expected.insert(format!("/{name}"), EntryData::File(b"user".to_vec()));
                }
            }
            let options = IsoFormatOptions {
                features: CreationFeatures::rock_ridge(),
                volume_name: "RELOCATION".to_string(),
                system_id: None,
                volume_set_id: None,
                publisher_id: None,
                preparer_id: None,
                application_id: None,
                sector_size: 2048,
                path_separator: PathSeparator::ForwardSlash,
                strict_charset: false,
            };
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
            assert_eq!(
                snapshot_host(&extracted).unwrap(),
                expected,
                "path components: {names:?}, collision: {collision:?}"
            );
        }
    }
}
