//! `copy_tree` between filesystems on every tier (S4), and the std host
//! helpers, which never write outside their target directory.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::MemError;
use common::sync::{MemFs, fixture};
use hadris_fs::sync::{
    DriverExt, FsDriver, PathExt, TreeExt, Volume, copy_tree, extract_to_host, import_from_host,
};
use hadris_fs::tree::Tree;
use hadris_fs::{AnyError, ErrorKind, NewNode};

fn link_target(fs: &mut MemFs, path: &str) -> Vec<u8> {
    let node = fs.resolve(path).unwrap();
    let mut buf = [0u8; 64];
    let n = fs.read_link(node, &mut buf).unwrap();
    fs.forget(node);
    buf[..n].to_vec()
}

#[test]
fn copies_between_tiers() {
    let mut src = fixture();
    let shared = Volume::local(MemFs::new());
    copy_tree(&mut src, "/", &shared, "/backup").unwrap();
    assert_eq!(src.open_nodes(), 1);
    assert_eq!(
        shared.read_to_vec("/backup/etc/conf").unwrap(),
        b"key=value"
    );
    assert_eq!(shared.read_to_vec("/backup/a.txt").unwrap(), b"root a");

    let owned = Arc::new(Volume::new(MemFs::new()));
    copy_tree(&shared, "/backup/etc", Arc::clone(&owned), "/").unwrap();
    assert_eq!(owned.read_to_vec("/conf").unwrap(), b"key=value");
    assert!(owned.metadata("/up").unwrap().file_type().is_symlink());

    let mut shared = shared.into_inner();
    assert_eq!(link_target(&mut shared, "/backup/link"), b"etc");
    assert_eq!(link_target(&mut shared, "/backup/loop"), b"loop");
    assert_eq!(shared.open_nodes(), 1);
    let owned = Arc::into_inner(owned).unwrap().into_inner();
    assert_eq!(owned.open_nodes(), 1);
}

#[test]
fn copies_one_file_and_overwrites() {
    let mut src = fixture();
    let mut dst = MemFs::new();
    dst.write_file("/conf", b"a much longer old value").unwrap();
    copy_tree(&mut src, "/etc/conf", &mut dst, "/conf").unwrap();
    assert_eq!(dst.read_to_vec("/conf").unwrap(), b"key=value");

    let big: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
    src.write_file("/big", &big).unwrap();
    copy_tree(&mut src, "/", &mut dst, "/").unwrap();
    assert_eq!(dst.read_to_vec("/big").unwrap(), big);
    src.write_file("/big", b"shorter").unwrap();
    copy_tree(&mut src, "/big", &mut dst, "/big").unwrap();
    assert_eq!(dst.read_to_vec("/big").unwrap(), b"shorter");
    let err = copy_tree(&mut src, "/etc", &mut dst, "/etc").unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::AlreadyExists,
        "symlinks are never replaced"
    );
    assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
}

#[test]
fn conflicts_and_failures_release_pins() {
    let mut src = fixture();
    let mut dst = MemFs::new();
    dst.create_dir_all("/out").unwrap();
    dst.write_file("/out/etc", b"not a directory").unwrap();
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
    let err: AnyError = copy_tree(&mut src, "/etc", &mut dst, "/copy").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Io);
    assert_eq!(
        err.downcast_device::<MemError>(),
        Some(&MemError::Timeout { lba: 9 })
    );
    assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
}

#[test]
fn cyclic_directories_are_corrupt() {
    let scratch = Scratch::new("cycle");
    let cases = [("/etc", "/etc"), ("/etc", "/"), ("/deep/er", "/deep")];
    for (case, (dir, target)) in cases.into_iter().enumerate() {
        let mut src = fixture();
        src.create_dir_all("/deep/er").unwrap();
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
        #[cfg(not(unix))]
        for link in ["/link", "/abs", "/loop", "/long", "/etc/up"] {
            src.remove_file(link).unwrap();
        }
        let err = extract_to_host(&mut src, "/", scratch.0.join(case.to_string())).unwrap_err();
        let kind = err
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<ErrorKind>());
        assert_eq!(kind, Some(&ErrorKind::Corrupt), "{dir} -> {target}: {err}");
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
            NewNode::Dir,
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
    src.create_dir_all("/deep/er").unwrap();
    src.write_file("/deep/er/file", b"nested").unwrap();
    #[cfg(not(unix))]
    for link in ["/link", "/abs", "/loop", "/long", "/etc/up"] {
        src.remove_file(link).unwrap();
    }
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

    let vol = Volume::local(MemFs::new());
    import_from_host(&out, &vol, "/imported").unwrap();
    assert_eq!(
        vol.read_to_vec("/imported/deep/er/file").unwrap(),
        b"nested"
    );
    assert_eq!(vol.read_to_vec("/imported/etc/conf").unwrap(), b"key=value");
    import_from_host(scratch.0.join("single.txt"), &vol, "/single").unwrap();
    assert_eq!(vol.read_to_vec("/single").unwrap(), b"root a");
    let mut fs = vol.into_inner();
    #[cfg(unix)]
    assert_eq!(link_target(&mut fs, "/imported/etc/up"), b"../etc");
    assert_eq!(fs.open_nodes(), 1);

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
    fs.add("/", "tree", NewNode::Dir, b"");
    fs.add("/tree", "x", NewNode::Dir, b"");
    let err = import_from_host(scratch.0.join("tree"), &mut fs, "/tree").unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs.open_nodes(), 1);

    #[cfg(unix)]
    {
        let socket = scratch.0.join("socket");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let err = import_from_host(&socket, &mut fs, "/socket").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        assert!(!fs.exists("/socket").unwrap());
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
    let names = |fs: &mut MemFs, path: &str| -> Vec<String> {
        fs.read_dir(path)
            .unwrap()
            .map(|item| item.unwrap().name_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        names(&mut fs, "/"),
        ["10", "9", "Alpha", "beta", "sub", "zeta"]
    );
    assert_eq!(names(&mut fs, "/sub"), ["a", "b"]);
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn long_symlink_targets_are_refused() {
    let mut src = MemFs::new();
    src.add("/", "huge", NewNode::Symlink(&[b'a'; 4097]), b"");
    let mut dst = MemFs::new();
    let err = copy_tree(&mut src, "/huge", &mut dst, "/huge").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
    assert!(!dst.exists("/huge").unwrap());

    src.add("/", "max", NewNode::Symlink(&[b'a'; 4096]), b"");
    copy_tree(&mut src, "/max", &mut dst, "/max").unwrap();
    assert_eq!(link_target_len(&mut dst, "/max"), 4096);

    #[cfg(unix)]
    {
        let scratch = Scratch::new("long-link");
        let err = extract_to_host(&mut src, "/huge", scratch.0.join("huge")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
        assert!(std::fs::symlink_metadata(scratch.0.join("huge")).is_err());
    }
    assert_eq!((src.open_nodes(), dst.open_nodes()), (1, 1));
}

fn link_target_len(fs: &mut MemFs, path: &str) -> usize {
    let node = fs.resolve(path).unwrap();
    let mut buf = [0u8; 8192];
    let n = fs.read_link(node, &mut buf).unwrap();
    fs.forget(node);
    n
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
    assert_eq!(src.open_nodes(), 1);
}
