//! `copy_tree` into a mounted filesystem, and the host module's trees,
//! which never write outside their target directory.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use std::path::PathBuf;

use common::MemError;
use common::sync::{MemFs, fixture};
use hadris_fs::host::{self, TreeOptions};
#[cfg(unix)]
use hadris_fs::host::{OnError, Symlinks};
use hadris_fs::sync::{FileSystem, Volume, copy_tree, read_tree};
use hadris_fs::{
    Content, DateTime, DeviceNumber, ErrorKind, Field, FileType, Node, Owner, PathError,
    Permissions, SetAttr, Tree, WarningKind,
};

fn file(data: &[u8]) -> Node {
    Node::file(Content::bytes(data))
}

fn sample() -> Tree {
    let time = DateTime::from_unix_seconds(1_700_000_000).unwrap();
    let mut tree = Tree::new();
    tree.insert(
        "etc/conf",
        file(b"key=value").with_attrs(SetAttr::new().with_modified(time)),
    )
    .unwrap();
    tree.insert("a.txt", file(b"root a")).unwrap();
    tree.insert(
        "big",
        Node::file(Content::bytes(
            (0..100_000u32).map(|i| i as u8).collect::<Vec<_>>(),
        )),
    )
    .unwrap();
    tree.insert(
        "empty",
        Node::dir().with_attrs(SetAttr::new().with_owner(Owner::new(1, 2))),
    )
    .unwrap();
    tree
}

#[test]
fn copies_files_and_directories() {
    let mut dst = MemFs::new();
    let root = dst.root();
    let report = copy_tree(&sample(), &mut dst, root).unwrap();
    assert_eq!(dst.contents("/etc/conf").unwrap(), b"key=value");
    assert_eq!(dst.contents("/a.txt").unwrap(), b"root a");
    assert_eq!(dst.contents("/big").unwrap().len(), 100_000);
    assert!(dst.contents("/empty").is_none());
    assert_eq!(report.size(), 0);
    let dropped: Vec<_> = report
        .warnings()
        .iter()
        .map(|w| (w.kind(), w.count()))
        .collect();
    assert_eq!(
        dropped,
        [
            (WarningKind::Dropped(Field::Modified), 1),
            (WarningKind::Dropped(Field::Owner), 1)
        ]
    );
    assert_eq!((dst.open_nodes(), dst.open_files()), (1, 0));
}

#[test]
fn nodes_the_trait_cannot_create_are_skipped() {
    let mut tree = sample();
    tree.insert("link", Node::symlink("etc")).unwrap();
    tree.insert(
        "dev/null",
        Node::special(FileType::CharDevice, Some(DeviceNumber::new(1, 3))),
    )
    .unwrap();
    tree.link("a.txt", "etc/again").unwrap();
    let mut dst = MemFs::new();
    let root = dst.root();
    let report = copy_tree(&tree, &mut dst, root).unwrap();
    let skipped: Vec<_> = report
        .warnings()
        .iter()
        .filter(|w| w.kind() == WarningKind::Skipped)
        .map(|w| w.path().unwrap().to_vec())
        .collect();
    assert_eq!(
        skipped,
        [
            b"/dev/null".to_vec(),
            b"/etc/again".to_vec(),
            b"/link".to_vec()
        ]
    );
    assert!(dst.contents("/etc/again").is_none());
    assert_eq!(dst.open_nodes(), 1);
}

#[test]
fn conflicts_and_failures_name_the_path_and_release_pins() {
    let mut dst = MemFs::new();
    dst.add("/", "etc", FileType::Dir, b"");
    let root = dst.root();
    let err = copy_tree(&sample(), &mut dst, root).unwrap_err();
    assert_eq!(
        (err.kind(), err.path()),
        (ErrorKind::AlreadyExists, Some(&b"/etc"[..]))
    );
    assert_eq!((dst.open_nodes(), dst.open_files()), (1, 0));

    let mut read_only = MemFs::new().read_only();
    let root = read_only.root();
    assert_eq!(
        copy_tree(&sample(), &mut read_only, root)
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );

    let mut dst = MemFs::new();
    dst.fail_next(MemError::Timeout { lba: 9 });
    let root = dst.root();
    let err: PathError = copy_tree(&sample(), &mut dst, root).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Io);
    assert_eq!(
        err.downcast_device::<MemError>(),
        Some(&MemError::Timeout { lba: 9 })
    );
    assert!(err.path().is_some());
    assert_eq!((dst.open_nodes(), dst.open_files()), (1, 0));
}

#[test]
fn volumes_copy_through_trees() {
    let vol = Volume::new(fixture());
    let tree = read_tree(&vol, "/etc").unwrap();
    let mut dst = MemFs::new();
    let root = dst.root();
    let report = copy_tree(&tree, &mut dst, root).unwrap();
    assert_eq!(dst.contents("/conf").unwrap(), b"key=value");
    assert_eq!(
        report
            .warnings()
            .iter()
            .filter(|w| w.kind() == WarningKind::Skipped)
            .count(),
        1
    );
    drop(tree);
    assert_eq!(vol.into_inner().ok().unwrap().open_nodes(), 1);
}

/// A unique scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "hadris-fs-{test}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn host_round_trip() {
    let scratch = Scratch::new("round-trip");
    let out = scratch.0.join("out");
    let time = DateTime::from_unix_seconds(1_600_000_000).unwrap();
    let mut tree = sample();
    tree.insert(
        "deep/er/file",
        file(b"nested").with_attrs(SetAttr::new().with_permissions(Permissions::new(0o600))),
    )
    .unwrap();
    tree.replace(
        "deep",
        Node::dir().with_attrs(SetAttr::new().with_modified(time)),
    )
    .unwrap();
    tree.link("a.txt", "deep/hard").unwrap();
    tree.insert("link", Node::symlink("etc")).unwrap();
    tree.insert("fifo", Node::special(FileType::Fifo, None))
        .unwrap();
    let report = host::write_tree(&out, &tree).unwrap();
    assert_eq!(std::fs::read(out.join("etc/conf")).unwrap(), b"key=value");
    assert_eq!(std::fs::read(out.join("deep/er/file")).unwrap(), b"nested");
    assert_eq!(std::fs::read(out.join("deep/hard")).unwrap(), b"root a");
    let modified = std::fs::metadata(out.join("deep"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        1_600_000_000
    );
    let kinds: Vec<_> = report.warnings().iter().map(|w| w.kind()).collect();
    assert!(kinds.contains(&WarningKind::Skipped), "{kinds:?}");
    assert!(
        kinds.contains(&WarningKind::Dropped(Field::Owner)),
        "{kinds:?}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        assert_eq!(
            std::fs::read_link(out.join("link")).unwrap(),
            PathBuf::from("etc")
        );
        let mode = std::fs::metadata(out.join("deep/er/file"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let a = std::fs::metadata(out.join("a.txt")).unwrap();
        let hard = std::fs::metadata(out.join("deep/hard")).unwrap();
        assert_eq!(a.ino(), hard.ino());
    }

    let (back, skipped) = host::read_tree(&out, &TreeOptions::new()).unwrap();
    assert!(skipped.is_empty());
    let mut reader =
        hadris_fs::sync::ContentReader::open(back.get("deep/er/file").unwrap().content().unwrap())
            .unwrap();
    let mut buf = [0u8; 6];
    reader.read_exact_at(0, &mut buf).unwrap();
    assert_eq!(&buf, b"nested");
    #[cfg(unix)]
    {
        assert_eq!(back.get("link").unwrap().target(), Some(&b"etc"[..]));
        assert_eq!(
            back.entry("a.txt").unwrap().id(),
            back.entry("deep/hard").unwrap().id()
        );
        assert_eq!(
            back.get("deep/er/file")
                .unwrap()
                .attrs()
                .permissions()
                .map(|p| p.bits() & 0o777),
            Some(0o600)
        );
    }
    assert_eq!(back.get("deep").unwrap().attrs().modified(), Some(time));

    let err = host::write_tree(&out, &tree).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert!(err.path().is_some() && err.host_path().is_some());
}

#[test]
fn read_tree_options() {
    let scratch = Scratch::new("options");
    let root = scratch.0.join("tree");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for name in [
        "zeta", "Alpha", "beta", "10", "9", "sub/b", "sub/a", "skip.o",
    ] {
        std::fs::write(root.join(name), name).unwrap();
    }
    let limit = DateTime::from_unix_seconds(86_400).unwrap();
    let options = TreeOptions::new()
        .with_exclude(|path| path.extension().is_some_and(|ext| ext == "o"))
        .with_owner(Owner::new(0, 0))
        .with_clamp(limit);
    let (tree, skipped) = host::read_tree(&root, &options).unwrap();
    assert!(skipped.is_empty());
    let names: Vec<_> = tree
        .root()
        .children()
        .map(|(name, _)| name.to_string())
        .collect();
    assert_eq!(names, ["10", "9", "Alpha", "beta", "sub", "zeta"]);
    let beta = tree.get("beta").unwrap().attrs();
    assert_eq!(beta.modified(), Some(limit));
    assert_eq!(beta.owner(), Some(Owner::new(0, 0)));
    assert_eq!(tree.root().node().attrs().modified(), Some(limit));

    assert_eq!(
        host::read_tree(root.join("beta"), &TreeOptions::new())
            .unwrap_err()
            .kind(),
        ErrorKind::NotADirectory
    );
}

#[cfg(unix)]
#[test]
fn read_tree_symlink_and_error_policies() {
    let scratch = Scratch::new("pol");
    let root = scratch.0.clone();
    std::fs::create_dir_all(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/file"), b"data").unwrap();
    std::os::unix::fs::symlink("dir/file", root.join("to-file")).unwrap();
    std::os::unix::fs::symlink("missing", root.join("dangling")).unwrap();
    std::os::unix::fs::symlink("..", root.join("dir/up")).unwrap();

    let (tree, _) = host::read_tree(&root, &TreeOptions::new()).unwrap();
    assert_eq!(
        tree.get("dangling").unwrap().target(),
        Some(&b"missing"[..])
    );
    let (tree, _) =
        host::read_tree(&root, &TreeOptions::new().with_symlinks(Symlinks::Skip)).unwrap();
    assert!(tree.get("to-file").is_none());
    let err =
        host::read_tree(&root, &TreeOptions::new().with_symlinks(Symlinks::Fail)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Symlink);

    let follow = TreeOptions::new()
        .with_symlinks(Symlinks::Follow)
        .with_on_error(OnError::Skip);
    let (tree, skipped) = host::read_tree(&root, &follow).unwrap();
    assert_eq!(tree.get("to-file").unwrap().content().unwrap().len(), 4);
    let paths: Vec<_> = skipped
        .iter()
        .map(|err| err.path().unwrap().to_vec())
        .collect();
    assert_eq!(paths, [b"/dangling".to_vec(), b"/dir/up".to_vec()]);
    assert!(host::read_tree(&root, &TreeOptions::new().with_symlinks(Symlinks::Follow)).is_err());

    let socket = root.join("s");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let (tree, _) = host::read_tree(&root, &TreeOptions::new()).unwrap();
    assert_eq!(tree.get("s").unwrap().file_type(), FileType::Socket);
}

#[cfg(unix)]
#[test]
fn write_tree_never_follows_host_symlinks() {
    let scratch = Scratch::new("symlink");
    let outside = scratch.0.join("outside");
    std::fs::write(&outside, b"keep").unwrap();
    let out = scratch.0.join("out");
    std::fs::create_dir_all(out.join("etc")).unwrap();
    std::os::unix::fs::symlink(&outside, out.join("etc/conf")).unwrap();

    let err = host::write_tree(&out, &sample()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");

    std::fs::remove_file(out.join("etc/conf")).unwrap();
    std::fs::remove_dir(out.join("etc")).unwrap();
    std::os::unix::fs::symlink(&scratch.0, out.join("etc")).unwrap();
    let err = host::write_tree(&out, &sample()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert!(!scratch.0.join("conf").exists());
}

#[test]
fn write_tree_refuses_names_that_leave_the_directory() {
    let scratch = Scratch::new("names");
    for name in [&b"a\\b"[..], b"c:\\x"] {
        let mut tree = Tree::new();
        tree.insert(name, file(b"x")).unwrap();
        let err = host::write_tree(&scratch.0, &tree).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{name:?}");
    }
    assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 0);
}

#[test]
fn source_date_epoch_is_read_from_the_environment() {
    assert!(host::source_date_epoch().is_ok());
}
