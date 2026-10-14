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
    image_of(area, size, 96)
}

fn image_of(area: &[u8], size: u32, blocks: u32) -> (Vec<u8>, NodeId) {
    let mut bytes = vec![0; blocks as usize * 2048];
    let root = record(&[0], &[], 20, size, true);
    let pvd = &mut bytes[16 * 2048..17 * 2048];
    pvd[..7].copy_from_slice(b"\x01CD001\x01");
    pvd[80..88].copy_from_slice(bytemuck::bytes_of(&U32Both::new(blocks)));
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

fn px(mode: u32, links: u32, serial: u32) -> Vec<u8> {
    let mut out = b"PX\x2c\x01".to_vec();
    for value in [mode, links, 0, 0, serial] {
        out.extend_from_slice(bytemuck::bytes_of(&U32Both::new(value)));
    }
    out
}

fn nm(name: &[u8]) -> Vec<u8> {
    let mut out = vec![b'N', b'M', 5 + name.len() as u8, 1, 0];
    out.extend_from_slice(name);
    out
}

/// A Rock Ridge root holding `entries` (identifier, system use, data
/// length), each with `hello` as data, and a path table.
fn files(entries: &[(&[u8], Vec<u8>, u32)]) -> (Vec<u8>, Vec<NodeId>) {
    let (mut bytes, first) = image(&[], 2048);
    let pvd = 16 * 2048;
    bytes[pvd + 132..pvd + 140].copy_from_slice(bytemuck::bytes_of(&U32Both::new(10)));
    bytes[pvd + 140..pvd + 144].copy_from_slice(&19u32.to_le_bytes());
    bytes[19 * 2048..19 * 2048 + 10].copy_from_slice(b"\x01\x00\x14\x00\x00\x00\x01\x00\x00\x00");
    let mut at = first.get() as usize;
    bytes[at..20 * 2048 + 2048].fill(0);
    let mut nodes = Vec::new();
    for (index, (name, area, len)) in entries.iter().enumerate() {
        let block = 90 + index as u32;
        let file = record(name, area, block, *len, false);
        bytes[at..at + file.len()].copy_from_slice(file.as_bytes());
        bytes[block as usize * 2048..block as usize * 2048 + 5].copy_from_slice(b"hello");
        nodes.push(NodeId::new(at as u64).unwrap());
        at += file.len();
    }
    (bytes, nodes)
}

/// A symlink record whose target, 30 components of 200 bytes, is split
/// over `SL` entries in block-sized continuation areas, and the target.
fn long_symlink() -> (Vec<u8>, NodeId, Vec<u8>) {
    let components: Vec<Vec<u8>> = (0..30).map(|i| vec![b'a' + i as u8 % 26; 200]).collect();
    let mut areas: Vec<Vec<u8>> = Vec::new();
    for (index, component) in components.iter().enumerate() {
        let last = index + 1 == components.len();
        let mut entry = vec![b'S', b'L', 207, 1, u8::from(!last), 0, 200];
        entry.extend_from_slice(component);
        if index % 9 == 0 {
            areas.push(Vec::new());
        }
        areas.last_mut().unwrap().extend(entry);
    }
    let count = areas.len();
    let len = |index: usize, areas: &[Vec<u8>]| {
        areas[index].len() as u32 + if index + 1 < count { 28 } else { 0 }
    };
    let mut su = nm(b"link");
    su.extend(px(0o120_777, 1, 0));
    su.extend(ce(30, 0, len(0, &areas)));
    let (mut bytes, nodes) = files(&[(b"LINK;1", su, 0)]);
    for index in 0..count {
        let mut area = areas[index].clone();
        if index + 1 < count {
            area.extend(ce(31 + index as u32, 0, len(index + 1, &areas)));
        }
        let at = (30 + index) * 2048;
        bytes[at..at + area.len()].copy_from_slice(&area);
    }
    (bytes, nodes[0], components.join(&b'/'))
}

fn chain(count: usize, cycle: bool) -> (Vec<u8>, NodeId) {
    let (mut bytes, node) = image_of(&ce(30, 0, 28), 2048, 96.max(31 + count as u32));
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
                #[allow(unused_imports)]
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
                    (31, false, true),
                    (256, false, true),
                    (257, false, false),
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

                let mut area = Vec::new();
                while area.len() < 1 << 20 {
                    let mut unknown = vec![0; 255];
                    unknown[..4].copy_from_slice(b"ZZ\xff\x7f");
                    area.extend(unknown);
                }
                area.extend(nm(b"big"));
                let (mut bytes, node) = image_of(&ce(30, 8, area.len() as u32), 2048, 600);
                bytes[30 * 2048 + 8..30 * 2048 + 8 + area.len()].copy_from_slice(&area);
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
                assert_eq!(entry.name().as_bytes(), b"big");

                let blocks = 2600u32;
                for len in [(4 << 20) + 1, (blocks - 30) * 2048] {
                    let (bytes, node) = image_of(&ce(30, 0, len), 2048, blocks);
                    let mut fs =
                        IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                            .await
                            .unwrap();
                    let err = fs.stat(node).await.unwrap_err();
                    assert_eq!(err.kind(), ErrorKind::Corrupt, "{len}");
                    assert_eq!(Detail::of(&err), Some(Detail::SystemUse));
                }
                let (bytes, node) = image_of(&ce(30, 0, 4 << 20), 2048, blocks);
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                fs.stat(node).await.unwrap();
                let (mut bytes, node) = image_of(&ce(30, 0, 3 << 20), 2048, 3600);
                bytes[30 * 2048..30 * 2048 + 28].copy_from_slice(&ce(2000, 0, 3 << 20));
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                let err = fs.stat(node).await.unwrap_err();
                assert_eq!(Detail::of(&err), Some(Detail::SystemUse));

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

macro_rules! damaged_regressions {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                #[allow(unused_imports)]
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{DirCursor, ErrorKind, FileType, Metadata, MountOptions, Name};
                use hadris_fs::{OpenMode, Permissions};
                use hadris_iso::Detail;
                use hadris_iso::$mode::IsoFs;
                use hadris_storage::MemDevice;

                for bad in [b"PX\x04\x01".to_vec(), ce(u32::MAX, 0, 4), ce(30, 0, 0)] {
                    let mut good = nm(b"good");
                    good.extend(px(0o100_644, 2, 7));
                    let (bytes, nodes) =
                        files(&[(b"FILE;1", bad.clone(), 5), (b"GOOD;1", good, 5)]);
                    let mut fs =
                        IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                            .await
                            .unwrap();
                    let first = fs
                        .readdir(fs.root(), DirCursor::START)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(first.name().as_bytes(), b"FILE", "{bad:?}");
                    assert_eq!(first.node(), nodes[0]);
                    assert_eq!(
                        first.metadata(),
                        &Metadata::new(FileType::File, Permissions::new(0))
                    );
                    let err = fs.stat(first.node()).await.unwrap_err();
                    assert_eq!(err.kind(), ErrorKind::Corrupt);
                    assert_eq!(Detail::of(&err), Some(Detail::SystemUse));
                    let err = fs.open(first.node(), OpenMode::Read).await.unwrap_err();
                    assert_eq!(err.kind(), ErrorKind::Corrupt);
                    let mut buf = [0u8; 64];
                    let err = fs.readlink(first.node(), &mut buf).await.unwrap_err();
                    assert_eq!(err.kind(), ErrorKind::Corrupt);

                    let second = fs
                        .readdir(fs.root(), first.next_cursor())
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(second.name().as_bytes(), b"good");
                    assert_eq!(second.node(), nodes[1]);
                    assert_eq!(second.metadata().permissions(), Permissions::new(0o100_644));
                    assert_eq!(second.metadata().nlink(), 2);
                    assert!(
                        fs.readdir(fs.root(), second.next_cursor())
                            .await
                            .unwrap()
                            .is_none()
                    );
                    let node = fs.lookup(fs.root(), Name::new(b"good")).await.unwrap();
                    assert_eq!(node, nodes[1]);
                    fs.open(node, OpenMode::Read).await.unwrap();
                    assert_eq!(fs.read(node, 0, &mut buf).await.unwrap(), 5);
                    assert_eq!(&buf[..5], b"hello");
                    assert_eq!(fs.stat(node).await.unwrap().len(), 5);
                    let node = fs.lookup(fs.root(), Name::new(b"FILE")).await.unwrap();
                    assert_eq!(node, nodes[0]);
                }

                let (bytes, node, target) = long_symlink();
                assert!(target.len() > 6000);
                let mut fs =
                    IsoFs::mount(MemDevice::new(bytes, common::SECTOR), MountOptions::new())
                        .await
                        .unwrap();
                let entry = fs
                    .readdir(fs.root(), DirCursor::START)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(entry.name().as_bytes(), b"link");
                assert_eq!(entry.file_type(), FileType::Symlink);
                assert_eq!(entry.metadata().len(), target.len() as u64);
                assert_eq!(fs.stat(node).await.unwrap().len(), target.len() as u64);
                let mut buf = vec![0u8; 8192];
                assert_eq!(fs.readlink(node, &mut buf).await.unwrap(), &target[..]);
                let mut short = vec![0u8; target.len() - 1];
                let err = fs.readlink(node, &mut short).await.unwrap_err();
                assert_eq!(err.kind(), ErrorKind::LimitExceeded);
            });
        }
    };
}

damaged_regressions!(sync, damaged_entries_list_sync, sync_case);
#[cfg(feature = "async")]
damaged_regressions!(r#async, damaged_entries_list_async, async_case);

macro_rules! timestamp_regressions {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                #[allow(unused_imports)]
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
