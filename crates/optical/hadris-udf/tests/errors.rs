//! Damaged volumes and options the writer cannot honour fail with the
//! documented kinds, without panicking.

mod common;

use common::Paths;
use common::{SECTOR, image, open, reseal, sample};
use hadris_fs::sync::FileSystem;
use hadris_fs::tree::{Content, Tree};
use hadris_fs::{ErrorKind, Extent, FileTimes, NodeId, SetMetadata};
use hadris_storage::{BlockSize, MemDevice};
use hadris_udf::sync::UdfFs;
use hadris_udf::{Bridge, Detail, UdfOptions, UdfRevision};

fn good() -> Vec<u8> {
    image(&sample(), &UdfOptions::default())
}

fn open_err(bytes: Vec<u8>) -> ErrorKind {
    UdfFs::open(MemDevice::new(bytes, SECTOR))
        .unwrap_err()
        .kind()
}

#[test]
fn malformed_volumes_are_refused() {
    let good = good();
    assert_eq!(open_err(vec![0u8; good.len()]), ErrorKind::Corrupt);

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
    if let Ok(mut udf) = UdfFs::open(MemDevice::new(truncated, SECTOR)) {
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

    let mut udf = UdfFs::open(MemDevice::new(good, BlockSize::new(4096).unwrap())).unwrap();
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
    assert_eq!(udf.volume_id(), "UDF_VOLUME");
    assert_eq!(udf.read_to_vec("/docs/sub/deep.txt").unwrap(), b"deep");
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
    let err = hadris_udf::sync::plan(
        &tree,
        &UdfOptions::default().with_volume_id("x".repeat(127)),
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
        let err = hadris_udf::sync::plan(&tree, &options).unwrap_err();
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
    long.add_file(&"n".repeat(255), Content::empty()).unwrap();
    let err = hadris_udf::sync::plan(&long, &UdfOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NameTooLong);

    let mut late = Tree::new();
    late.add_file("f", Content::empty()).unwrap();
    let far = hadris_fs::DateTime::from_unix_seconds(253_402_300_800).unwrap();
    late.set_metadata(
        "f",
        SetMetadata::new().with_times(FileTimes::new().with_modified(far)),
    )
    .unwrap();
    let err = hadris_udf::sync::plan(&late, &UdfOptions::default()).unwrap_err();
    assert_eq!(
        err.detail().and_then(hadris_udf::Detail::from_code),
        Some(Detail::Timestamp)
    );

    let mut stored = Tree::new();
    stored
        .add_file("f", Content::stored([Extent::new(2048 * 400, 10)]))
        .unwrap();
    let err = hadris_udf::sync::plan(&stored, &UdfOptions::default()).unwrap_err();
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

#[test]
fn bridge_volumes_take_only_stored_content() {
    let bridge = UdfOptions::default().with_bridge(Bridge::new(3));
    let mut dev = MemDevice::new(vec![0u8; 4 << 20], SECTOR);

    let mut bytes = Tree::new();
    bytes.add_file("f", Content::bytes("data")).unwrap();
    let err = hadris_udf::sync::write(&mut dev, &bytes, &bridge).unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.detail().and_then(hadris_udf::Detail::from_code)
        ),
        (ErrorKind::Unsupported, Some(Detail::StoredContent))
    );

    let mut unaligned = Tree::new();
    unaligned
        .add_file("f", Content::stored([Extent::new(2048 * 400 + 1, 10)]))
        .unwrap();
    let err = hadris_udf::sync::write(&mut dev, &unaligned, &bridge).unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.detail().and_then(hadris_udf::Detail::from_code)
        ),
        (ErrorKind::InvalidInput, Some(Detail::StoredContent))
    );

    let mut early = Tree::new();
    early
        .add_file("f", Content::stored([Extent::new(2048 * 291, 10)]))
        .unwrap();
    let err = hadris_udf::sync::write(&mut dev, &early, &bridge).unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.detail().and_then(hadris_udf::Detail::from_code)
        ),
        (ErrorKind::InvalidInput, Some(Detail::StoredContent))
    );
    assert!(dev.get_ref().iter().all(|&b| b == 0));
}
