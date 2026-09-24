//! Crafted images: the driver contract, every mode, and malformed
//! structures that must fail with an error, never a panic.

use hadris_fs::Error;
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, ErrorKind, FileType, Name, NodeId};
use hadris_ntfs::Detail;
use hadris_ntfs::raw;
use hadris_ntfs::sync::NtfsFs;
use hadris_storage::{BlockSize, MemDevice};

#[path = "support/image.rs"]
mod image;
use image::*;
#[path = "support/paths.rs"]
mod paths;
use paths::PathOps;

fn device(image: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(image, BlockSize::new(512).unwrap())
}

fn open(image: Vec<u8>) -> NtfsFs<MemDevice<Vec<u8>>> {
    NtfsFs::open(device(image)).expect("crafted image must mount")
}

fn open_err(image: Vec<u8>) -> Error<core::convert::Infallible> {
    match NtfsFs::open(device(image)) {
        Ok(_) => panic!("the image mounted"),
        Err(err) => err.into(),
    }
}
fn list(fs: &mut NtfsFs<MemDevice<Vec<u8>>>, path: &str) -> Vec<String> {
    fs.entries(path)
        .unwrap()
        .iter()
        .map(|entry| entry.name().to_str().unwrap().to_string())
        .collect()
}

fn name(s: &str) -> &Name {
    Name::new(s)
}

#[test]
fn crafted_image_mounts_and_walks() {
    let mut fs = open(base_image());
    assert_eq!(list(&mut fs, "/"), ["HELLO.TXT", "SUBDIR", "BIN.DAT"]);
    assert_eq!(fs.read_to_vec("/HELLO.TXT").unwrap(), b"hello ntfs");
    assert_eq!(fs.read_to_vec("/BIN.DAT").unwrap(), b"bin");
    assert!(list(&mut fs, "/SUBDIR").is_empty());
    assert_eq!(fs.metadata("/SUBDIR").unwrap().file_type(), FileType::Dir);
    assert_eq!(fs.volume_serial(), 0x1122_3344_5566_7788);
    assert_eq!(fs.cluster_size(), 512);
    assert_eq!(fs.mft_record_size(), 1024);
    let mut label = [0u8; 64];
    assert_eq!(fs.label(&mut label).unwrap(), Some("HADRIS"));
}

#[test]
fn contract_holds_in_every_mode_and_tier() {
    let image = base_image();
    let mut fs = open(image.clone());
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
    let vol = hadris_fs::sync::Volume::new(open(image.clone()));
    hadris_fs::sync::contract::check_read_only(&mut *vol.lock()).unwrap();
    let mut fs = open(allocation_image(1024, &[0x01], None));
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();

    block_on(async {
        let mut fs = hadris_ntfs::r#async::NtfsFs::open(device(image.clone()))
            .await
            .unwrap();
        hadris_fs::r#async::contract::check_read_only(&mut fs)
            .await
            .unwrap();
        let fs = hadris_ntfs::r#async::NtfsFs::open(device(image))
            .await
            .unwrap();
        let vol = hadris_fs::r#async::Volume::new(fs);
        hadris_fs::r#async::contract::check_read_only(&mut *vol.lock().await)
            .await
            .unwrap();
    });
}

#[test]
fn metadata_files_and_dos_aliases_are_hidden_but_found() {
    let mut fs = open(base_image());
    let root = fs.root();
    assert!(
        !list(&mut fs, "/")
            .iter()
            .any(|n| n.starts_with('$') || n == ".")
    );
    let mft = fs.lookup(root, name("$MFT")).unwrap();
    assert_eq!(mft.get() & 0xFFFF_FFFF_FFFF, raw::RECORD_MFT);
    let alias = fs.lookup(root, name("hello~1.txt")).unwrap();
    assert_eq!(alias, fs.lookup(root, name("HELLO.TXT")).unwrap());
}

#[test]
fn names_fold_case_through_upcase() {
    let mut fs = open(base_image());
    let root = fs.root();
    let hello = fs.lookup(root, name("HELLO.TXT")).unwrap();
    assert_eq!(fs.lookup(root, name("hello.txt")).unwrap(), hello);
    assert_eq!(fs.lookup(root, name("Hello.Txt")).unwrap(), hello);
    assert!(fs.lookup(root, name("BIN.DAT")).is_ok());
    let posix = fs.lookup(root, name("bin.dat")).unwrap_err();
    assert_eq!(posix.kind(), ErrorKind::NotFound);
    assert_eq!(
        fs.lookup(root, name("HELLO.TX")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn an_unreadable_upcase_page_fails_only_folded_lookups() {
    let mut image = base_image();
    let runs = [
        0x11,
        0x01,
        UPCASE_LCN as u8,
        0x31,
        0x01,
        0xFF,
        0xFF,
        0x0F,
        0x02,
        0xFE,
        0x00,
        0x00,
    ];
    let upcase = non_resident(raw::ATTR_DATA, &[], 255, 131_072, 131_072, &runs);
    put(
        &mut image,
        raw::RECORD_UPCASE,
        &file_record(FILE, &[upcase]),
    );
    let mut entries = index_entry(20, "\u{100}BC.TXT", false, raw::FILE_NAME_WIN32);
    entries.extend(index_entry(16, "HELLO.TXT", false, raw::FILE_NAME_WIN32));
    entries.extend(last_entry());
    put(
        &mut image,
        raw::RECORD_ROOT,
        &file_record(DIR, &[named(5, ".", true), index_root(&entries, 1024)]),
    );
    let mut fs = open(image);
    let root = fs.root();
    assert!(fs.lookup(root, name("HELLO.TXT")).is_ok());
    assert_eq!(
        fs.lookup(root, name("hello.txt")).unwrap_err().kind(),
        ErrorKind::Corrupt
    );
}

#[test]
fn metadata_reports_times_attributes_and_links() {
    let mut fs = open(base_image());
    let meta = fs.metadata("/HELLO.TXT").unwrap();
    assert_eq!(meta.file_type(), FileType::File);
    assert_eq!(meta.len(), 10);
    assert_eq!(meta.nlink(), 1);
    let times = meta;
    assert_eq!(times.created().unwrap().unix_seconds(), 1_577_836_800);
    assert_eq!(times.modified().unwrap().unix_seconds(), 1_577_836_801);
    assert_eq!(times.changed().unwrap().unix_seconds(), 1_577_836_802);
    assert_eq!(times.accessed().unwrap().unix_seconds(), 1_577_836_803);
    assert_eq!(
        meta.attributes(),
        hadris_fs::Attributes::READ_ONLY | hadris_fs::Attributes::ARCHIVE
    );
    let root = fs.root();
    let sub = fs.lookup(root, name("SUBDIR")).unwrap();
    assert_eq!(fs.parent(sub).unwrap(), root);
    assert_eq!(fs.parent(root).unwrap(), root);
}

#[test]
fn named_streams_are_listed_and_read() {
    let mut fs = open(base_image());
    let root = fs.root();
    let hello = fs.lookup(root, name("HELLO.TXT")).unwrap();
    let mut streams = Vec::new();
    fs.streams(hello, |name, len| streams.push((name.to_string(), len)))
        .unwrap();
    assert_eq!(streams, [("extra".to_string(), 9)]);
    let mut buf = [0u8; 16];
    let n = fs.read_stream_at(hello, "EXTRA", 5, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"data");
    assert_eq!(
        fs.read_stream_at(hello, "missing", 0, &mut buf)
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn stats_count_free_clusters() {
    let mut fs = open(base_image());
    let stats = fs.statfs().unwrap();
    assert_eq!(stats.total_blocks(), (IMAGE_LEN / SECTOR) as u64);
    assert_eq!(stats.free_blocks(), (IMAGE_LEN / SECTOR) as u64 - 160);
    assert_eq!(stats.block_size(), 512);
}

#[test]
fn index_blocks_are_listed_through_the_bitmap() {
    let mut fs = open(allocation_image(1024, &[0x01], None));
    assert_eq!(list(&mut fs, "/SUBDIR"), ["A.TXT", "B.TXT"]);
    assert_eq!(fs.read_to_vec("/SUBDIR/b.txt").unwrap(), b"bb");
    assert_eq!(
        fs.metadata("/SUBDIR/UNUSED.TXT").unwrap_err().kind(),
        ErrorKind::NotFound
    );

    let mut fs = open(allocation_image(1024, &[0x03], None));
    assert_eq!(list(&mut fs, "/SUBDIR"), ["A.TXT", "B.TXT", "UNUSED.TXT"]);
}

#[test]
fn cursors_resume_across_index_blocks() {
    let mut fs = open(allocation_image(1024, &[0x03], None));
    let root = fs.root();
    let sub = fs.lookup(root, name("SUBDIR")).unwrap();
    let mut cursor = DirCursor::START;
    let mut stored = Vec::new();
    while let Some(entry) = fs.readdir(sub, cursor).unwrap() {
        cursor = entry.next_cursor();
        stored.push((cursor, entry.node()));
    }
    assert_eq!(stored.len(), 3);
    let resumed = stored[1].0;
    let next = fs.readdir(sub, resumed).unwrap().unwrap();
    assert_eq!(next.node(), stored[2].1);
    assert_eq!(next.name().as_bytes(), b"UNUSED.TXT");
    let end = next.next_cursor();
    assert!(fs.readdir(sub, end).unwrap().is_none());
    assert!(fs.readdir(sub, end).unwrap().is_none());
    assert!(end.into_raw() <= DirCursor::MAX_RAW);
}

#[test]
fn open_rejects_bad_boot_sectors() {
    type Edit = fn(&mut Vec<u8>);
    let cases: [(Edit, ErrorKind); 8] = [
        (|i| i[3] = b'X', ErrorKind::NotRecognized),
        (|i| i[510] = 0, ErrorKind::NotRecognized),
        (
            |i| i[11..13].copy_from_slice(&1000u16.to_le_bytes()),
            ErrorKind::Corrupt,
        ),
        (|i| i[13] = 3, ErrorKind::Corrupt),
        (
            |i| i[48..56].copy_from_slice(&4096u64.to_le_bytes()),
            ErrorKind::Corrupt,
        ),
        (|i| i[64] = 0, ErrorKind::Corrupt),
        (|i| i[64] = (-13i8) as u8, ErrorKind::Unsupported),
        (
            |i| {
                i[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
                i[48..56].copy_from_slice(&(1u64 << 60).to_le_bytes());
            },
            ErrorKind::Corrupt,
        ),
    ];
    for (edit, kind) in cases {
        let mut image = base_image();
        edit(&mut image);
        assert_eq!(open_err(image).kind(), kind);
    }
    let err = open_err(vec![0u8; 100]);
    assert_eq!(err.kind(), ErrorKind::NotRecognized);
    assert_eq!(Detail::of(&err), Some(Detail::BootSector));
    assert_eq!(open_err(boot_sector()).kind(), ErrorKind::Corrupt);
}

#[test]
fn open_refuses_blocks_above_4096_and_gives_the_device_back() {
    let dev = MemDevice::new(base_image(), BlockSize::new(8192).unwrap());
    let Err(err) = NtfsFs::open(dev) else {
        panic!("mounted");
    };
    let (err, dev) = err.into_parts();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(dev.into_inner().len(), IMAGE_LEN);
}

#[test]
fn open_needs_a_directory_root_and_an_upcase_table() {
    let mut image = base_image();
    put(&mut image, raw::RECORD_ROOT, &file_record(FILE, &[]));
    assert_eq!(open_err(image).kind(), ErrorKind::Corrupt);

    let mut image = base_image();
    let short = non_resident(
        raw::ATTR_DATA,
        &[],
        0,
        512,
        512,
        &[0x11, 0x01, UPCASE_LCN as u8, 0],
    );
    put(&mut image, raw::RECORD_UPCASE, &file_record(FILE, &[short]));
    assert_eq!(open_err(image).kind(), ErrorKind::Corrupt);

    let mut image = base_image();
    let sparse = non_resident(
        raw::ATTR_DATA,
        &[],
        255,
        131_072,
        0,
        &[0x02, 0x00, 0x01, 0x00],
    );
    put(
        &mut image,
        raw::RECORD_UPCASE,
        &file_record(FILE, &[sparse]),
    );
    let mut fs = open(image);
    let root = fs.root();
    assert!(fs.lookup(root, name("HELLO.TXT")).is_ok());
    assert!(fs.lookup(root, name("hello.txt")).is_err());
}

#[test]
fn huge_run_lengths_fail_without_overflow() {
    let mut image = base_image();
    let mut runs = vec![0x18u8];
    runs.extend(u64::MAX.to_le_bytes());
    runs.extend([0x01, 0x00]);
    let bin = non_resident(raw::ATTR_DATA, &[], u64::MAX - 1, 4096, 4096, &runs);
    put(
        &mut image,
        18,
        &file_record(FILE, &[named(5, "BIN.DAT", false), bin]),
    );
    let mut fs = open(image);
    let err = fs.read_to_vec("/BIN.DAT").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
}

#[test]
fn streams_past_the_volume_fail() {
    let mut image = base_image();
    let bin = non_resident(
        raw::ATTR_DATA,
        &[],
        0,
        512,
        512,
        &[0x31, 0x01, 0xFF, 0xFF, 0x7F, 0],
    );
    put(
        &mut image,
        18,
        &file_record(FILE, &[named(5, "BIN.DAT", false), bin]),
    );
    let mut fs = open(image);
    let err = fs.read_to_vec("/BIN.DAT").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
}

#[test]
fn uninitialized_tails_and_sparse_runs_read_as_zeros() {
    let mut image = base_image();
    let bin = non_resident(
        raw::ATTR_DATA,
        &[],
        (1 << 41) - 1,
        1 << 50,
        3,
        &[0x11, 0x01, BIN_LCN as u8, 0],
    );
    put(
        &mut image,
        18,
        &file_record(FILE, &[named(5, "BIN.DAT", false), bin]),
    );
    let mut fs = open(image);
    let root = fs.root();
    let bin = fs.lookup(root, name("BIN.DAT")).unwrap();
    let mut buf = [0xEEu8; 8];
    assert_eq!(fs.read(bin, 0, &mut buf).unwrap(), 8);
    assert_eq!(&buf, b"bin\0\0\0\0\0");
    assert_eq!(fs.read(bin, 1 << 40, &mut buf).unwrap(), 8);
    assert_eq!(buf, [0; 8]);
    assert_eq!(fs.stat(bin).unwrap().len(), 1 << 50);
}

#[test]
fn corrupt_index_blocks_fail() {
    let cases = [
        allocation_image(4, &[0x01], None),
        allocation_image(u32::MAX, &[0x01], None),
        allocation_image(1024, &[], None),
        allocation_image(
            1024,
            &[0x01],
            Some([
                {
                    let mut bad = indx(&last_entry());
                    bad[510] ^= 0xFF;
                    bad
                },
                indx(&last_entry()),
            ]),
        ),
        allocation_image(1024, &[0x01], Some([vec![0u8; REC], indx(&last_entry())])),
    ];
    for image in cases {
        let mut fs = open(image);
        let root = fs.root();
        let sub = fs.lookup(root, name("SUBDIR")).unwrap();
        let mut cursor = DirCursor::START;
        let mut failed = false;
        for _ in 0..8 {
            match fs.readdir(sub, cursor) {
                Ok(Some(entry)) => cursor = entry.next_cursor(),
                Ok(None) => break,
                Err(err) => {
                    assert!(
                        matches!(err.kind(), ErrorKind::Corrupt | ErrorKind::Unsupported),
                        "{err:?}"
                    );
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed);
    }
}

#[test]
fn forged_ids_are_invalid_handles() {
    let mut fs = open(base_image());
    for raw_id in [u64::MAX, 1 << 40, reference(16) + (1 << 48), reference(25)] {
        let err = fs.stat(NodeId::new(raw_id).unwrap()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidHandle, "{raw_id:#x}");
    }
    let root = fs.root();
    let hello = fs.lookup(root, name("HELLO.TXT")).unwrap();
    assert_eq!(
        fs.readdir(hello, DirCursor::START).unwrap_err().kind(),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fs.read(root, 0, &mut [0u8; 4]).unwrap_err().kind(),
        ErrorKind::IsADirectory
    );
}

#[test]
fn attribute_lists_join_extension_records() {
    let (image, content) = listed_image(2);
    let mut fs = open(image.clone());
    assert_eq!(fs.read_to_vec("/BIN.DAT").unwrap(), content);
    let meta = fs.metadata("/BIN.DAT").unwrap();
    assert_eq!((meta.len(), meta.nlink()), (2000, 1));
    let root = fs.root();
    let bin = fs.lookup(root, name("BIN.DAT")).unwrap();
    let mut buf = [0u8; 100];
    assert_eq!(fs.read(bin, 1000, &mut buf).unwrap(), 100);
    assert_eq!(&buf[..], &content[1000..1100]);
    let mut streams = Vec::new();
    fs.streams(bin, |name, len| streams.push((name.to_string(), len)))
        .unwrap();
    assert_eq!(streams, [("alt".to_string(), 9)]);
    let n = fs.read_stream_at(bin, "ALT", 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"alternate");
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
    block_on(async {
        let mut fs = hadris_ntfs::r#async::NtfsFs::open(device(image))
            .await
            .unwrap();
        assert_eq!(read_async(&mut fs, "/BIN.DAT").await, content);
    });
}

#[test]
fn attribute_list_gaps_fail() {
    let (image, content) = listed_image(3);
    let mut fs = open(image);
    let root = fs.root();
    let bin = fs.lookup(root, name("BIN.DAT")).unwrap();
    let mut buf = [0u8; 1024];
    assert_eq!(fs.read(bin, 0, &mut buf).unwrap(), 1024);
    assert_eq!(&buf[..], &content[..1024]);
    assert_eq!(
        fs.read(bin, 1024, &mut buf).unwrap_err().kind(),
        ErrorKind::Corrupt
    );
}

#[test]
fn fragmented_mft_is_followed() {
    let mut fs = open(fragmented_mft_image());
    assert_eq!(list(&mut fs, "/"), ["HELLO.TXT", "SUBDIR", "BIN.DAT"]);
    assert_eq!(fs.read_to_vec("/HELLO.TXT").unwrap(), b"hello ntfs");
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();
}

#[test]
fn index_roots_in_extension_records_are_followed() {
    let mut image = base_image();
    let mut entries_list = list_entry(raw::ATTR_FILE_NAME, &[], 0, 17, 1);
    entries_list.extend(list_entry(raw::ATTR_INDEX_ROOT, &raw::I30, 0, 24, 0));
    let sub = [
        with_id(named(5, "SUBDIR", true), 1),
        resident(raw::ATTR_ATTRIBUTE_LIST, &[], &entries_list),
    ];
    put(&mut image, 17, &file_record(DIR, &sub));
    let mut entries = index_entry(19, "INNER.TXT", false, raw::FILE_NAME_WIN32);
    entries.extend(last_entry());
    put(
        &mut image,
        24,
        &file_record(FILE, &[index_root(&entries, 1024)]),
    );
    let inner = [
        named(17, "INNER.TXT", false),
        resident(raw::ATTR_DATA, &[], b"inner"),
    ];
    put(&mut image, 19, &file_record(FILE, &inner));
    let mut fs = open(image.clone());
    assert_eq!(list(&mut fs, "/SUBDIR"), ["INNER.TXT"]);
    assert_eq!(fs.read_to_vec("/SUBDIR/inner.txt").unwrap(), b"inner");
    hadris_fs::sync::contract::check_read_only(&mut fs).unwrap();

    put(&mut image, 24, &file_record(FILE, &[]));
    let mut fs = open(image);
    let root = fs.root();
    let sub = fs.lookup(root, name("SUBDIR")).unwrap();
    let err = fs.lookup(sub, name("INNER.TXT")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
}

#[test]
fn compressed_and_encrypted_streams_are_unsupported() {
    for flag in [raw::ATTR_FLAG_COMPRESSED, raw::ATTR_FLAG_ENCRYPTED] {
        let mut image = base_image();
        let mut bin = non_resident(
            raw::ATTR_DATA,
            &[],
            0,
            3,
            3,
            &[0x11, 0x01, BIN_LCN as u8, 0],
        );
        bin[0x0C..0x0E].copy_from_slice(&flag.to_le_bytes());
        put(
            &mut image,
            18,
            &file_record(FILE, &[named(5, "BIN.DAT", false), bin]),
        );
        let mut fs = open(image);
        assert_eq!(
            fs.read_to_vec("/BIN.DAT").unwrap_err().kind(),
            ErrorKind::Unsupported
        );
        assert_eq!(fs.metadata("/BIN.DAT").unwrap().len(), 3);
    }
}

#[test]
fn large_streams_read_across_runs() {
    let mut image = base_image();
    let len = 3 * SECTOR + 100;
    let runs = [
        0x21,
        0x02,
        STREAM_LCN as u8,
        0,
        0x01,
        0x01,
        0x11,
        0x01,
        0x05,
        0x00,
    ];
    let big = non_resident(raw::ATTR_DATA, &[], 3, len as u64, len as u64, &runs);
    put(
        &mut image,
        18,
        &file_record(FILE, &[named(5, "BIN.DAT", false), big]),
    );
    let content: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    image[STREAM_LCN * SECTOR..(STREAM_LCN + 2) * SECTOR].copy_from_slice(&content[..2 * SECTOR]);
    let third = (STREAM_LCN + 5) * SECTOR;
    image[third..third + 100].copy_from_slice(&content[3 * SECTOR..]);
    let mut expected = content.clone();
    expected[2 * SECTOR..3 * SECTOR].fill(0);
    let mut fs = open(image);
    assert_eq!(fs.read_to_vec("/BIN.DAT").unwrap(), expected);
    let root = fs.root();
    let bin = fs.lookup(root, name("BIN.DAT")).unwrap();
    let mut buf = [0u8; 700];
    assert_eq!(fs.read(bin, 400, &mut buf).unwrap(), 700);
    assert_eq!(&buf[..], &expected[400..1100]);
}

#[test]
fn async_modes_walk_and_reject_corruption() {
    use hadris_fs::r#async::FileSystem as _;
    block_on(async {
        let mut fs = hadris_ntfs::r#async::NtfsFs::open(device(base_image()))
            .await
            .unwrap();
        assert_eq!(read_async(&mut fs, "/HELLO.TXT").await, b"hello ntfs");
        let mut fs =
            hadris_ntfs::r#async::NtfsFs::open(device(allocation_image(u32::MAX, &[1], None)))
                .await
                .unwrap();
        let root = fs.root();
        let sub = fs.lookup(root, name("SUBDIR")).await.unwrap();
        assert!(fs.readdir(sub, DirCursor::START).await.is_err());
    });
}

async fn read_async<F: hadris_fs::r#async::FileSystem>(fs: &mut F, path: &str) -> Vec<u8> {
    let node = fs
        .resolve(path.as_bytes(), hadris_fs::Resolve::Lexical)
        .await
        .unwrap();
    fs.open(node, hadris_fs::OpenMode::Read).await.unwrap();
    let mut out = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match fs.read(node, out.len() as u64, &mut chunk).await.unwrap() {
            0 => break,
            n => out.extend_from_slice(&chunk[..n]),
        }
    }
    fs.close(node).await.unwrap();
    fs.forget(node, 1);
    out
}

fn block_on<F: core::future::Future>(future: F) -> F::Output {
    use core::task::{Context, Poll, Waker};
    let mut future = core::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
    }
}
