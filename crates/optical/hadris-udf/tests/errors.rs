//! Damaged volumes and options the writer cannot honour fail with the
//! documented kinds, without panicking.

mod common;

use common::Paths;
use common::{SECTOR, image, open, reseal, sample};
use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, Node, Tree};
use hadris_fs::{ErrorKind, Extent, NodeId, SetAttr};
use hadris_storage::{BlockSize, MemDevice};
use hadris_udf::sync::UdfFs;
use hadris_udf::{Detail, UdfId, UdfOptions, UdfRevision};

fn good() -> Vec<u8> {
    image(&sample(), &UdfOptions::default())
}

fn open_err(bytes: Vec<u8>) -> ErrorKind {
    UdfFs::mount(MemDevice::new(bytes, SECTOR), MountOptions::new())
        .unwrap_err()
        .kind()
}

#[test]
fn malformed_volumes_are_refused() {
    let good = good();
    assert_eq!(open_err(vec![0u8; good.len()]), ErrorKind::NotRecognized);

    let mut bad = good.clone();
    bad[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"XXXXX");
    assert_eq!(open_err(bad), ErrorKind::NotRecognized);

    let mut bad = good.clone();
    bad[290 * 2048 + 100] ^= 1;
    assert_eq!(open_err(bad), ErrorKind::Corrupt);

    let mut bad = good.clone();
    bad[291 * 2048 + 60] ^= 1;
    assert_eq!(open_err(bad), ErrorKind::Corrupt);

    let mut bad = good.clone();
    for sector in [256, good.len() / 2048 - 257] {
        bad[sector * 2048 + 4] ^= 1;
    }
    assert_eq!(open_err(bad), ErrorKind::Corrupt);

    let mut bad = good.clone();
    for sector in [257, 273] {
        bad[sector * 2048 + 30] ^= 1;
    }
    assert_eq!(open_err(bad), ErrorKind::Corrupt);

    let mut truncated = good.clone();
    truncated.truncate(300 * 2048);
    if let Ok(mut udf) = UdfFs::mount(MemDevice::new(truncated, SECTOR), MountOptions::new()) {
        assert!(udf.read_to_vec("/docs/big.bin").is_err());
    }

    let mut bad = good.clone();
    let root_fids = 292 * 2048;
    let name = bad[root_fids..root_fids + 2048]
        .windows(9)
        .position(|w| w == b"\x08readme.t")
        .unwrap();
    bad[root_fids + name + 1] = b'R';
    let mut udf = open(bad);
    assert_eq!(udf.names("/").unwrap_err().kind(), ErrorKind::Corrupt);

    let mut udf = UdfFs::mount(
        MemDevice::new(good, BlockSize::new(4096).unwrap()),
        MountOptions::new(),
    )
    .unwrap();
    assert_eq!(udf.read_to_vec("/readme.txt").unwrap(), b"hello");
}

#[test]
fn damaged_entries_behind_listed_ids_are_corrupt() {
    let good = good();
    let mut udf = open(good.clone());
    let node = udf.resolve_path("/readme.txt").unwrap();
    let block = ((node.get() - 1) & 0xFFFF_FFFF) as u32;
    let sector = (0..good.len() / 2048)
        .find(|&sector| {
            hadris_udf::raw::Tag::read(&good[sector * 2048..]).is_some_and(|tag| {
                tag.is_checksum_valid()
                    && matches!(tag.identifier.get(), 261 | 266)
                    && tag.location.get() == block
            })
        })
        .unwrap();
    let mut bad = good;
    bad[sector * 2048 + 100] ^= 0xFF;
    let mut udf = open(bad);
    let listed = udf.resolve_path("/readme.txt").unwrap();
    assert_eq!(listed, node);
    assert_eq!(udf.stat(listed).unwrap_err().kind(), ErrorKind::Corrupt);
    let names = udf.names("/").unwrap();
    assert!(names.iter().any(|name| name == "readme.txt"), "{names:?}");
    assert!(names.iter().any(|name| name == "docs"), "{names:?}");
    let root = udf.root();
    let mut cursor = hadris_fs::DirCursor::START;
    while let Some(entry) = udf.readdir(root, cursor).unwrap() {
        if entry.node() == node {
            assert_eq!(entry.metadata().len(), 0);
            assert_eq!(
                entry.metadata().permissions(),
                hadris_fs::Permissions::new(0)
            );
        }
        cursor = entry.next_cursor();
    }
}

#[test]
fn forged_node_ids_are_invalid_handles() {
    let mut udf = open(good());
    for raw in [12345, u64::MAX, (5u64 << 32) + 2] {
        let err = udf.stat(NodeId::new(raw).unwrap()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidHandle, "{raw}");
    }
    let err = udf.stat(NodeId::new(1).unwrap()).unwrap_err();
    assert_eq!(
        err.kind(),
        ErrorKind::Corrupt,
        "an id inside a partition is read as an entry"
    );
    let file = udf.resolve_path("/readme.txt").unwrap();
    assert_eq!(
        udf.lookup(file, hadris_fs::Name::new("x"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotADirectory
    );
    let root = udf.root();
    assert_eq!(
        udf.read(root, 0, &mut [0; 4]).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
    assert_eq!(
        udf.readlink(file, &mut [0; 4]).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let link = udf.resolve_path("/abs").unwrap();
    assert_eq!(
        udf.readlink(link, &mut [0; 4]).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}

#[test]
fn reserve_sequence_and_backup_anchor_are_used() {
    let good = good();
    let mut bad = good.clone();
    bad[256 * 2048 + 4] ^= 1;
    assert_eq!(open(bad).read_to_vec("/readme.txt").unwrap(), b"hello");

    let mut bad = good;
    bad[257 * 2048 + 30] ^= 1;
    let mut udf = open(bad);
    assert_eq!(udf.info().id(hadris_udf::UdfId::Volume), "UDF_VOLUME");
    assert_eq!(udf.read_to_vec("/docs/sub/deep.txt").unwrap(), b"deep");
}

#[test]
fn backup_boot_reads_the_end_anchors_and_the_reserve_sequence() {
    let good = good();
    let backup = |bytes: Vec<u8>| {
        UdfFs::mount(
            MemDevice::new(bytes, SECTOR),
            MountOptions::new().backup_boot(),
        )
    };
    let mut reserve = good.clone();
    reserve[273 * 2048 + 25] = b'X';
    reseal(&mut reserve, 273, 496);
    assert_eq!(
        open(reserve.clone()).info().id(hadris_udf::UdfId::Volume),
        "UDF_VOLUME"
    );
    let mut udf = backup(reserve).unwrap();
    assert_eq!(udf.info().id(hadris_udf::UdfId::Volume), "XDF_VOLUME");
    assert_eq!(udf.read_to_vec("/readme.txt").unwrap(), b"hello");

    let mut ends = good.clone();
    let last = good.len() / 2048 - 1;
    for sector in [last - 256, last] {
        ends[sector * 2048 + 4] ^= 1;
    }
    assert_eq!(
        open(ends.clone()).read_to_vec("/readme.txt").unwrap(),
        b"hello"
    );
    assert_eq!(backup(ends).unwrap_err().kind(), ErrorKind::Corrupt);

    let mut main = good;
    main[256 * 2048 + 4] ^= 1;
    main[257 * 2048 + 30] ^= 1;
    assert_eq!(
        backup(main).unwrap().read_to_vec("/readme.txt").unwrap(),
        b"hello"
    );
}

#[test]
fn unsupported_partition_maps_are_refused() {
    let mut bad = good();
    for sector in [260usize, 276] {
        let at = sector * 2048;
        bad[at + 264..at + 268].copy_from_slice(&64u32.to_le_bytes());
        bad[at + 440] = 2;
        bad[at + 441] = 64;
        reseal(&mut bad, sector as u64, 496);
    }
    assert_eq!(open_err(bad), ErrorKind::Unsupported);
}

#[test]
fn the_writer_refuses_what_it_cannot_store() {
    let tree = sample();
    let err = hadris_udf::plan(
        &tree,
        &UdfOptions::default().with_id(UdfId::Volume, &"x".repeat(127)),
    )
    .unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.detail().and_then(hadris_udf::Detail::from_code)
        ),
        (ErrorKind::InvalidInput, Some(Detail::Identifier))
    );

    for revision in [UdfRevision::V2_50, UdfRevision::V2_60] {
        let options = UdfOptions::default().with_revision(revision);
        let err = hadris_udf::plan(&tree, &options).unwrap_err();
        assert_eq!(
            (
                err.kind(),
                err.detail().and_then(hadris_udf::Detail::from_code)
            ),
            (ErrorKind::Unsupported, Some(Detail::PartitionMap)),
            "UDF {revision} needs a metadata partition"
        );
        let mut dev = MemDevice::new(vec![0u8; 1 << 20], SECTOR);
        let err = hadris_udf::sync::write(&mut dev, &tree, &options).unwrap_err();
        assert_eq!(
            err.detail().and_then(hadris_udf::Detail::from_code),
            Some(Detail::PartitionMap)
        );
        assert!(dev.get_ref().iter().all(|&byte| byte == 0));
    }

    let mut long = Tree::new();
    long.insert("n".repeat(255), Node::file(Content::empty()))
        .unwrap();
    let err = hadris_udf::plan(&long, &UdfOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NameTooLong);

    let mut late = Tree::new();
    let far = hadris_fs::DateTime::from_unix_seconds(253_402_300_800).unwrap();
    late.insert(
        "f",
        Node::file(Content::empty()).with_attrs(SetAttr::new().with_modified(far)),
    )
    .unwrap();
    let err = hadris_udf::plan(&late, &UdfOptions::default()).unwrap_err();
    assert_eq!(
        err.detail().and_then(hadris_udf::Detail::from_code),
        Some(Detail::Timestamp)
    );

    let mut stored = Tree::new();
    stored
        .insert(
            "f",
            Node::file(Content::stored([Extent::new(2048 * 400, 10)]).unwrap()),
        )
        .unwrap();
    let err = hadris_udf::plan(&stored, &UdfOptions::default()).unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.detail().and_then(hadris_udf::Detail::from_code)
        ),
        (ErrorKind::Unsupported, Some(Detail::StoredContent))
    );

    let mut dev = MemDevice::new(vec![0u8; 4 << 20], BlockSize::new(4096).unwrap());
    let err = hadris_udf::sync::write(&mut dev, &tree, &UdfOptions::default()).unwrap_err();
    assert_eq!(
        err.detail().and_then(hadris_udf::Detail::from_code),
        Some(Detail::OutputBlockSize)
    );
    assert!(dev.get_ref().iter().all(|&b| b == 0));
}

#[derive(Debug)]
struct FailingDevice {
    inner: MemDevice<Vec<u8>>,
    fail: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl FailingDevice {
    fn new(bytes: Vec<u8>, fail: u64) -> Self {
        Self {
            inner: MemDevice::new(bytes, SECTOR),
            fail: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(fail)),
        }
    }

    fn read(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<std::io::Error>> {
        use hadris_storage::sync::BlockDevice;
        let fail = self.fail.load(std::sync::atomic::Ordering::Relaxed);
        if first.get() <= fail && fail - first.get() < (buf.len() / 2048) as u64 {
            return Err(hadris_io::Error::device(
                std::io::Error::from_raw_os_error(5),
                "injected device failure",
            ));
        }
        self.inner
            .read_blocks(first, buf)
            .map_err(|err| err.map_device(|never| match never {}))
    }
}

impl hadris_io::ErrorType for FailingDevice {
    type Error = std::io::Error;
}

impl hadris_storage::sync::BlockDevice for FailingDevice {
    fn block_size(&self) -> BlockSize {
        SECTOR
    }
    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(&self.inner)
    }
    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.read(first, buf)
    }
}

#[cfg(feature = "async")]
impl hadris_storage::r#async::BlockDevice for FailingDevice {
    fn block_size(&self) -> BlockSize {
        SECTOR
    }
    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(&self.inner)
    }
    async fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.read(first, buf)
    }
}

fn assert_device_failure(err: hadris_io::Error<std::io::Error>) {
    assert_eq!(err.kind(), ErrorKind::Io);
    assert_eq!(err.into_device_error().unwrap().raw_os_error(), Some(5));
}

#[test]
fn reserve_sequence_preserves_device_failures() {
    let base = good();
    for backup in [false, true] {
        let (primary, fallback) = if backup { (273, 257) } else { (257, 273) };
        let mut bytes = base.clone();
        bytes[primary * 2048 + 30] ^= 1;
        let options = if backup {
            MountOptions::new().backup_boot()
        } else {
            MountOptions::new()
        };
        let failed =
            UdfFs::mount(FailingDevice::new(bytes.clone(), fallback as u64), options).unwrap_err();
        assert_device_failure(failed.into_error());
        #[cfg(feature = "async")]
        common::block_on(async {
            let failed = hadris_udf::r#async::UdfFs::mount(
                FailingDevice::new(bytes, fallback as u64),
                options,
            )
            .await
            .unwrap_err();
            assert_device_failure(failed.into_error());
        });
    }
}

fn symlink_image() -> (Vec<u8>, NodeId, u64) {
    let mut tree = Tree::new();
    tree.insert("link", Node::symlink("target")).unwrap();
    let bytes = image(&tree, &UdfOptions::default());
    let node = open(bytes.clone()).resolve_path("/link").unwrap();
    let icb = 290 + node.get() - 1;
    let entry = &bytes[icb as usize * 2048..];
    let payload = 290 + u64::from(u32::from_le_bytes(entry[180..184].try_into().unwrap()));
    (bytes, node, payload)
}

#[test]
fn symlink_metadata_preserves_device_failures() {
    let (bytes, node, payload) = symlink_image();
    let dev = FailingDevice::new(bytes.clone(), u64::MAX);
    let fail = dev.fail.clone();
    let mut udf = UdfFs::mount(dev, MountOptions::new()).unwrap();
    assert_eq!(udf.stat(node).unwrap().len(), 6);
    fail.store(payload, std::sync::atomic::Ordering::Relaxed);
    assert_device_failure(udf.stat(node).unwrap_err());
    assert_device_failure(udf.readlink(node, &mut [0; 64]).unwrap_err());
    assert_device_failure(
        udf.readdir(udf.root(), hadris_fs::DirCursor::START)
            .unwrap_err(),
    );
    #[cfg(feature = "async")]
    common::block_on(async {
        use hadris_fs::r#async::FileSystem;
        let dev = FailingDevice::new(bytes, u64::MAX);
        let fail = dev.fail.clone();
        let mut udf = hadris_udf::r#async::UdfFs::mount(dev, MountOptions::new())
            .await
            .unwrap();
        assert_eq!(udf.stat(node).await.unwrap().len(), 6);
        fail.store(payload, std::sync::atomic::Ordering::Relaxed);
        assert_device_failure(udf.stat(node).await.unwrap_err());
        assert_device_failure(udf.readlink(node, &mut [0; 64]).await.unwrap_err());
        assert_device_failure(
            udf.readdir(udf.root(), hadris_fs::DirCursor::START)
                .await
                .unwrap_err(),
        );
    });
}

#[test]
fn malformed_symlinks_fail_stat_and_list_with_damaged_metadata() {
    let (mut bytes, node, payload) = symlink_image();
    bytes[payload as usize * 2048] = 99;
    let mut udf = open(bytes.clone());
    assert_eq!(udf.stat(node).unwrap_err().kind(), ErrorKind::Corrupt);
    assert_eq!(
        udf.readlink(node, &mut [0; 64]).unwrap_err().kind(),
        ErrorKind::Corrupt
    );
    let entry = udf
        .readdir(udf.root(), hadris_fs::DirCursor::START)
        .unwrap()
        .unwrap();
    assert_eq!(entry.node(), node);
    assert_eq!(entry.metadata().len(), 0);
    assert_eq!(
        entry.metadata().permissions(),
        hadris_fs::Permissions::new(0)
    );
    #[cfg(feature = "async")]
    common::block_on(async {
        use hadris_fs::r#async::FileSystem;
        let mut udf =
            hadris_udf::r#async::UdfFs::mount(MemDevice::new(bytes, SECTOR), MountOptions::new())
                .await
                .unwrap();
        assert_eq!(udf.stat(node).await.unwrap_err().kind(), ErrorKind::Corrupt);
        let entry = udf
            .readdir(udf.root(), hadris_fs::DirCursor::START)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.node(), node);
        assert_eq!(entry.metadata().len(), 0);
    });
}

#[test]
fn cached_symlink_reads_preserve_device_failures_and_retry() {
    let (bytes, node, payload) = symlink_image();
    let dev = FailingDevice::new(bytes.clone(), payload);
    let fail = dev.fail.clone();
    let mut udf = UdfFs::mount(
        hadris_storage::sync::Cache::new(dev, 8),
        MountOptions::new(),
    )
    .unwrap();
    assert_device_failure(udf.stat(node).unwrap_err());
    assert_device_failure(udf.readlink(node, &mut [0; 64]).unwrap_err());
    assert_device_failure(
        udf.readdir(udf.root(), hadris_fs::DirCursor::START)
            .unwrap_err(),
    );
    fail.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(udf.stat(node).unwrap().len(), 6);
    assert_eq!(udf.readlink(node, &mut [0; 64]).unwrap(), b"target");
    #[cfg(feature = "async")]
    common::block_on(async {
        use hadris_fs::r#async::FileSystem;
        let dev = FailingDevice::new(bytes, payload);
        let fail = dev.fail.clone();
        let mut udf = hadris_udf::r#async::UdfFs::mount(
            hadris_storage::r#async::Cache::new(dev, 8),
            MountOptions::new(),
        )
        .await
        .unwrap();
        assert_device_failure(udf.stat(node).await.unwrap_err());
        assert_device_failure(udf.readlink(node, &mut [0; 64]).await.unwrap_err());
        assert_device_failure(
            udf.readdir(udf.root(), hadris_fs::DirCursor::START)
                .await
                .unwrap_err(),
        );
        fail.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(udf.stat(node).await.unwrap().len(), 6);
        assert_eq!(udf.readlink(node, &mut [0; 64]).await.unwrap(), b"target");
    });
}

#[test]
fn malformed_identifier_fields_are_refused() {
    let good = good();
    let mut at = 292 * 2048;
    loop {
        let iu = usize::from(u16::from_le_bytes(
            good[at + 36..at + 38].try_into().unwrap(),
        ));
        let n = usize::from(good[at + 19]);
        if n > 0 && &good[at + 39 + iu..at + 38 + iu + n] == b"readme.txt" {
            break;
        }
        at += (38 + iu + n + 3) & !3;
    }
    let tag = hadris_udf::raw::Tag::read(&good[at..]).unwrap();
    let crc_length = usize::from(tag.crc_length.get());
    let iu = usize::from(u16::from_le_bytes(
        good[at + 36..at + 38].try_into().unwrap(),
    ));
    let n = usize::from(good[at + 19]);
    let padding = 38 + iu + n;
    assert!(padding % 4 != 0);
    for field in [
        "file-version",
        "reserved-characteristics",
        "padding",
        "tag-location",
        "tag-reserved-byte",
        "implementation-use-alignment",
        "parent-name",
    ] {
        let mut bad = good.clone();
        match field {
            "file-version" => bad[at + 16..at + 18].fill(0),
            "reserved-characteristics" => bad[at + 18] |= 0x80,
            "padding" => bad[at + padding] = 0xFF,
            "implementation-use-alignment" => {
                bad[at + 36..at + 38].copy_from_slice(&1u16.to_le_bytes())
            }
            "parent-name" => bad[at + 18] |= 8,
            _ => (),
        }
        let location = tag.location.get() + u32::from(field == "tag-location");
        hadris_udf::raw::Tag::seal(
            &mut bad[at..],
            tag.identifier.get(),
            tag.version.get(),
            location,
            crc_length,
        );
        if field == "tag-reserved-byte" {
            bad[at + 5] = 1;
            bad[at + 4] = (0..16)
                .filter(|i| *i != 4)
                .fold(0u8, |sum, i| sum.wrapping_add(bad[at + i]));
        }
        let mut fs = open(bad.clone());
        let root = fs.root();
        let err = fs
            .lookup(root, hadris_fs::Name::new("readme.txt"))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt, "{field}");
        assert_eq!(
            fs.names("/").unwrap_err().kind(),
            ErrorKind::Corrupt,
            "{field}"
        );
        #[cfg(feature = "async")]
        common::block_on(async {
            use hadris_fs::r#async::FileSystem;
            let mut fs =
                hadris_udf::r#async::UdfFs::mount(MemDevice::new(bad, SECTOR), MountOptions::new())
                    .await
                    .unwrap();
            let root = fs.root();
            assert_eq!(
                fs.lookup(root, hadris_fs::Name::new("readme.txt"))
                    .await
                    .unwrap_err()
                    .kind(),
                ErrorKind::Corrupt,
                "{field}"
            );
        });
    }
}
