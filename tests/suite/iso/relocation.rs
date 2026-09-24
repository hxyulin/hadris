use std::collections::BTreeMap;
use std::fs;

use hadris_fs::tree::{Content, Tree};
use hadris_iso::{IsoOptions, Relocation, RockRidge, VolumeIdentifiers};
use hadris_tests::harness::command::{require_or_skip, run_command};
use hadris_tests::harness::tree::{EntryData, snapshot_host};
use hadris_tests::iso::hadris::write_tree;

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
        for (collision, relocation) in [
            (None, "rr_moved"),
            (Some(("rr_moved", false)), ".rr_moved"),
            (Some((".rr_moved", true)), "rr_moved"),
            (Some(("rr_moved", true)), "rr_moved"),
        ] {
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
            let mut tree = Tree::new();
            tree.add_file(&format!("{path}/leaf.txt"), Content::bytes("deep"))
                .unwrap();
            if let Some((name, directory)) = collision {
                if directory {
                    tree.add_file(&format!("{name}/user.txt"), Content::bytes("user"))
                        .unwrap();
                    tree.add_file(&format!("{name}/RRD000001/own.txt"), Content::bytes("own"))
                        .unwrap();
                    expected.insert(format!("/{name}"), EntryData::Directory);
                    expected.insert(
                        format!("/{name}/user.txt"),
                        EntryData::File(b"user".to_vec()),
                    );
                    expected.insert(format!("/{name}/RRD000001"), EntryData::Directory);
                    expected.insert(
                        format!("/{name}/RRD000001/own.txt"),
                        EntryData::File(b"own".to_vec()),
                    );
                } else {
                    tree.add_file(name, Content::bytes("user")).unwrap();
                    expected.insert(format!("/{name}"), EntryData::File(b"user".to_vec()));
                }
            }
            let options = IsoOptions::default()
                .with_volume(VolumeIdentifiers::new("RELOCATION"))
                .with_rock_ridge(
                    RockRidge::default().with_relocation(Relocation::Directory(relocation.into())),
                );
            let image = write_tree(&tree, &options).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let iso = temp.path().join("image.iso");
            let extracted = temp.path().join("extracted");
            fs::write(&iso, image).unwrap();
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
