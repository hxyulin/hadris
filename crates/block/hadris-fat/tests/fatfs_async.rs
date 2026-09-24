//! `FatFs` in the `async` and `async_send` modes.

#[path = "common/fatfs.rs"]
mod common;

use std::sync::Arc;

use common::{CASES, INNER, INNER_FILES, LONG_NAME, block_on};
use hadris_fs::{
    DirCursor, ErrorKind, Name, NameBuf, NewNode, OpenOptions, RemoveKind, RenameFlags, SetMetadata,
};

#[test]
fn async_mode_reads_through_every_tier() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::{DriverExt, PathExt, Volume};
    use hadris_io::r#async::Read as _;

    let case = CASES[2];
    block_on(async {
        let mut fs = FatFs::open(common::device(case, common::build(case)))
            .await
            .unwrap();
        let root = fs.root();
        let long = fs
            .lookup(root, Name::new("a LONG file name.TXT").unwrap())
            .await
            .unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(fs.read_at(long, 4990, &mut buf).await.unwrap(), 10);
        assert_eq!(buf[..10], common::payload(5000, 1)[4990..]);
        fs.forget(long);

        let mut cursor = DirCursor::start();
        let mut name = NameBuf::new();
        let first = fs
            .read_dir_entry(root, &mut cursor, &mut name)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(name.as_bytes(), b"README.TXT");
        assert!(first.file_type().is_file());
        assert_eq!(
            fs.read_to_vec("/Nested Dir/sibling.txt").await.unwrap(),
            b"sibling"
        );

        let vol = Volume::new(fs);
        let mut dir = vol.read_dir(INNER).await.unwrap();
        let mut count = 0;
        while let Some(item) = dir.next_entry().await {
            item.unwrap();
            count += 1;
        }
        drop(dir);
        assert_eq!(count, INNER_FILES);
        let mut file = vol.open("/frag.bin", OpenOptions::read()).await.unwrap();
        let mut head = [0u8; 100];
        file.read_exact(&mut head).await.unwrap();
        assert_eq!(head[..], common::payload(100, 2)[..]);
        file.close().await.unwrap();
        assert_eq!(vol.into_inner().open_nodes(), 1);
    });
}

fn spawn_read<F: hadris_fs::async_send::FileSystem + 'static>(
    fs: Arc<F>,
    path: &'static str,
) -> std::thread::JoinHandle<Vec<u8>> {
    use hadris_fs::async_send::PathExt;
    std::thread::spawn(move || block_on(async move { fs.read_to_vec(path).await.unwrap() }))
}

#[test]
fn async_send_futures_move_to_other_threads() {
    use hadris_fat::async_send::FatFs;
    use hadris_fs::async_send::Volume;

    let case = CASES[1];
    let fs = block_on(FatFs::open(common::device(case, common::build(case)))).unwrap();
    let vol = Arc::new(Volume::new(fs));
    let deep = spawn_read(Arc::clone(&vol), "/nested dir/inner/deep.bin");
    let long = spawn_read(Arc::clone(&vol), "/A long file name.txt");
    assert_eq!(deep.join().unwrap(), common::payload(70_000, 5));
    assert_eq!(long.join().unwrap(), common::payload(5000, 1));

    let task = async {
        let mut fs = vol.lock().await;
        let root = fs.root();
        fs.lookup(root, Name::new(LONG_NAME).unwrap())
            .await
            .map(|node| fs.forget(node))
    };
    fn assert_send<T: Send>(value: T) -> T {
        value
    }
    block_on(assert_send(task)).unwrap();
    let fs = Arc::into_inner(vol).unwrap().into_inner();
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn async_send_mounts_with_options() {
    use hadris_fat::async_send::FatFs;
    use hadris_fat::{Cp437, MountOptions};
    use hadris_fs::async_send::Volume;
    use hadris_fs::{HeapTable, SystemClock};

    let case = CASES[0];
    let options = MountOptions::new()
        .with_table(HeapTable::new())
        .with_clock(SystemClock)
        .with_code_page(Cp437);
    let fs = block_on(FatFs::open_with(
        common::device(case, common::build(case)),
        options,
    ))
    .unwrap();
    let vol = Arc::new(Volume::new(fs));
    let kanji = spawn_read(Arc::clone(&vol), "/\u{3C3}ABC.TXT");
    assert_eq!(kanji.join().unwrap(), b"kanji");
}

#[test]
fn async_failed_opens_give_the_device_back() {
    let case = CASES[0];
    let mut corrupt = common::build(case);
    corrupt[11..13].copy_from_slice(&0u16.to_le_bytes());
    for image in [vec![0u8; 64 * 1024], corrupt] {
        block_on(async {
            let err = hadris_fat::r#async::FatFs::open(common::device(case, image.clone()))
                .await
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Corrupt);
            assert_eq!(err.into_device().into_inner(), image);

            let err = hadris_fat::async_send::FatFs::open(common::device(case, image.clone()))
                .await
                .unwrap_err();
            let (error, dev) = err.into_parts();
            assert_eq!(error.kind(), ErrorKind::Corrupt);
            assert_eq!(dev.into_inner(), image);

            let options = hadris_fat::MountOptions::new().with_read_only();
            let err = hadris_fat::async_send::FatFs::open_with(
                common::device(case, image.clone()),
                options,
            )
            .await
            .unwrap_err();
            assert_eq!(err.into_device().into_inner(), image);
        });
    }
}

#[test]
fn async_mode_writes() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::{DriverExt, PathExt, Volume};
    use hadris_io::r#async::Write as _;

    let case = CASES[0];
    let image = block_on(async {
        let mut fs = FatFs::open(common::device(case, common::blank(case)))
            .await
            .unwrap();
        let root = fs.root();
        let meta = SetMetadata::new();
        let dir = fs
            .create(root, Name::new("A Directory").unwrap(), NewNode::Dir, &meta)
            .await
            .unwrap();
        let file = fs
            .create(dir, Name::new("notes.txt").unwrap(), NewNode::File, &meta)
            .await
            .unwrap();
        let data = common::payload(9_000, 3);
        assert_eq!(fs.write_at(file, 0, &data).await.unwrap(), data.len());
        fs.set_len(file, 8_000).await.unwrap();
        fs.rename(
            dir,
            Name::new("notes.txt").unwrap(),
            root,
            Name::new("Renamed Notes.txt").unwrap(),
            RenameFlags::empty(),
        )
        .await
        .unwrap();
        fs.open_node(dir).await.unwrap();
        assert_eq!(
            fs.remove(root, Name::new("A Directory").unwrap(), RemoveKind::Any)
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::Busy
        );
        fs.close_node(dir);
        fs.remove(root, Name::new("a directory").unwrap(), RemoveKind::Dir)
            .await
            .unwrap();
        assert_eq!(
            fs.node_metadata(dir).await.unwrap_err().kind(),
            ErrorKind::NotFound
        );
        fs.forget(dir);
        let mut buf = vec![0u8; 9_000];
        assert_eq!(fs.read_at(file, 0, &mut buf).await.unwrap(), 8_000);
        assert_eq!(buf[..8_000], data[..8_000]);
        fs.forget(file);
        fs.sync().await.unwrap();
        assert_eq!(fs.open_nodes(), 1);
        fs.write_file("/second.bin", b"second").await.unwrap();

        let vol = Volume::new(fs);
        let mut log = vol
            .open("/log.txt", OpenOptions::write().create().append())
            .await
            .unwrap();
        log.write_all(b"one ").await.unwrap();
        log.write_all(b"two").await.unwrap();
        log.close().await.unwrap();
        assert_eq!(vol.read_to_vec("/LOG.TXT").await.unwrap(), b"one two");
        let mut fs = vol.into_inner();
        fs.sync().await.unwrap();
        fs.into_inner().into_inner()
    });
    let mut fs = common::mount(case, &image);
    assert_eq!(
        common::read(&mut fs, "/Renamed Notes.txt"),
        common::payload(8_000, 3)
    );
    assert_eq!(common::read(&mut fs, "/second.bin"), b"second");
    assert_eq!(common::read(&mut fs, "/log.txt"), b"one two");
    assert!(
        !common::names(&mut fs, "/")
            .iter()
            .any(|n| n.eq_ignore_ascii_case("A Directory"))
    );
}

#[test]
fn async_send_writers_on_other_threads() {
    use hadris_fat::async_send::FatFs;
    use hadris_fs::async_send::{FileSystem, PathExt, Volume};

    let case = CASES[2];
    let fs = block_on(FatFs::open(common::device(case, common::blank(case)))).unwrap();
    let vol = Arc::new(Volume::new(fs));
    let writers: Vec<_> = (0..4u8)
        .map(|i| {
            let vol = Arc::clone(&vol);
            std::thread::spawn(move || {
                block_on(async move {
                    let path = format!("/writer {i}.bin");
                    vol.write_file(&path, &common::payload(20_000, i))
                        .await
                        .unwrap();
                })
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    for i in 0..4u8 {
        let data = spawn_read(
            Arc::clone(&vol),
            [
                "/writer 0.bin",
                "/writer 1.bin",
                "/writer 2.bin",
                "/writer 3.bin",
            ][i as usize],
        );
        assert_eq!(data.join().unwrap(), common::payload(20_000, i));
    }
    block_on(vol.sync()).unwrap();
    let fs = Arc::into_inner(vol).unwrap().into_inner();
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn async_failed_formats_give_the_device_back() {
    use hadris_fat::{FatKind, FormatOptions};
    use hadris_storage::{BlockSize, MemDevice};

    let device = || MemDevice::new(vec![0u8; 4 << 20], BlockSize::new(512).unwrap());
    let options = || FormatOptions::new().with_kind(FatKind::Fat32);
    block_on(async {
        let err = hadris_fat::r#async::format(device(), options())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NoSpace);
        assert_eq!(err.into_device().into_inner(), vec![0u8; 4 << 20]);

        let (error, dev) = hadris_fat::async_send::format(device(), options())
            .await
            .unwrap_err()
            .into_parts();
        assert_eq!(error.kind(), ErrorKind::NoSpace);
        assert_eq!(dev.into_inner(), vec![0u8; 4 << 20]);
    });
}

#[test]
fn format_in_the_async_modes() {
    use hadris_fat::{FatKind, FormatOptions, VolumeLabel};
    use hadris_fs::sync::DriverExt as _;
    use hadris_storage::{BlockSize, MemDevice};

    fn assert_send<T: Send>(value: T) -> T {
        value
    }
    let options = || {
        FormatOptions::new()
            .with_kind(FatKind::Fat32)
            .with_label(VolumeLabel::new("ASYNC").unwrap())
    };
    let device = || MemDevice::new(vec![0u8; 40 << 20], BlockSize::new(512).unwrap());

    let sync = hadris_fat::sync::format(device(), options())
        .unwrap()
        .into_inner()
        .into_inner();
    let image = block_on(async {
        use hadris_fs::r#async::{FileSystem, PathExt, Volume};
        let fs = hadris_fat::r#async::format(device(), options())
            .await
            .unwrap();
        assert_eq!(fs.kind(), FatKind::Fat32);
        let vol = Volume::new(fs);
        vol.write_file("/async.txt", b"async").await.unwrap();
        vol.sync().await.unwrap();
        vol.into_inner().into_inner().into_inner()
    });
    let send = block_on(assert_send(hadris_fat::async_send::format(
        device(),
        options(),
    )))
    .unwrap()
    .into_inner()
    .into_inner();
    assert_eq!(sync, send);
    let mut fs =
        hadris_fat::sync::FatFs::open(MemDevice::new(image.clone(), BlockSize::new(512).unwrap()))
            .unwrap();
    assert_eq!(fs.label().unwrap().unwrap().as_str(), "ASYNC");
    assert_eq!(fs.read_to_vec("/async.txt").unwrap(), b"async");
    common::fsck(&image, "async format");
}

fn assert_below<F: core::future::Future>(what: &str, future: F, limit: usize) {
    let size = size_of_val(&future);
    assert!(size < limit, "{what} future is {size} bytes, limit {limit}");
}

/// Mount futures hold one block buffer, and directory operations hold no
/// block-sized buffer or extra copies of a long name or an entry set.
#[test]
fn async_futures_stay_small() {
    use hadris_fat::r#async::FatFs;
    use hadris_fat::exfat::r#async::ExFatFs;
    use hadris_fs::FixedTable;
    use hadris_storage::{BlockSize, MemDevice};

    const BLOCK: usize = 4096;
    let case = CASES[0];
    let name = Name::new("a long file name.txt").unwrap();
    let flags = RenameFlags::empty();
    let meta = SetMetadata::new();
    let empty = || MemDevice::new(Vec::new(), BlockSize::new(512).unwrap());

    let options = hadris_fat::MountOptions::new().with_table(FixedTable::<1>::new());
    assert_below("FatFs mount", FatFs::open_with(empty(), options), 2 * BLOCK);
    let mut fat = block_on(FatFs::open(common::device(case, common::build(case)))).unwrap();
    let root = fat.root();
    assert_below(
        "FatFs rename",
        fat.rename(root, name, root, name, flags),
        3456,
    );
    assert_below(
        "FatFs create",
        fat.create(root, name, NewNode::File, &meta),
        2240,
    );

    let options = hadris_fat::exfat::MountOptions::new().with_table(FixedTable::<1>::new());
    assert_below(
        "ExFatFs mount",
        ExFatFs::open_with(empty(), options),
        3 * BLOCK,
    );
    let dev = MemDevice::new(vec![0u8; 4 << 20], BlockSize::new(512).unwrap());
    let formatted =
        hadris_fat::exfat::r#async::format(dev, hadris_fat::exfat::FormatOptions::new());
    let mut exfat = block_on(formatted).unwrap();
    let root = exfat.root();
    assert_below(
        "ExFatFs rename",
        exfat.rename(root, name, root, name, flags),
        2 * BLOCK,
    );
    assert_below(
        "ExFatFs create",
        exfat.create(root, name, NewNode::File, &meta),
        4480,
    );
}

#[path = "common/cancel.rs"]
mod cancel;

#[test]
fn dropped_operations_leave_no_lost_clusters_or_unequal_fats() {
    use hadris_fat::MountOptions;
    use hadris_fat::r#async::{FatFs, check_with};
    use hadris_fs::HeapTable;

    for case in [CASES[0], CASES[2]] {
        let dev = cancel::YieldDev(common::device(case, common::blank(case)));
        let options = MountOptions::new().with_table(HeapTable::new());
        let mut fs = cancel::run_for(FatFs::open_with(dev, options), usize::MAX)
            .unwrap()
            .unwrap();
        let mut rng = cancel::Rng(7);
        let mut dropped = 0;
        for i in 0..400 {
            let kind = rng.below(5);
            let polls = rng.below(50) as usize + 1;
            match cancel::run_for(cancel::step(&mut fs, kind, i), polls) {
                None => dropped += 1,
                Some(Err(kind @ (ErrorKind::Corrupt | ErrorKind::Io))) => {
                    panic!("{}: step {i}: {kind:?}", case.name)
                }
                Some(_) => {}
            }
        }
        assert!(dropped > 50, "{}: {dropped} dropped", case.name);
        cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
        let mut findings = Vec::new();
        let report = cancel::run_for(
            check_with(&mut fs, &mut [0u8; 8192], |finding| findings.push(finding)),
            usize::MAX,
        )
        .unwrap()
        .unwrap();
        assert!(report.is_clean(), "{}: {findings:?}", case.name);
        common::fsck(&fs.into_inner().0.into_inner(), "dropped operations");
    }
}
