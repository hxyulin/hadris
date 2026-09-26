use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hadris_fs::{Content, Node, Tree};
use hadris_iso::{IsoId, IsoOptions, NameCase, Relocation};
use hadris_tests::harness::command::{require_or_skip, run_command};
use hadris_tests::harness::tree::{EntryData, snapshot_host};
use hadris_tests::iso::hadris::write_tree;
use hadris_tests::iso::xorriso;

type Snapshot = BTreeMap<String, EntryData>;

#[derive(Clone, Copy, Debug)]
enum Collision {
    None,
    File(&'static str),
    Directory(&'static str),
    Directories(&'static [&'static str]),
}

#[derive(Clone, Copy)]
enum TreeMatch {
    Exact,
    /// xorriso can leave the emptied relocation directory behind.
    AllowEmptyRelocationDirectory,
}

fn relocation_paths() -> [Vec<String>; 3] {
    [
        (1..=9).map(|i| format!("level{i}")).collect(),
        (1..=15).map(|i| format!("level{i}")).collect(),
        (0..5).map(|i| format!("{i}_{}", "x".repeat(58))).collect(),
    ]
}

fn relocation_cases() -> [(Collision, Relocation); 7] {
    [
        (Collision::None, Relocation::RrMoved),
        (Collision::None, Relocation::DotRrMoved),
        (Collision::File("rr_moved"), Relocation::DotRrMoved),
        (Collision::Directory(".rr_moved"), Relocation::RrMoved),
        (Collision::Directory("rr_moved"), Relocation::RrMoved),
        (Collision::Directory(".rr_moved"), Relocation::DotRrMoved),
        (
            Collision::Directories(&["rr_moved", ".rr_moved"]),
            Relocation::RrMoved,
        ),
    ]
}

fn insert_file(tree: &mut Tree, expected: &mut Snapshot, path: &str, data: &'static str) {
    tree.insert(path, Node::file(Content::bytes(data))).unwrap();
    let mut prefix = String::new();
    let (parents, _) = path.rsplit_once('/').unwrap_or(("", path));
    for part in parents.split('/').filter(|part| !part.is_empty()) {
        prefix.push('/');
        prefix.push_str(part);
        expected.insert(prefix.clone(), EntryData::Directory);
    }
    expected.insert(
        format!("/{path}"),
        EntryData::File(data.as_bytes().to_vec()),
    );
}

fn nested_tree(names: &[String]) -> (Tree, Snapshot) {
    let mut tree = Tree::new();
    let mut expected = Snapshot::new();
    insert_file(
        &mut tree,
        &mut expected,
        &format!("{}/leaf.txt", names.join("/")),
        "deep",
    );
    (tree, expected)
}

fn apply_collision(collision: Collision, tree: &mut Tree, expected: &mut Snapshot) {
    let user_directory = |tree: &mut Tree, expected: &mut Snapshot, name: &str| {
        insert_file(tree, expected, &format!("{name}/user.txt"), "user");
        insert_file(tree, expected, &format!("{name}/RRD000001/own.txt"), "own");
    };
    match collision {
        Collision::None => {}
        Collision::File(name) => insert_file(tree, expected, name, "user"),
        Collision::Directory(name) => user_directory(tree, expected, name),
        Collision::Directories(names) => {
            for name in names {
                user_directory(tree, expected, name);
            }
        }
    }
}

fn options(relocation: Relocation) -> IsoOptions {
    IsoOptions::default()
        .with_id(IsoId::Volume, "RELOCATION")
        .with_rock_ridge()
        .with_relocation(relocation)
}

fn extract_with_bsdtar(iso: &Path, extracted: &Path) {
    run_command(
        "bsdtar",
        vec![
            "-xf".into(),
            iso.as_os_str().to_owned(),
            "-C".into(),
            extracted.as_os_str().to_owned(),
        ],
    )
    .unwrap();
}

fn extract_with_xorriso(iso: &Path, extracted: &Path) {
    xorriso::extract(iso, extracted).unwrap();
}

fn extract(tree: &Tree, options: &IsoOptions, extractor: fn(&Path, &Path)) -> Snapshot {
    let image = write_tree(tree, options).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let iso = temp.path().join("image.iso");
    let extracted = temp.path().join("extracted");
    fs::write(&iso, image).unwrap();
    fs::create_dir_all(&extracted).unwrap();
    extractor(&iso, &extracted);
    snapshot_host(&extracted).unwrap()
}

fn is_empty_relocation_directory(path: &str, data: &EntryData, tree: &Snapshot) -> bool {
    let prefix = format!("{path}/");
    matches!(path, "/rr_moved" | "/.rr_moved")
        && data == &EntryData::Directory
        && !tree.keys().any(|child| child.starts_with(&prefix))
}

fn assert_tree(actual: &Snapshot, expected: &Snapshot, mode: TreeMatch, context: &str) {
    match mode {
        TreeMatch::Exact => assert_eq!(actual, expected, "{context}"),
        TreeMatch::AllowEmptyRelocationDirectory => {
            for (path, data) in expected {
                assert_eq!(actual.get(path), Some(data), "{context}: {path}");
            }
            for (path, data) in actual {
                assert!(
                    expected.contains_key(path)
                        || is_empty_relocation_directory(path, data, actual),
                    "{context}: unexpected {path}"
                );
            }
        }
    }
}

fn check_relocated_trees(extractor: fn(&Path, &Path), mode: TreeMatch) {
    for names in relocation_paths() {
        for (collision, relocation) in relocation_cases() {
            let (mut tree, mut expected) = nested_tree(&names);
            apply_collision(collision, &mut tree, &mut expected);
            let actual = extract(&tree, &options(relocation), extractor);
            let context =
                format!("path components: {names:?}, collision: {collision:?}, {relocation:?}");
            assert_tree(&actual, &expected, mode, &context);
        }
    }
}

fn check_user_rr_moved_with_preserved_case(extractor: fn(&Path, &Path), mode: TreeMatch) {
    let names: Vec<_> = (1..=9).map(|i| format!("level{i}")).collect();
    let (mut tree, mut expected) = nested_tree(&names);
    apply_collision(Collision::Directory("rr_moved"), &mut tree, &mut expected);
    let options = options(Relocation::RrMoved).with_name_case(NameCase::Preserve);
    let actual = extract(&tree, &options, extractor);
    assert_tree(&actual, &expected, mode, "user rr_moved, preserved case");
}

fn check_deep_user_tree_inside_rr_moved(extractor: fn(&Path, &Path), mode: TreeMatch) {
    let names: Vec<_> = (1..=9).map(|i| format!("level{i}")).collect();
    let (mut tree, mut expected) = nested_tree(&names);
    insert_file(&mut tree, &mut expected, "rr_moved/user.txt", "user");
    let deep: Vec<_> = (1..=8).map(|i| format!("d{i}")).collect();
    insert_file(
        &mut tree,
        &mut expected,
        &format!("rr_moved/{}/inside.txt", deep.join("/")),
        "inside",
    );
    let actual = extract(&tree, &options(Relocation::RrMoved), extractor);
    assert_tree(&actual, &expected, mode, "deep user tree inside rr_moved");
}

#[test]
fn bsdtar_extracts_relocated_trees() {
    if !require_or_skip("bsdtar", "--version") {
        return;
    }
    check_relocated_trees(extract_with_bsdtar, TreeMatch::Exact);
}

#[test]
fn bsdtar_extracts_user_rr_moved_with_preserved_case() {
    if !require_or_skip("bsdtar", "--version") {
        return;
    }
    check_user_rr_moved_with_preserved_case(extract_with_bsdtar, TreeMatch::Exact);
}

#[test]
fn bsdtar_extracts_deep_user_tree_inside_rr_moved() {
    if !require_or_skip("bsdtar", "--version") {
        return;
    }
    check_deep_user_tree_inside_rr_moved(extract_with_bsdtar, TreeMatch::Exact);
}

#[test]
fn xorriso_extracts_relocated_trees() {
    if !xorriso::require() {
        return;
    }
    check_relocated_trees(
        extract_with_xorriso,
        TreeMatch::AllowEmptyRelocationDirectory,
    );
}

#[test]
fn xorriso_extracts_user_rr_moved_with_preserved_case() {
    if !xorriso::require() {
        return;
    }
    check_user_rr_moved_with_preserved_case(
        extract_with_xorriso,
        TreeMatch::AllowEmptyRelocationDirectory,
    );
}

#[test]
fn xorriso_extracts_deep_user_tree_inside_rr_moved() {
    if !xorriso::require() {
        return;
    }
    check_deep_user_tree_inside_rr_moved(
        extract_with_xorriso,
        TreeMatch::AllowEmptyRelocationDirectory,
    );
}
