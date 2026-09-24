//! `Tree` built from a mounted filesystem and read back through
//! `ContentReader` in each mode.

#![cfg(all(feature = "sync", feature = "std"))]

mod common;

use common::sync::fixture;
use hadris_fs::ErrorKind;
use hadris_fs::sync::{ContentReader, TreeExt, Volume};
use hadris_fs::tree::{Content, NodeKind, Tree};

#[test]
fn from_filesystem_reads_every_tier() {
    let mut fs = fixture();
    let tree = Tree::from_filesystem(&mut fs).unwrap();
    assert_eq!(fs.open_nodes(), 1);
    match tree.get("etc/conf").unwrap().kind() {
        NodeKind::File(content) => assert_eq!(content.as_bytes(), Some(&b"key=value"[..])),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        tree.get("link").unwrap().kind(),
        NodeKind::Symlink(b"etc")
    ));

    let shared = Volume::local(fixture());
    let tree = Tree::from_filesystem(&shared).unwrap();
    assert!(tree.get("etc/up").is_some());
    assert!(tree.warnings().is_empty());
}

#[test]
fn content_reader_reads_every_kind() {
    let mut buf = [0u8; 8];
    let bytes = Content::bytes("hello");
    let mut reader = ContentReader::open(&bytes).unwrap();
    assert_eq!((reader.len(), reader.read_at(1, &mut buf).unwrap()), (5, 4));
    assert_eq!(&buf[..4], b"ello");
    assert_eq!(reader.read_at(5, &mut buf).unwrap(), 0);

    let source = Content::source(vec![9u8; 20]);
    let mut reader = ContentReader::open(&source).unwrap();
    reader.read_exact_at(12, &mut buf).unwrap();
    assert_eq!(buf, [9; 8]);
    assert_eq!(
        reader.read_exact_at(13, &mut buf).unwrap_err().kind(),
        ErrorKind::Corrupt
    );

    let path = std::env::temp_dir().join(format!("hadris-content-{}", std::process::id()));
    std::fs::write(&path, b"from the host").unwrap();
    let host = Content::path(&path);
    let mut reader = ContentReader::open(&host).unwrap();
    assert_eq!(reader.len(), 13);
    reader.read_exact_at(5, &mut buf).unwrap();
    assert_eq!(&buf, b"the host");
    std::fs::remove_file(&path).unwrap();

    let stored = Content::stored([hadris_fs::tree::Extent::new(0, 4)]);
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
    let content = Content::path(disk);
    let mut reader = ContentReader::open(&content).unwrap();
    assert_eq!(reader.len(), len);
    let mut block = [0u8; 512];
    reader.read_exact_at(len - 512, &mut block).unwrap();
}

#[cfg(feature = "async-send")]
#[test]
fn async_writers_read_async_sources() {
    use hadris_fs::r#async::ContentReader as AsyncReader;
    use hadris_fs::async_send::ContentReader as SendReader;

    let content = Content::async_source(vec![3u8; 10]);
    let mut buf = [0u8; 4];
    let mut reader = common::block_on(SendReader::open(&content)).unwrap();
    assert_eq!(common::block_on(reader.read_at(8, &mut buf)).unwrap(), 2);
    let reader = common::block_on(AsyncReader::open(&content)).unwrap();
    assert_eq!(reader.len(), 10);
    assert_eq!(
        ContentReader::open(&content).unwrap_err().kind(),
        ErrorKind::Unsupported
    );

    let blocking = Content::source(vec![1u8; 3]);
    let mut reader = common::block_on(AsyncReader::open(&blocking)).unwrap();
    assert_eq!(common::block_on(reader.read_at(0, &mut buf)).unwrap(), 3);
}
