//! `read_tree` over a mounted volume and `ContentReader` in each mode.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use common::sync::{MemFs, fixture};
use hadris_fs::sync::{ContentReader, Volume, read_tree};
use hadris_fs::{Content, ErrorKind, Extent, FileType, host};

fn read_all(content: &Content) -> Vec<u8> {
    let mut reader = ContentReader::open(content).unwrap();
    let mut out = vec![0u8; reader.len() as usize];
    reader.read_exact_at(0, &mut out).unwrap();
    out
}

#[test]
fn read_tree_reads_content_lazily_through_the_volume() {
    let vol = Volume::new(fixture());
    let tree = read_tree(&vol, "/").unwrap();
    let conf = tree.get("etc/conf").unwrap();
    assert_eq!(conf.content().unwrap().len(), 9);
    assert_eq!(read_all(conf.content().unwrap()), b"key=value");
    assert_eq!(tree.get("link").unwrap().target(), Some(&b"etc"[..]));
    assert_eq!(tree.get("etc/up").unwrap().file_type(), FileType::Symlink);
    assert_eq!(vol.lock().open_files(), 0);

    let vol = match vol.into_inner() {
        Ok(_) => panic!("the tree holds the volume"),
        Err(vol) => vol,
    };
    let clone = tree.get("a.txt").unwrap().content().unwrap().clone();
    drop(tree);
    assert_eq!(read_all(&clone), b"root a");
    drop(clone);
    let fs = vol.into_inner().ok().unwrap();
    assert_eq!((fs.open_nodes(), fs.open_files()), (1, 0));
}

#[test]
fn read_tree_of_a_file_holds_that_file() {
    let vol = Volume::new(fixture());
    let tree = read_tree(&vol, "/etc/conf").unwrap();
    let names: Vec<_> = tree
        .root()
        .children()
        .map(|(name, _)| name.as_bytes().to_vec())
        .collect();
    assert_eq!(names, [b"conf".to_vec()]);
    let err = read_tree(&vol, "/missing").unwrap_err();
    assert_eq!(
        (err.kind(), err.path()),
        (ErrorKind::NotFound, Some(&b"/missing"[..]))
    );
    drop(tree);
    assert_eq!(vol.into_inner().ok().unwrap().open_nodes(), 1);
}

#[test]
fn read_tree_stops_at_cycles_and_depth() {
    for (dir, target) in [("/etc", "/etc"), ("/etc", "/")] {
        let mut fs = fixture();
        fs.alias(dir, "back", target);
        let vol = Volume::new(fs);
        let err = read_tree(&vol, "/").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt, "{dir} -> {target}");
        assert!(err.path().is_some());
        assert_eq!(vol.into_inner().ok().unwrap().open_nodes(), 1);
    }
    let mut fs = MemFs::new();
    let mut path = String::new();
    for _ in 0..1100 {
        fs.add(
            if path.is_empty() { "/" } else { &path },
            "d",
            FileType::Dir,
            b"",
        );
        path.push_str("/d");
    }
    let vol = Volume::new(fs);
    assert_eq!(
        read_tree(&vol, "/").unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(vol.into_inner().ok().unwrap().open_nodes(), 1);
}

#[test]
fn content_reader_reads_every_kind() {
    let mut buf = [0u8; 8];
    let bytes = Content::bytes("hello");
    let mut reader = ContentReader::open(&bytes).unwrap();
    assert_eq!((reader.len(), reader.read_at(1, &mut buf).unwrap()), (5, 4));
    assert_eq!(&buf[..4], b"ello");
    assert_eq!(reader.read_at(5, &mut buf).unwrap(), 0);
    assert_eq!(
        reader.read_exact_at(1, &mut buf).unwrap_err().kind(),
        ErrorKind::Corrupt
    );

    let path = std::env::temp_dir().join(format!("hadris-content-{}", std::process::id()));
    std::fs::write(&path, b"from the host").unwrap();
    let host = host::file(&path).unwrap();
    assert_eq!(host.len(), 13);
    let mut reader = ContentReader::open(&host).unwrap();
    reader.read_exact_at(5, &mut buf).unwrap();
    assert_eq!(&buf, b"the host");
    std::fs::write(&path, b"changed").unwrap();
    let err = ContentReader::open(&host).unwrap_err();
    assert_eq!(
        (err.kind(), err.host_path()),
        (ErrorKind::Corrupt, Some(path.as_path()))
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(host::file(&path).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(
        host::file(std::env::temp_dir()).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );

    let stored = Content::stored([Extent::new(0, 4)]);
    assert_eq!(
        ContentReader::open(&stored).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

/// A disk device read as file content has the device's length, not the 0
/// that its metadata reports. Uses the device CI attaches at
/// `HADRIS_TEST_DISK` with `HADRIS_TEST_DISK_LEN` bytes, and skips when it
/// is unset unless `HADRIS_REQUIRE_TEST_DISK` is set.
#[test]
fn content_path_reads_a_disk_device() {
    let Some(disk) = std::env::var_os("HADRIS_TEST_DISK") else {
        assert!(
            std::env::var_os("HADRIS_REQUIRE_TEST_DISK").is_none(),
            "HADRIS_REQUIRE_TEST_DISK is set but HADRIS_TEST_DISK is not"
        );
        eprintln!("skipped: HADRIS_TEST_DISK is not set");
        return;
    };
    let len: u64 = std::env::var("HADRIS_TEST_DISK_LEN")
        .expect("HADRIS_TEST_DISK_LEN")
        .parse()
        .unwrap();
    let content = host::file(disk).unwrap();
    let mut reader = ContentReader::open(&content).unwrap();
    assert_eq!(reader.len(), len);
    let mut block = [0u8; 512];
    reader.read_exact_at(len - 512, &mut block).unwrap();
}

#[cfg(feature = "async")]
#[test]
fn lazy_content_is_read_in_its_own_mode() {
    use common::asynch;
    use hadris_fs::r#async::{
        ContentReader as AsyncReader, Volume as AsyncVolume, read_tree as read_tree_async,
    };

    let vol = AsyncVolume::new(asynch::fixture());
    let tree = common::block_on(read_tree_async(&vol, "/")).unwrap();
    let content = tree.get("etc/conf").unwrap().content().unwrap();
    let mut buf = [0u8; 16];
    let mut reader = common::block_on(AsyncReader::open(content)).unwrap();
    assert_eq!(common::block_on(reader.read_at(4, &mut buf)).unwrap(), 5);
    assert_eq!(&buf[..5], b"value");
    assert_eq!(
        ContentReader::open(content).unwrap_err().kind(),
        ErrorKind::Unsupported
    );

    let sync_vol = Volume::new(fixture());
    let sync_tree = read_tree(&sync_vol, "/").unwrap();
    let sync_content = sync_tree.get("a.txt").unwrap().content().unwrap();
    assert_eq!(
        common::block_on(AsyncReader::open(sync_content))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    let path = std::env::temp_dir().join(format!("hadris-async-content-{}", std::process::id()));
    std::fs::write(&path, b"x").unwrap();
    let host = host::file(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        common::block_on(AsyncReader::open(&host))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    drop(tree);
    assert_eq!(
        common::block_on(vol.into_inner())
            .ok()
            .unwrap()
            .open_nodes(),
        1
    );
}
