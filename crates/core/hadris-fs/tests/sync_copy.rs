//! `copy_tree` between filesystems (S4), and the std host helpers, which
//! never write outside their target directory.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use std::path::PathBuf;

use common::MemError;
use common::sync::{MemFs, fixture};
use hadris_fs::sync::{FileSystem, TreeExt, copy_tree, extract_to_host, import_from_host};
use hadris_fs::tree::Tree;
use hadris_fs::{ErrorKind, FileType, Name, OpenMode, PathError, Resolve, SetAttr};

/// The fixture without its symlinks, which the shared trait cannot create.
fn plain() -> MemFs {
    let mut fs = fixture();
    let root = fs.root();
    for link in ["link", "abs", "loop", "long"] {
        fs.unlink(root, Name::new(link)).unwrap();
    }
    let etc = fs.lookup(root, Name::new("etc")).unwrap();
    fs.unlink(etc, Name::new("up")).unwrap();
    fs.forget(etc, 1);
    fs
}

fn put(fs: &mut MemFs, path: &str, data: &[u8]) {
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let name = Name::new(path.rsplit('/').next().unwrap());
    let dir = fs.resolve(dir.as_bytes(), Resolve::Lexical).unwrap();
    let node = match fs.lookup(dir, name) {
        Ok(node) => node,
        Err(_) => fs.create(dir, name, &SetAttr::new()).unwrap(),
    };
    fs.open(node, OpenMode::Write).unwrap();
    fs.truncate(node, 0).unwrap();
    fs.write(node, 0, data).unwrap();
    fs.close(node).unwrap();
    fs.forget(node, 1);
    fs.forget(dir, 1);
}

fn mkdirs(fs: &mut MemFs, path: &str) {
    let mut dir = fs.root();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let name = Name::new(part);
        let next = match fs.lookup(dir, name) {
            Ok(node) => node,
            Err(_) => fs.mkdir(dir, name, &SetAttr::new()).unwrap(),
        };
        fs.forget(dir, 1);
        dir = next;
    }
    fs.forget(dir, 1);
}

#[test]
fn copies_trees_and_files() {
    let mut src = plain();
    let mut dst = MemFs::new();
    copy_tree(&mut src, "/", &mut dst, "/backup").unwrap();
    assert_eq!(dst.contents("/backup/etc/conf").unwrap(), b"key=value");
    assert_eq!(dst.contents("/backup/a.txt").unwrap(), b"root a");

    put(&mut dst, "/conf", b"a much longer old value");
    copy_tree(&mut src, "/etc/conf", &mut dst, "/conf").unwrap();
    assert_eq!(dst.contents("/conf").unwrap(), b"key=value");
    let big: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
    put(&mut src, "/big", &big);
    copy_tree(&mut src, "/", &mut dst, "/").unwrap();
    assert_eq!(dst.contents("/big").unwrap(), big);
    put(&mut src, "/big", b"shorter");
    copy_tree(&mut src, "/big", &mut dst, "/big").unwrap();
    assert_eq!(dst.contents("/big").unwrap(), b"shorter");
    assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
    assert_eq!((src.open_files(), dst.open_files()), (0, 0));
}

#[test]
fn conflicts_and_failures_release_pins() {
    let mut src = fixture();
    let mut dst = MemFs::new();
    let err = copy_tree(&mut src, "/", &mut dst, "/").unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::Unsupported,
        "symlinks cannot be created"
    );

    let mut src = plain();
    mkdirs(&mut dst, "/out");
    put(&mut dst, "/out/etc", b"not a directory");
    let err = copy_tree(&mut src, "/", &mut dst, "/out").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));

    let err = copy_tree(&mut src, "/missing", &mut dst, "/x").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let err = copy_tree(&mut src, "/a.txt", &mut dst, "/").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);

    let mut read_only = MemFs::new().read_only();
    let err = copy_tree(&mut src, "/etc", &mut read_only, "/etc").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ReadOnly);

    src.fail_next(MemError::Timeout { lba: 9 });
    let err: PathError = copy_tree(&mut src, "/etc", &mut dst, "/copy").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Io);
    assert_eq!(
        err.downcast_device::<MemError>(),
        Some(&MemError::Timeout { lba: 9 })
    );
    assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
    assert_eq!((src.open_files(), dst.open_files()), (0, 0));
}

#[test]
fn cyclic_directories_are_corrupt() {
    let scratch = Scratch::new("cycle");
    let cases = [("/etc", "/etc"), ("/etc", "/"), ("/deep/er", "/deep")];
    for (case, (dir, target)) in cases.into_iter().enumerate() {
        let mut src = plain();
        mkdirs(&mut src, "/deep/er");
        src.alias(dir, "back", target);
        let err = Tree::from_filesystem(&mut src).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt, "{dir} -> {target}");
        let mut dst = MemFs::new();
        let err = copy_tree(&mut src, "/", &mut dst, "/").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt, "{dir} -> {target}");
        if dir == "/deep/er" {
            let err = copy_tree(&mut src, "/deep", &mut dst, "/copy").unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Corrupt);
        }
        let err = extract_to_host(&mut src, "/", scratch.0.join(case.to_string())).unwrap_err();
        let kind = err
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<hadris_fs::Error<core::convert::Infallible>>())
            .map(hadris_fs::Error::kind);
        assert_eq!(kind, Some(ErrorKind::Corrupt), "{dir} -> {target}: {err}");
        assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
    }
}

#[test]
fn walks_stop_at_a_depth_limit() {
    let mut src = MemFs::new();
    let mut path = String::new();
    for _ in 0..1100 {
        src.add(
            if path.is_empty() { "/" } else { &path },
            "d",
            FileType::Dir,
            b"",
        );
        path.push_str("/d");
    }
    let err = Tree::from_filesystem(&mut src).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
    let mut dst = MemFs::new();
    let err = copy_tree(&mut src, "/", &mut dst, "/").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
    assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
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
    let mut src = fixture();
    mkdirs(&mut src, "/deep/er");
    put(&mut src, "/deep/er/file", b"nested");
    #[cfg(not(unix))]
    let mut src = {
        let mut plain = plain();
        mkdirs(&mut plain, "/deep/er");
        put(&mut plain, "/deep/er/file", b"nested");
        plain
    };
    extract_to_host(&mut src, "/", &out).unwrap();
    assert_eq!(src.open_nodes(), 1);
    assert_eq!(std::fs::read(out.join("etc/conf")).unwrap(), b"key=value");
    assert_eq!(std::fs::read(out.join("deep/er/file")).unwrap(), b"nested");
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_link(out.join("link")).unwrap(),
        PathBuf::from("etc")
    );

    extract_to_host(&mut src, "/a.txt", scratch.0.join("single.txt")).unwrap();
    assert_eq!(
        std::fs::read(scratch.0.join("single.txt")).unwrap(),
        b"root a"
    );

    let mut fs = MemFs::new();
    #[cfg(unix)]
    {
        let err = import_from_host(&out, &mut fs, "/imported").unwrap_err();
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::Unsupported,
            "symlinks cannot be created"
        );
        for link in ["link", "abs", "loop", "long", "etc/up"] {
            std::fs::remove_file(out.join(link)).unwrap();
        }
        fs = MemFs::new();
    }
    import_from_host(&out, &mut fs, "/imported").unwrap();
    assert_eq!(fs.contents("/imported/deep/er/file").unwrap(), b"nested");
    assert_eq!(fs.contents("/imported/etc/conf").unwrap(), b"key=value");
    import_from_host(scratch.0.join("single.txt"), &mut fs, "/single").unwrap();
    assert_eq!(fs.contents("/single").unwrap(), b"root a");
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));

    let err = import_from_host(scratch.0.join("missing"), &mut fs, "/x").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    let err = extract_to_host(&mut fs, "/missing", &out).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(fs.open_nodes(), 1);
}

#[cfg(unix)]
#[test]
fn extraction_never_follows_host_symlinks() {
    let scratch = Scratch::new("symlink");
    let outside = scratch.0.join("outside");
    std::fs::write(&outside, b"keep").unwrap();
    let out = scratch.0.join("out");
    std::fs::create_dir_all(out.join("etc")).unwrap();
    std::os::unix::fs::symlink(&outside, out.join("etc/conf")).unwrap();

    let mut src = fixture();
    let err = extract_to_host(&mut src, "/", &out).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    assert_eq!(src.open_nodes(), 1);

    std::fs::remove_file(out.join("etc/conf")).unwrap();
    std::fs::remove_dir(out.join("etc")).unwrap();
    std::os::unix::fs::symlink(&scratch.0, out.join("etc")).unwrap();
    let err = extract_to_host(&mut src, "/", &out).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(!scratch.0.join("conf").exists());
    assert_eq!(src.open_nodes(), 1);
}

#[test]
fn import_rejects_special_files_and_conflicts() {
    let scratch = Scratch::new("import");
    std::fs::create_dir_all(scratch.0.join("tree")).unwrap();
    std::fs::write(scratch.0.join("tree/x"), b"x").unwrap();
    let mut fs = MemFs::new();
    fs.add("/", "tree", FileType::Dir, b"");
    fs.add("/tree", "x", FileType::Dir, b"");
    let err = import_from_host(scratch.0.join("tree"), &mut fs, "/tree").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs.open_nodes(), 1);

    #[cfg(unix)]
    {
        let socket = scratch.0.join("socket");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let err = import_from_host(&socket, &mut fs, "/socket").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        assert!(fs.contents("/socket").is_none());
        assert_eq!(fs.open_nodes(), 1);
    }
}

#[test]
fn import_walks_host_directories_in_name_order() {
    let scratch = Scratch::new("sorted");
    let tree = scratch.0.join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    for name in ["zeta", "Alpha", "beta", "10", "9", "sub/b", "sub/a"] {
        std::fs::write(tree.join(name), name).unwrap();
    }
    let mut fs = MemFs::new();
    import_from_host(&tree, &mut fs, "/").unwrap();
    let vol = hadris_fs::sync::Volume::new(fs);
    assert_eq!(
        common::files::names(&vol, "/"),
        ["10", "9", "Alpha", "beta", "sub", "zeta"]
    );
    assert_eq!(common::files::names(&vol, "/sub"), ["a", "b"]);
    assert_eq!(vol.into_inner().unwrap().open_nodes(), 1);
}

#[cfg(unix)]
#[test]
fn long_symlink_targets_are_refused() {
    let mut src = MemFs::new();
    src.add("/", "huge", FileType::Symlink, &[b'a'; 4097]);
    let scratch = Scratch::new("long-link");
    let err = extract_to_host(&mut src, "/huge", scratch.0.join("huge")).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Other);
    assert!(std::fs::symlink_metadata(scratch.0.join("huge")).is_err());
    assert_eq!(src.open_nodes(), 1);
}

#[cfg(unix)]
#[test]
fn extraction_replaces_files_instead_of_truncating_them() {
    let scratch = Scratch::new("hard-link");
    let outside = scratch.0.join("outside");
    std::fs::write(&outside, b"keep").unwrap();
    let out = scratch.0.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::hard_link(&outside, out.join("a.txt")).unwrap();
    std::fs::write(out.join("link"), b"a file where the image has a link").unwrap();

    let mut src = fixture();
    extract_to_host(&mut src, "/", &out).unwrap();
    assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"root a");
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    assert_eq!(
        std::fs::read_link(out.join("link")).unwrap(),
        PathBuf::from("etc")
    );

    extract_to_host(&mut src, "/a.txt", &outside).unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"root a");
    assert_eq!((src.open_nodes(), src.open_files()), (1, 0));
}
