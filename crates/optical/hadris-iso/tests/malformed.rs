mod common;

use hadris_fs::NodeId;
use hadris_iso::raw::{DirectoryRecord, FileFlags, U16Both, U32Both};

fn record(name: &[u8], area: &[u8], block: u32, len: u32, directory: bool) -> DirectoryRecord {
    let mut record = DirectoryRecord::new(name, area).unwrap();
    let header = record.header_mut();
    header.extent = U32Both::new(block);
    header.data_len = U32Both::new(len);
    header.volume_sequence_number = U16Both::new(1);
    header.flags = if directory {
        FileFlags::DIRECTORY.bits()
    } else {
        0
    };
    record
}

fn image(area: &[u8], size: u32) -> (Vec<u8>, NodeId) {
    let mut bytes = vec![0; 96 * 2048];
    let root = record(&[0], &[], 20, size, true);
    let pvd = &mut bytes[16 * 2048..17 * 2048];
    pvd[..7].copy_from_slice(b"\x01CD001\x01");
    pvd[80..88].copy_from_slice(bytemuck::bytes_of(&U32Both::new(96)));
    for at in [120, 124] {
        pvd[at..at + 4].copy_from_slice(bytemuck::bytes_of(&U16Both::new(1)));
    }
    pvd[128..132].copy_from_slice(bytemuck::bytes_of(&U16Both::new(2048)));
    pvd[156..190].copy_from_slice(root.as_bytes());
    bytes[17 * 2048..17 * 2048 + 7].copy_from_slice(b"\xffCD001\x01");
    let dot = record(
        &[0],
        b"SP\x07\x01\xbe\xef\x00RR\x05\x01\x01",
        20,
        size,
        true,
    );
    let parent = record(&[1], &[], 20, size, true);
    let mut at = 20 * 2048;
    for entry in [dot, parent] {
        bytes[at..at + entry.len()].copy_from_slice(entry.as_bytes());
        at += entry.len();
    }
    let file = record(b"FILE;1", area, 90, 1, false);
    bytes[at..at + file.len()].copy_from_slice(file.as_bytes());
    (bytes, NodeId::new(at as u64).unwrap())
}

fn ce(block: u32, offset: u32, len: u32) -> Vec<u8> {
    let mut out = b"CE\x1c\x01".to_vec();
    for value in [block, offset, len] {
        out.extend_from_slice(bytemuck::bytes_of(&U32Both::new(value)));
    }
    out
}

fn chain(count: usize, cycle: bool) -> (Vec<u8>, NodeId) {
    let (mut bytes, node) = image(&ce(30, 0, 28), 2048);
    for i in 0..count {
        let area = if i + 1 < count || cycle {
            ce(if cycle { 30 } else { 31 + i as u32 }, 0, 28)
        } else {
            let mut area = b"NM\x09\x01\x00tailST\x04\x01".to_vec();
            area.resize(28, 0);
            area
        };
        let at = (30 + i) * 2048;
        bytes[at..at + area.len()].copy_from_slice(&area);
    }
    (bytes, node)
}

macro_rules! regressions {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{DirCursor, ErrorKind, MountOptions};
                use hadris_iso::Detail;
                use hadris_iso::$mode::IsoFs;
                use hadris_storage::MemDevice;

                for area in [
                    &b"PX\x04\x01"[..],
                    &b"TF\x04\x01"[..],
                    &b"NM\x04\x01"[..],
                    &b"SL\x04\x01"[..],
                    &b"ER\x08\x01\x08\x00\x00\x01"[..],
                    &b"SL\x06\x01\x00\x00"[..],
                    &b"PX\x03\x01"[..],
                    &b"PX\x09\x01"[..],
                    &b"NM\x05\x02\x00"[..],
                ] {
                    let (bytes, node) = image(area, 2048);
                    let mut fs =
                        IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                            .await
                            .unwrap();
                    let err = fs.stat(node).await.unwrap_err();
                    assert_eq!(err.kind(), ErrorKind::Corrupt, "{area:?}");
                    assert_eq!(Detail::of(&err), Some(Detail::SystemUse));
                }
                for (count, cycle, valid) in [
                    (1, false, true),
                    (16, false, true),
                    (17, false, false),
                    (1, true, false),
                ] {
                    let (bytes, node) = chain(count, cycle);
                    let mut fs =
                        IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                            .await
                            .unwrap();
                    let result = fs.stat(node).await;
                    if valid {
                        result.unwrap();
                        let entry = fs
                            .readdir(fs.root(), DirCursor::START)
                            .await
                            .unwrap()
                            .unwrap();
                        assert_eq!(entry.name().as_bytes(), b"tail");
                    } else {
                        let err = result.unwrap_err();
                        assert_eq!(err.kind(), ErrorKind::Corrupt);
                        assert_eq!(Detail::of(&err), Some(Detail::SystemUse));
                    }
                }
                for mut area in [
                    ce(30, 0, 0),
                    ce(30, 2048, 4),
                    ce(95, 2047, 4),
                    ce(u32::MAX, 0, 4),
                    ce(30, 0, 28),
                ] {
                    if area == ce(30, 0, 28) {
                        area[8] ^= 1;
                    }
                    let (bytes, node) = image(&area, 2048);
                    let mut fs =
                        IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                            .await
                            .unwrap();
                    let err = fs.stat(node).await.unwrap_err();
                    assert_eq!(err.kind(), ErrorKind::Corrupt);
                    assert_eq!(Detail::of(&err), Some(Detail::SystemUse));
                }
                let mut area = b"ZZ\x06\x7fxx".to_vec();
                for _ in 0..8 {
                    let mut unknown = vec![0; 255];
                    unknown[..4].copy_from_slice(b"ZZ\xff\x7f");
                    area.extend(unknown);
                }
                area.extend_from_slice(b"NM\x0e\x01\x00continuedST\x04\x01");
                let (mut bytes, node) = image(&ce(30, 2040, area.len() as u32), 2048);
                let at = 30 * 2048 + 2040;
                bytes[at..at + area.len()].copy_from_slice(&area);
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                fs.stat(node).await.unwrap();
                let entry = fs
                    .readdir(fs.root(), DirCursor::START)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(entry.name().as_bytes(), b"continued");

                let mut area = ce(33, 0, 13);
                for _ in 0..8 {
                    let mut unknown = vec![0; 255];
                    unknown[..4].copy_from_slice(b"ZZ\xff\x7f");
                    area.extend(unknown);
                }
                let (mut bytes, _) = image(&ce(30, 0, area.len() as u32), 2048);
                bytes[30 * 2048..30 * 2048 + area.len()].copy_from_slice(&area);
                bytes[33 * 2048..33 * 2048 + 13].copy_from_slice(b"NM\x09\x01\x00tailST\x04\x01");
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                let entry = fs
                    .readdir(fs.root(), DirCursor::START)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(entry.name().as_bytes(), b"tail");

                for field in [4, 12, 20] {
                    let mut area = ce(30, 0, 28);
                    area[field + 4] ^= 1;
                    let (bytes, node) = image(&area, 2048);
                    let mut fs =
                        IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                            .await
                            .unwrap();
                    assert_eq!(
                        Detail::of(&fs.stat(node).await.unwrap_err()),
                        Some(Detail::SystemUse)
                    );
                }
                let mut long_name = vec![b'a'; 255];
                long_name[..5].copy_from_slice(b"NM\xff\x01\x01");
                long_name = long_name.repeat(4);
                long_name.extend_from_slice(b"NM\x0b\x01\x00aaaaaa");
                let (mut bytes, _) = image(&ce(30, 0, long_name.len() as u32), 2048);
                bytes[30 * 2048..30 * 2048 + long_name.len()].copy_from_slice(&long_name);
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                let entry = fs
                    .readdir(fs.root(), DirCursor::START)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(entry.name().as_bytes(), b"FILE");

                let (mut bytes, _) = image(&[], 2048);
                let dot = record(&[0], b"SP ordinary!", 20, 2048, true);
                bytes[20 * 2048..20 * 2048 + dot.len()].copy_from_slice(dot.as_bytes());
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                fs.stat(fs.root()).await.unwrap();
                fs.lookup(fs.root(), hadris_fs::Name::new(b"FILE"))
                    .await
                    .unwrap();

                let (_, node) = image(&[], 2048);
                let size = (node.get() - 20 * 2048) as u32 + 1;
                let (bytes, _) = image(&[], size);
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                let err = fs.readdir(fs.root(), DirCursor::START).await.unwrap_err();
                assert_eq!(err.kind(), ErrorKind::Corrupt);
                assert_eq!(Detail::of(&err), Some(Detail::DirectoryRecord));
                let (bytes, _) = image(&[], size + 39);
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                assert!(
                    fs.readdir(fs.root(), DirCursor::START)
                        .await
                        .unwrap()
                        .is_some()
                );

                let (mut bytes, _) = image(&[], 4096);
                let file = record(b"FILE;1", &[], 90, 1, false);
                bytes[20 * 2048 + 2047] = file.len() as u8;
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                let err = fs
                    .readdir(fs.root(), DirCursor::from_raw(2047))
                    .await
                    .unwrap_err();
                assert_eq!(Detail::of(&err), Some(Detail::DirectoryRecord));
            });
        }
    };
}

macro_rules! sync_case {
    ($($body:tt)*) => { common::block_on(hadris_macros::strip_async! { $($body)* }) };
}
#[cfg(feature = "async")]
macro_rules! async_case {
    ($body:expr) => {
        common::block_on($body)
    };
}
regressions!(sync, malformed_records_sync, sync_case);
#[cfg(feature = "async")]
regressions!(r#async, malformed_records_async, async_case);

macro_rules! timestamp_regressions {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{DateTime, DirCursor, MountOptions};
                use hadris_iso::raw::DirDateTime;
                use hadris_iso::$mode::IsoFs;
                use hadris_storage::MemDevice;

                let recorded = DateTime::from_unix_seconds(1_600_000_000)
                    .unwrap()
                    .with_utc_offset_minutes(Some(0))
                    .unwrap();
                let extended = DateTime::from_unix_seconds(1_700_000_000)
                    .unwrap()
                    .with_utc_offset_minutes(Some(0))
                    .unwrap();
                for flag in [1u8, 2, 4, 8] {
                    let mut area = b"TF\x0c\x01".to_vec();
                    area.push(flag);
                    area.extend_from_slice(bytemuck::bytes_of(&DirDateTime::from_datetime(
                        extended,
                    )));
                    let (mut bytes, node) = image(&area, 2048);
                    let at = node.get() as usize + 18;
                    bytes[at..at + 7]
                        .copy_from_slice(bytemuck::bytes_of(&DirDateTime::from_datetime(recorded)));
                    let mut fs =
                        IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                            .await
                            .unwrap();
                    let meta = fs.stat(node).await.unwrap();
                    assert_eq!(
                        meta.modified(),
                        Some(if flag == 2 { extended } else { recorded }),
                        "TF flags {flag}"
                    );
                    assert_eq!(meta.created(), (flag == 1).then_some(extended));
                    assert_eq!(meta.accessed(), (flag == 4).then_some(extended));
                    assert_eq!(meta.changed(), (flag == 8).then_some(extended));
                    let entry = fs
                        .readdir(fs.root(), DirCursor::START)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(entry.metadata(), &meta);
                }
            });
        }
    };
}

timestamp_regressions!(sync, partial_timestamps_sync, sync_case);
#[cfg(feature = "async")]
timestamp_regressions!(r#async, partial_timestamps_async, async_case);
