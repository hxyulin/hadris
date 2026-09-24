//! `FatFs` in the `async` mode.

#[path = "common/fatfs.rs"]
mod common;
use common::FsPaths;
use common::paths::r#async::{FsPaths as _, VolumePaths as _};
use hadris_fs::r#async::FileSystem;
use hadris_fs::{Cp437, MountOptions};

use std::sync::Arc;

use common::{CASES, INNER, INNER_FILES, LONG_NAME, block_on};
use hadris_fs::{DirCursor, ErrorKind, Name, OpenMode, OpenOptions, RenameMode, SetAttr};

#[test]
fn async_mode_reads_through_every_tier() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::Volume;
    use hadris_io::r#async::Read as _;

    let case = CASES[2];
    block_on(async {
        let mut fs = FatFs::mount(
            common::device(case, common::build(case)),
            MountOptions::new(),
        )
        .await
        .unwrap();
        let root = fs.root();
        let long = fs
            .lookup(root, Name::new("a LONG file name.TXT"))
            .await
            .unwrap();
        let mut buf = [0u8; 16];
        assert_eq!(fs.read(long, 4990, &mut buf).await.unwrap(), 10);
        assert_eq!(buf[..10], common::payload(5000, 1)[4990..]);
        fs.forget(long, 1);

        let first = fs.readdir(root, DirCursor::START).await.unwrap().unwrap();
        assert_eq!(first.name().as_bytes(), b"README.TXT");
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
        let mut file = vol
            .open("/frag.bin", OpenOptions::new().read())
            .await
            .unwrap();
        let mut head = [0u8; 100];
        file.read_exact(&mut head).await.unwrap();
        assert_eq!(head[..], common::payload(100, 2)[..]);
        file.close().await.unwrap();
        assert_eq!(vol.into_inner().await.unwrap().open_nodes(), 1);
    });
}

fn spawn_read<F: hadris_fs::r#async::FileSystem + Send + 'static>(
    vol: Arc<hadris_fs::r#async::Volume<F>>,
    path: &'static str,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || block_on(async move { vol.read_to_vec(path).await.unwrap() }))
}

#[test]
fn async_futures_move_to_other_threads() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::Volume;

    let case = CASES[1];
    let fs = block_on(FatFs::mount(
        common::device(case, common::build(case)),
        MountOptions::new(),
    ))
    .unwrap();
    let vol = Arc::new(Volume::new(fs));
    let deep = spawn_read(Arc::clone(&vol), "/nested dir/inner/deep.bin");
    let long = spawn_read(Arc::clone(&vol), "/A long file name.txt");
    assert_eq!(deep.join().unwrap(), common::payload(70_000, 5));
    assert_eq!(long.join().unwrap(), common::payload(5000, 1));

    let task = async {
        let mut fs = vol.lock().await;
        let root = fs.root();
        fs.lookup(root, Name::new(LONG_NAME))
            .await
            .map(|node| fs.forget(node, 1))
    };
    fn assert_send<T: Send>(value: T) -> T {
        value
    }
    block_on(assert_send(task)).unwrap();
    let fs = block_on(Arc::into_inner(vol).unwrap().into_inner()).unwrap();
    assert_eq!(fs.open_nodes(), 1);
}

#[test]
fn async_mounts_with_options() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::SystemClock;
    use hadris_fs::r#async::Volume;

    let case = CASES[0];
    let options = MountOptions::new()
        .with_clock(&SystemClock)
        .with_code_page(&Cp437);
    let fs = block_on(FatFs::mount(
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
            let err = hadris_fat::r#async::FatFs::mount(
                common::device(case, image.clone()),
                MountOptions::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotRecognized);
            assert_eq!(err.into_device().into_inner(), image);

            let err = hadris_fat::r#async::FatFs::mount(
                common::device(case, image.clone()),
                MountOptions::new(),
            )
            .await
            .unwrap_err();
            let (error, dev) = err.into_parts();
            assert_eq!(error.kind(), ErrorKind::NotRecognized);
            assert_eq!(dev.into_inner(), image);

            let options = hadris_fs::MountOptions::new().read_only();
            let err =
                hadris_fat::r#async::FatFs::mount(common::device(case, image.clone()), options)
                    .await
                    .unwrap_err();
            assert_eq!(err.into_device().into_inner(), image);
        });
    }
}

#[test]
fn async_mode_writes() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::Volume;
    use hadris_io::r#async::Write as _;

    let case = CASES[0];
    let image = block_on(async {
        let mut fs = FatFs::mount(
            common::device(case, common::blank(case)),
            MountOptions::new(),
        )
        .await
        .unwrap();
        let root = fs.root();
        let meta = SetAttr::new();
        let dir = fs
            .mkdir(root, Name::new("A Directory"), &meta)
            .await
            .unwrap();
        let file = fs.create(dir, Name::new("notes.txt"), &meta).await.unwrap();
        let data = common::payload(9_000, 3);
        assert_eq!(fs.write(file, 0, &data).await.unwrap(), data.len());
        fs.truncate(file, 8_000).await.unwrap();
        fs.rename(
            dir,
            Name::new("notes.txt"),
            root,
            Name::new("Renamed Notes.txt"),
            RenameMode::Replace,
        )
        .await
        .unwrap();
        fs.open(file, OpenMode::Write).await.unwrap();
        assert_eq!(
            fs.unlink(root, Name::new("renamed notes.txt"))
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::Busy
        );
        fs.close(file).await.unwrap();
        fs.rmdir(root, Name::new("a directory")).await.unwrap();
        assert_eq!(fs.stat(dir).await.unwrap_err().kind(), ErrorKind::NotFound);
        fs.forget(dir, 1);
        let mut buf = vec![0u8; 9_000];
        assert_eq!(fs.read(file, 0, &mut buf).await.unwrap(), 8_000);
        assert_eq!(buf[..8_000], data[..8_000]);
        fs.forget(file, 1);
        fs.sync().await.unwrap();
        assert_eq!(fs.open_nodes(), 1);
        fs.write_file("/second.bin", b"second").await.unwrap();

        let vol = Volume::new(fs);
        let mut log = vol
            .open("/log.txt", OpenOptions::new().write().create().append())
            .await
            .unwrap();
        log.write_all(b"one ").await.unwrap();
        log.write_all(b"two").await.unwrap();
        log.close().await.unwrap();
        assert_eq!(vol.read_to_vec("/LOG.TXT").await.unwrap(), b"one two");
        let mut fs = vol.into_inner().await.unwrap();
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
fn async_writers_on_other_threads() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::Volume;

    let case = CASES[2];
    let fs = block_on(FatFs::mount(
        common::device(case, common::blank(case)),
        MountOptions::new(),
    ))
    .unwrap();
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
    block_on(async { vol.lock().await.sync().await }).unwrap();
    let fs = block_on(Arc::into_inner(vol).unwrap().into_inner()).unwrap();
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

        let (error, dev) = hadris_fat::r#async::format(device(), options())
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
        use hadris_fs::r#async::Volume;
        let fs = hadris_fat::r#async::format(device(), options())
            .await
            .unwrap();
        assert_eq!(fs.kind(), FatKind::Fat32);
        let vol = Volume::new(fs);
        vol.write_file("/async.txt", b"async").await.unwrap();
        vol.lock().await.sync().await.unwrap();
        vol.into_inner().await.unwrap().into_inner().into_inner()
    });
    let send = block_on(assert_send(hadris_fat::r#async::format(
        device(),
        options(),
    )))
    .unwrap()
    .into_inner()
    .into_inner();
    assert_eq!(sync, send);
    let mut fs = hadris_fat::sync::FatFs::mount(
        MemDevice::new(image.clone(), BlockSize::new(512).unwrap()),
        MountOptions::new(),
    )
    .unwrap();
    assert_eq!(fs.label_text().unwrap().unwrap(), "ASYNC");
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
    use hadris_storage::{BlockSize, MemDevice};

    const BLOCK: usize = 4096;
    let case = CASES[0];
    let name = Name::new("a long file name.txt");
    let flags = RenameMode::Replace;
    let meta = SetAttr::new();
    let empty = || MemDevice::new(Vec::new(), BlockSize::new(512).unwrap());

    let options = hadris_fs::MountOptions::new();
    assert_below("FatFs mount", FatFs::mount(empty(), options), 2 * BLOCK);
    let mut fat = block_on(FatFs::mount(
        common::device(case, common::build(case)),
        MountOptions::new(),
    ))
    .unwrap();
    let root = fat.root();
    assert_below(
        "FatFs rename",
        fat.rename(root, name, root, name, flags),
        3584,
    );
    assert_below("FatFs create", fat.create(root, name, &meta), 2240);

    let options = hadris_fs::MountOptions::new();
    assert_below("ExFatFs mount", ExFatFs::mount(empty(), options), 3 * BLOCK);
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
    assert_below("ExFatFs create", exfat.create(root, name, &meta), 4608);
}

#[path = "common/cancel.rs"]
mod cancel;

#[test]
fn dropped_operations_leave_no_lost_clusters_or_unequal_fats() {
    use hadris_fat::r#async::{FatFs, check};
    use hadris_fs::MountOptions;

    for case in [CASES[0], CASES[2]] {
        let dev = cancel::YieldDev(common::device(case, common::blank(case)));
        let options = MountOptions::new();
        let mut fs = cancel::run_for(FatFs::mount(dev, options), usize::MAX)
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
        let mut dev = fs.into_inner();
        let mut findings = Vec::new();
        let report = cancel::run_for(
            check(&mut dev, &mut [0u8; 8192], |finding| {
                findings.push(finding.to_string())
            }),
            usize::MAX,
        )
        .unwrap()
        .unwrap();
        assert!(report.is_clean(), "{}: {findings:?}", case.name);
        common::fsck(&dev.0.into_inner(), "dropped operations");
    }
}
