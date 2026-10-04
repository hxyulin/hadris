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
fn async_failed_formats_write_nothing() {
    use hadris_fat::{FatKind, FatOptions};
    use hadris_storage::{BlockSize, MemDevice};

    let mut dev = MemDevice::new(vec![0u8; 4 << 20], BlockSize::new(512).unwrap());
    let options = FatOptions::new().with_kind(FatKind::Fat32);
    block_on(async {
        let err = hadris_fat::r#async::format(&mut dev, &options)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NoSpace);
    });
    assert_eq!(dev.into_inner(), vec![0u8; 4 << 20]);
}

#[test]
fn format_in_the_async_modes() {
    use hadris_fat::{FatKind, FatOptions, VolumeLabel};

    use hadris_storage::{BlockSize, MemDevice};

    fn assert_send<T: Send>(value: T) -> T {
        value
    }
    let options = || {
        FatOptions::new()
            .with_kind(FatKind::Fat32)
            .with_label(VolumeLabel::new("ASYNC").unwrap())
    };
    let device = || MemDevice::new(vec![0u8; 40 << 20], BlockSize::new(512).unwrap());

    let sync = {
        let mut dev = device();
        hadris_fat::sync::format(&mut dev, &options()).unwrap();
        dev.into_inner()
    };
    let image = block_on(async {
        use hadris_fs::r#async::Volume;
        let mut dev = device();
        let geometry = hadris_fat::r#async::format(&mut dev, &options())
            .await
            .unwrap();
        assert_eq!(geometry.kind(), FatKind::Fat32);
        let fs = hadris_fat::r#async::FatFs::mount(dev, MountOptions::new())
            .await
            .unwrap();
        let vol = Volume::new(fs);
        vol.write_file("/async.txt", b"async").await.unwrap();
        vol.lock().await.sync().await.unwrap();
        vol.into_inner().await.unwrap().into_inner().into_inner()
    });
    let mut dev = device();
    block_on(assert_send(hadris_fat::r#async::format(
        &mut dev,
        &options(),
    )))
    .unwrap();
    let send = dev.into_inner();
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
/// FAT budgets include 64 bytes for the read-through metadata adapter.
#[test]
#[cfg_attr(
    feature = "tracing",
    ignore = "resource budgets apply without hosted tracing spans"
)]
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
        3648,
    );
    assert_below("FatFs create", fat.create(root, name, &meta), 2304);
    let label = Some(hadris_fat::VolumeLabel::new("DATA").unwrap());
    assert_below("FatFs set_label", fat.set_label(label), 3648);
    let mut out = [hadris_fs::Extent::new(0, 0); 1];
    assert_below("FatFs extents", fat.extents(root, 0, &mut out), 576);

    let options = hadris_fs::MountOptions::new();
    assert_below("ExFatFs mount", ExFatFs::mount(empty(), options), 3 * BLOCK);
    let mut dev = MemDevice::new(vec![0u8; 4 << 20], BlockSize::new(512).unwrap());
    let options = hadris_fat::exfat::ExFatOptions::new();
    let formatted = hadris_fat::exfat::r#async::format(&mut dev, &options);
    assert_below("exFAT format", formatted, 2 * BLOCK);
    block_on(hadris_fat::exfat::r#async::format(&mut dev, &options)).unwrap();
    let mut exfat = block_on(ExFatFs::mount(dev, MountOptions::new())).unwrap();
    let root = exfat.root();
    assert_below(
        "ExFatFs rename",
        exfat.rename(root, name, root, name, flags),
        2 * BLOCK,
    );
    assert_below("ExFatFs create", exfat.create(root, name, &meta), 4608);
    assert_below(
        "ExFatFs set_volume_serial",
        exfat.set_volume_serial(1),
        2560,
    );
    assert_below("ExFatFs records", exfat.records(root, &mut out), 2048);
}

#[test]
fn async_extras_match_the_sync_mode() {
    use hadris_fat::r#async::FatFs;

    let case = CASES[2];
    block_on(async {
        let mut fs = FatFs::mount(
            common::device(case, common::build(case)),
            MountOptions::new(),
        )
        .await
        .unwrap();
        assert!(!fs.was_dirty());
        assert_eq!(fs.info().kind(), case.kind);
        let root = fs.root();
        let long = fs.lookup(root, Name::new(LONG_NAME)).await.unwrap();
        let mut out = [hadris_fs::Extent::new(0, 0); 4];
        let n = fs.extents(long, 0, &mut out).await.unwrap();
        let mut data = Vec::new();
        for extent in &out[..n] {
            let mut buf = vec![0; extent.len() as usize];
            fs.read_raw(extent.offset(), &mut buf).await.unwrap();
            data.extend(buf);
        }
        assert_eq!(data, common::payload(5000, 1));
        assert_eq!(fs.records(long, &mut out).await.unwrap(), 1);
        fs.forget(long, 1);
        fs.set_label(Some(hadris_fat::VolumeLabel::new("ASYNC").unwrap()))
            .await
            .unwrap();
        fs.set_volume_serial(7).await.unwrap();
        assert_eq!(fs.info().volume_serial(), Some(7));
        let mut buf = [0u8; 16];
        assert_eq!(fs.label(&mut buf).await.unwrap(), Some("ASYNC"));
    });
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

#[test]
fn single_cluster_append_recovers_at_every_await() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;
    for case in CASES[..3].iter().copied() {
        let blank = common::blank(case);
        let cluster = hadris_fat_raw::parse_boot(blank[..512].try_into().unwrap())
            .unwrap()
            .cluster_size() as usize;
        let mut fs =
            hadris_fat::sync::FatFs::mount(common::device(case, blank), MountOptions::new())
                .unwrap();
        let root = hadris_fs::sync::FileSystem::root(&fs);
        let file = hadris_fs::sync::FileSystem::create(
            &mut fs,
            root,
            Name::new("LOG.BIN"),
            &SetAttr::new(),
        )
        .unwrap();
        hadris_fs::sync::FileSystem::write(&mut fs, file, 0, &vec![7; cluster]).unwrap();
        let before = fs.unmount().unwrap().into_inner();
        let mut completed = false;
        for budget in 0..100 {
            let mut fs = cancel::run_for(
                FatFs::mount(
                    cancel::YieldDev(common::device(case, before.clone())),
                    MountOptions::new(),
                ),
                usize::MAX,
            )
            .unwrap()
            .unwrap();
            let file = cancel::run_for(fs.lookup(fs.root(), Name::new("LOG.BIN")), usize::MAX)
                .unwrap()
                .unwrap();
            let result = cancel::run_for(fs.write(file, cluster as u64, &[9]), budget);
            cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
            let image = cancel::run_for(fs.unmount(), usize::MAX)
                .unwrap()
                .unwrap()
                .0
                .into_inner();
            common::assert_checks_clean(case, &image, case.name);
            let mut fresh = hadris_fat::sync::FatFs::mount(
                common::device(case, image),
                MountOptions::new().read_only(),
            )
            .unwrap();
            let root = hadris_fs::sync::FileSystem::root(&fresh);
            let file = hadris_fs::sync::FileSystem::lookup(&mut fresh, root, Name::new("LOG.BIN"))
                .unwrap();
            let mut data = vec![0; cluster + 1];
            let n = hadris_fs::sync::FileSystem::read(&mut fresh, file, 0, &mut data).unwrap();
            assert_eq!(&data[..cluster], &vec![7; cluster]);
            assert_eq!(n, cluster + usize::from(result.is_some()));
            if let Some(result) = result {
                assert_eq!(result.unwrap(), 1);
                assert_eq!(data[cluster], 9);
                completed = true;
                break;
            }
        }
        assert!(completed, "{} append never completed", case.name);
    }
}

#[test]
fn multi_cluster_append_recovers_at_every_await() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::r#async::FileSystem;
    for case in [CASES[0], CASES[1], CASES[2], CASES[4]] {
        let blank = common::blank(case);
        let geo = hadris_fat_raw::parse_boot(blank[..512].try_into().unwrap()).unwrap();
        let cluster = geo.cluster_size() as usize;
        let clusters = if case.kind == hadris_fat::FatKind::Fat12 {
            3
        } else {
            case.block as usize / case.kind.entry_len() - 5
        };
        let old = vec![7; clusters * cluster];
        let new = vec![9; 16 * cluster];
        let mut fs =
            hadris_fat::sync::FatFs::mount(common::device(case, blank), MountOptions::new())
                .unwrap();
        let root = hadris_fs::sync::FileSystem::root(&fs);
        let file = hadris_fs::sync::FileSystem::create(
            &mut fs,
            root,
            Name::new("LOG.BIN"),
            &SetAttr::new(),
        )
        .unwrap();
        hadris_fs::sync::FileSystem::write(&mut fs, file, 0, &old).unwrap();
        let before = fs.unmount().unwrap().into_inner();
        for cached in [false, true] {
            let mut completed = false;
            for budget in 0..200 {
                let mut fs = cancel::run_for(
                    FatFs::mount(
                        cancel::YieldDev(common::device(case, before.clone())),
                        MountOptions::new(),
                    ),
                    usize::MAX,
                )
                .unwrap()
                .unwrap();
                if cached {
                    fs = fs.with_cache(hadris_fat::CacheOptions::new());
                }
                let file = cancel::run_for(fs.lookup(fs.root(), Name::new("LOG.BIN")), usize::MAX)
                    .unwrap()
                    .unwrap();
                cancel::run_for(
                    fs.read(file, (old.len() - cluster) as u64, &mut vec![0; cluster]),
                    usize::MAX,
                )
                .unwrap()
                .unwrap();
                let result = cancel::run_for(fs.write(file, old.len() as u64, &new), budget);
                cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
                let image = cancel::run_for(fs.unmount(), usize::MAX)
                    .unwrap()
                    .unwrap()
                    .0
                    .into_inner();
                common::assert_checks_clean(
                    case,
                    &image,
                    &format!("{} poll {budget} cached {cached}", case.name),
                );
                let mut fresh = hadris_fat::sync::FatFs::mount(
                    common::device(case, image),
                    MountOptions::new().read_only(),
                )
                .unwrap();
                let root = hadris_fs::sync::FileSystem::root(&fresh);
                let file =
                    hadris_fs::sync::FileSystem::lookup(&mut fresh, root, Name::new("LOG.BIN"))
                        .unwrap();
                let mut data = vec![0; old.len() + new.len()];
                let n = hadris_fs::sync::FileSystem::read(&mut fresh, file, 0, &mut data).unwrap();
                assert_eq!(&data[..old.len()], &old);
                assert_eq!(n, old.len() + if result.is_some() { new.len() } else { 0 });
                if let Some(result) = result {
                    assert_eq!(result.unwrap(), new.len());
                    assert_eq!(&data[old.len()..], &new);
                    completed = true;
                    break;
                }
            }
            assert!(
                completed,
                "{} multi-cluster append never completed",
                case.name
            );
        }
    }
}

#[test]
fn nonempty_rename_recovers_at_every_await_and_keeps_pinned_nodes() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::sync::FileSystem as _;
    const SOURCE: &str = "source file with a long name.txt";
    const TARGET: &str = "destination file with a long name.txt";
    for case in CASES {
        for (cross, replace, directory) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (true, false, true),
            (true, true, true),
        ] {
            let mut original = common::formatted(case, hadris_fat::FatOptions::new());
            let root = original.root();
            let from = original
                .mkdir(root, Name::new("FROM"), &SetAttr::new())
                .unwrap();
            let to = original
                .mkdir(root, Name::new("TO"), &SetAttr::new())
                .unwrap();
            let destination = if cross { to } else { from };
            let source_data = vec![7; 2700];
            let target_data = vec![9; 1800];
            let source = if directory {
                let dir = original
                    .mkdir(from, Name::new(SOURCE), &SetAttr::new())
                    .unwrap();
                let child = original
                    .create(dir, Name::new("CHILD.BIN"), &SetAttr::new())
                    .unwrap();
                original.write(child, 0, &source_data).unwrap();
                original.close(child).unwrap();
                dir
            } else {
                let file = original
                    .create(from, Name::new(SOURCE), &SetAttr::new())
                    .unwrap();
                original.write(file, 0, &source_data).unwrap();
                original.close(file).unwrap();
                file
            };
            original.forget(source, 1);
            if replace {
                if directory {
                    original
                        .mkdir(destination, Name::new(TARGET), &SetAttr::new())
                        .unwrap();
                } else {
                    let file = original
                        .create(destination, Name::new(TARGET), &SetAttr::new())
                        .unwrap();
                    original.write(file, 0, &target_data).unwrap();
                    original.close(file).unwrap();
                }
            }
            let image = original.unmount().unwrap().into_inner();
            let mut completed = false;
            for budget in 0..240 {
                let mut fs = cancel::run_for(
                    FatFs::mount(
                        cancel::YieldDev(common::device(case, image.clone())),
                        MountOptions::new(),
                    ),
                    usize::MAX,
                )
                .unwrap()
                .unwrap()
                .with_cache(hadris_fat::CacheOptions::new());
                let root = fs.root();
                let from = cancel::run_for(fs.lookup(root, Name::new("FROM")), usize::MAX)
                    .unwrap()
                    .unwrap();
                let to = if cross {
                    cancel::run_for(fs.lookup(root, Name::new("TO")), usize::MAX)
                        .unwrap()
                        .unwrap()
                } else {
                    from
                };
                let source = cancel::run_for(fs.lookup(from, Name::new(SOURCE)), usize::MAX)
                    .unwrap()
                    .unwrap();
                let mut expected = source_data.clone();
                if !directory {
                    cancel::run_for(
                        fs.write(source, source_data.len() as u64, &[11; 100]),
                        usize::MAX,
                    )
                    .unwrap()
                    .unwrap();
                    expected.extend_from_slice(&[11; 100]);
                }
                let target = replace.then(|| {
                    cancel::run_for(fs.lookup(to, Name::new(TARGET)), usize::MAX)
                        .unwrap()
                        .unwrap()
                });
                let result = cancel::run_for(
                    fs.rename(
                        from,
                        Name::new(SOURCE),
                        to,
                        Name::new(TARGET),
                        RenameMode::Replace,
                    ),
                    budget,
                );
                cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
                let old = cancel::run_for(fs.lookup(from, Name::new(SOURCE)), usize::MAX).unwrap();
                let committed = old.is_err();
                if committed {
                    assert_eq!(old.unwrap_err().kind(), ErrorKind::NotFound);
                    assert_eq!(
                        cancel::run_for(fs.lookup(to, Name::new(TARGET)), usize::MAX)
                            .unwrap()
                            .unwrap(),
                        source
                    );
                    if let Some(target) = target {
                        assert_eq!(
                            cancel::run_for(fs.stat(target), usize::MAX)
                                .unwrap()
                                .unwrap_err()
                                .kind(),
                            ErrorKind::NotFound
                        );
                    }
                } else {
                    assert_eq!(old.unwrap(), source);
                    if let Some(target) = target {
                        assert_eq!(
                            cancel::run_for(fs.lookup(to, Name::new(TARGET)), usize::MAX)
                                .unwrap()
                                .unwrap(),
                            target
                        );
                    } else {
                        assert_eq!(
                            cancel::run_for(fs.lookup(to, Name::new(TARGET)), usize::MAX)
                                .unwrap()
                                .unwrap_err()
                                .kind(),
                            ErrorKind::NotFound
                        );
                    }
                }
                let file = if directory {
                    assert_eq!(
                        cancel::run_for(fs.parent(source), usize::MAX)
                            .unwrap()
                            .unwrap(),
                        if committed { to } else { from }
                    );
                    cancel::run_for(fs.lookup(source, Name::new("CHILD.BIN")), usize::MAX)
                        .unwrap()
                        .unwrap()
                } else {
                    source
                };
                let mut data = vec![0; expected.len()];
                assert_eq!(
                    cancel::run_for(fs.read(file, 0, &mut data), usize::MAX)
                        .unwrap()
                        .unwrap(),
                    data.len()
                );
                assert_eq!(data, expected);
                if !directory && replace && !committed {
                    let mut data = vec![0; target_data.len()];
                    cancel::run_for(fs.read(target.unwrap(), 0, &mut data), usize::MAX)
                        .unwrap()
                        .unwrap();
                    assert_eq!(data, target_data);
                }
                let image = cancel::run_for(fs.unmount(), usize::MAX)
                    .unwrap()
                    .unwrap()
                    .0
                    .into_inner();
                common::assert_checks_clean(
                    case,
                    &image,
                    &format!(
                        "rename await {budget}: cross={cross}, replace={replace}, dir={directory}"
                    ),
                );
                if let Some(result) = result {
                    result.unwrap();
                    completed = true;
                    break;
                }
            }
            assert!(completed, "rename never completed");
        }
    }
}

#[test]
fn cancelled_rename_does_not_publish_stale_entries_beyond_directory_end() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::sync::FileSystem as _;
    const TARGET: &str = "A replacement long name.txt";
    for case in CASES[..3].iter().copied() {
        let mut original = common::formatted(case, hadris_fat::FatOptions::new());
        let file = original
            .create(original.root(), Name::new("OLD.TXT"), &SetAttr::new())
            .unwrap();
        original.write(file, 0, b"preserved").unwrap();
        let mut image = original.unmount().unwrap().into_inner();
        let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
        let base = match geo.root() {
            hadris_fat_raw::RootLocation::Fixed { start, .. } => start,
            hadris_fat_raw::RootLocation::Cluster(first) => geo.cluster_offset(first).unwrap(),
        } as usize;
        let short = 1 + hadris_fat_raw::lfn::Encoded::new(TARGET).unwrap().entries();
        let mut probe = hadris_fat::sync::FatFs::mount(
            common::device(case, image.clone()),
            MountOptions::new(),
        )
        .unwrap();
        probe
            .rename(
                probe.root(),
                Name::new("OLD.TXT"),
                probe.root(),
                Name::new(TARGET),
                RenameMode::NoReplace,
            )
            .unwrap();
        let renamed = probe.unmount().unwrap().into_inner();
        image[base + short * 32..base + (short + 1) * 32]
            .copy_from_slice(&renamed[base + short * 32..base + (short + 1) * 32]);
        let garbage =
            hadris_fat_raw::ShortEntry::new(*b"GARBAGE BIN", hadris_fat_raw::ATTR_ARCHIVE);
        image[base + (short + 1) * 32..base + (short + 2) * 32].copy_from_slice(&garbage.encode());
        common::assert_checks_clean(case, &image, "stale entries hidden after End");
        let mut completed = false;
        for budget in 0..120 {
            let mut fs = cancel::run_for(
                FatFs::mount(
                    cancel::YieldDev(common::device(case, image.clone())),
                    MountOptions::new(),
                ),
                usize::MAX,
            )
            .unwrap()
            .unwrap()
            .with_cache(hadris_fat::CacheOptions::new());
            let root = fs.root();
            let source = cancel::run_for(fs.lookup(root, Name::new("OLD.TXT")), usize::MAX)
                .unwrap()
                .unwrap();
            let result = cancel::run_for(
                fs.rename(
                    root,
                    Name::new("OLD.TXT"),
                    root,
                    Name::new(TARGET),
                    RenameMode::NoReplace,
                ),
                budget,
            );
            cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
            let old = cancel::run_for(fs.lookup(root, Name::new("OLD.TXT")), usize::MAX).unwrap();
            let new = cancel::run_for(fs.lookup(root, Name::new(TARGET)), usize::MAX).unwrap();
            assert!(old.is_ok() ^ new.is_ok(), "{} budget {budget}", case.name);
            assert_eq!(old.or(new).unwrap(), source);
            assert_eq!(
                cancel::run_for(fs.lookup(root, Name::new("GARBAGE.BIN")), usize::MAX)
                    .unwrap()
                    .unwrap_err()
                    .kind(),
                ErrorKind::NotFound
            );
            let mut buf = [0; 9];
            cancel::run_for(fs.read(source, 0, &mut buf), usize::MAX)
                .unwrap()
                .unwrap();
            assert_eq!(&buf, b"preserved");
            let image = cancel::run_for(fs.unmount(), usize::MAX)
                .unwrap()
                .unwrap()
                .0
                .into_inner();
            common::assert_checks_clean(
                case,
                &image,
                &format!("stale destination budget {budget}"),
            );
            if let Some(result) = result {
                result.unwrap();
                completed = true;
                break;
            }
        }
        assert!(completed);
    }
}

#[test]
fn rename_recovery_itself_survives_cancellation_at_every_await() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::sync::FileSystem as _;
    let case = CASES[2];
    let mut original = common::formatted(case, hadris_fat::FatOptions::new());
    let root = original.root();
    let from = original
        .mkdir(root, Name::new("FROM"), &SetAttr::new())
        .unwrap();
    original
        .mkdir(root, Name::new("TO"), &SetAttr::new())
        .unwrap();
    let directory = original
        .mkdir(from, Name::new("SOURCE"), &SetAttr::new())
        .unwrap();
    let child = original
        .create(directory, Name::new("DATA.BIN"), &SetAttr::new())
        .unwrap();
    original.write(child, 0, &[7; 2700]).unwrap();
    let image = original.unmount().unwrap().into_inner();
    let mount = || {
        let mut fs = cancel::run_for(
            FatFs::mount(
                cancel::YieldDev(common::device(case, image.clone())),
                MountOptions::new(),
            ),
            usize::MAX,
        )
        .unwrap()
        .unwrap()
        .with_cache(hadris_fat::CacheOptions::new());
        let root = fs.root();
        let from = cancel::run_for(fs.lookup(root, Name::new("FROM")), usize::MAX)
            .unwrap()
            .unwrap();
        let to = cancel::run_for(fs.lookup(root, Name::new("TO")), usize::MAX)
            .unwrap()
            .unwrap();
        let source = cancel::run_for(fs.lookup(from, Name::new("SOURCE")), usize::MAX)
            .unwrap()
            .unwrap();
        (fs, from, to, source)
    };
    let mut rename_budget = None;
    for budget in 0..120 {
        let (mut fs, from, to, _) = mount();
        if let Some(result) = cancel::run_for(
            fs.rename(
                from,
                Name::new("SOURCE"),
                to,
                Name::new("Moved directory with a long name"),
                RenameMode::NoReplace,
            ),
            budget,
        ) {
            result.unwrap();
            rename_budget = Some(budget - 1);
            break;
        }
    }
    let rename_budget = rename_budget.unwrap();
    for recovery_budget in 0..120 {
        let (mut fs, from, to, source) = mount();
        assert!(
            cancel::run_for(
                fs.rename(
                    from,
                    Name::new("SOURCE"),
                    to,
                    Name::new("Moved directory with a long name"),
                    RenameMode::NoReplace
                ),
                rename_budget
            )
            .is_none()
        );
        let result = cancel::run_for(fs.sync(), recovery_budget);
        cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
        assert_eq!(
            cancel::run_for(
                fs.lookup(to, Name::new("Moved directory with a long name")),
                usize::MAX
            )
            .unwrap()
            .unwrap(),
            source
        );
        assert_eq!(
            cancel::run_for(fs.lookup(from, Name::new("SOURCE")), usize::MAX)
                .unwrap()
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            cancel::run_for(fs.parent(source), usize::MAX)
                .unwrap()
                .unwrap(),
            to
        );
        let child = cancel::run_for(fs.lookup(source, Name::new("DATA.BIN")), usize::MAX)
            .unwrap()
            .unwrap();
        let mut data = vec![0; 2700];
        cancel::run_for(fs.read(child, 0, &mut data), usize::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(data, vec![7; 2700]);
        let image = cancel::run_for(fs.unmount(), usize::MAX)
            .unwrap()
            .unwrap()
            .0
            .into_inner();
        common::assert_checks_clean(
            case,
            &image,
            &format!("rename recovery await {recovery_budget}"),
        );
        if let Some(result) = result {
            result.unwrap();
            return;
        }
    }
    panic!("rename recovery never completed");
}

#[test]
fn cancelled_replacement_rollback_cannot_commit_restored_identical_empty_metadata() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::sync::FileSystem as _;
    let case = CASES[0];
    let mut original = common::formatted(case, hadris_fat::FatOptions::new());
    let root = original.root();
    original
        .create(root, Name::new("SOURCE.TXT"), &SetAttr::new())
        .unwrap();
    original
        .create(root, Name::new("TARGET.TXT"), &SetAttr::new())
        .unwrap();
    let image = original.unmount().unwrap().into_inner();
    let mount = || {
        let mut fs = cancel::run_for(
            FatFs::mount(
                cancel::YieldDev(common::device(case, image.clone())),
                MountOptions::new(),
            ),
            usize::MAX,
        )
        .unwrap()
        .unwrap()
        .with_cache(hadris_fat::CacheOptions::new());
        let root = fs.root();
        let source = cancel::run_for(fs.lookup(root, Name::new("SOURCE.TXT")), usize::MAX)
            .unwrap()
            .unwrap();
        let target = cancel::run_for(fs.lookup(root, Name::new("TARGET.TXT")), usize::MAX)
            .unwrap()
            .unwrap();
        (fs, root, source, target)
    };
    for rename_budget in 0..80 {
        let (mut baseline, root, _, _) = mount();
        let rename_result = cancel::run_for(
            baseline.rename(
                root,
                Name::new("SOURCE.TXT"),
                root,
                Name::new("TARGET.TXT"),
                RenameMode::Replace,
            ),
            rename_budget,
        );
        cancel::run_for(baseline.sync(), usize::MAX)
            .unwrap()
            .unwrap();
        let committed = cancel::run_for(baseline.lookup(root, Name::new("SOURCE.TXT")), usize::MAX)
            .unwrap()
            .is_err();
        for recovery_budget in 0..80 {
            let (mut fs, root, source, target) = mount();
            cancel::run_for(
                fs.rename(
                    root,
                    Name::new("SOURCE.TXT"),
                    root,
                    Name::new("TARGET.TXT"),
                    RenameMode::Replace,
                ),
                rename_budget,
            );
            let recovery_result = cancel::run_for(fs.sync(), recovery_budget);
            cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
            let old =
                cancel::run_for(fs.lookup(root, Name::new("SOURCE.TXT")), usize::MAX).unwrap();
            assert_eq!(
                old.is_err(),
                committed,
                "rename {rename_budget}, recovery {recovery_budget}"
            );
            let new = cancel::run_for(fs.lookup(root, Name::new("TARGET.TXT")), usize::MAX)
                .unwrap()
                .unwrap();
            assert_eq!(new, if committed { source } else { target });
            if committed {
                assert_eq!(
                    cancel::run_for(fs.stat(target), usize::MAX)
                        .unwrap()
                        .unwrap_err()
                        .kind(),
                    ErrorKind::NotFound
                );
            }
            let image = cancel::run_for(fs.unmount(), usize::MAX)
                .unwrap()
                .unwrap()
                .0
                .into_inner();
            common::assert_checks_clean(case, &image, "identical empty replacement recovery");
            if let Some(result) = recovery_result {
                result.unwrap();
                break;
            }
            assert!(recovery_budget < 79);
        }
        if let Some(result) = rename_result {
            result.unwrap();
            return;
        }
    }
    panic!("empty replacement never completed");
}

#[test]
fn cancelled_nonempty_rename_at_budget_five_does_not_leave_cross_links() {
    use hadris_fat::r#async::FatFs;
    use hadris_fs::sync::FileSystem as _;
    let case = CASES[2];
    let mut original = common::formatted(case, hadris_fat::FatOptions::new());
    let file = original
        .create(original.root(), Name::new("OLD.TXT"), &SetAttr::new())
        .unwrap();
    original.write(file, 0, b"preserved").unwrap();
    let image = original.unmount().unwrap().into_inner();
    let mut fs = cancel::run_for(
        FatFs::mount(
            cancel::YieldDev(common::device(case, image)),
            MountOptions::new(),
        ),
        usize::MAX,
    )
    .unwrap()
    .unwrap()
    .with_cache(hadris_fat::CacheOptions::new());
    let root = fs.root();
    let source = cancel::run_for(fs.lookup(root, Name::new("OLD.TXT")), usize::MAX)
        .unwrap()
        .unwrap();
    assert!(
        cancel::run_for(
            fs.rename(
                root,
                Name::new("OLD.TXT"),
                root,
                Name::new("A replacement long name.txt"),
                RenameMode::NoReplace
            ),
            5
        )
        .is_none()
    );
    cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
    let mut buf = [0; 9];
    cancel::run_for(fs.read(source, 0, &mut buf), usize::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"preserved");
    let image = cancel::run_for(fs.unmount(), usize::MAX)
        .unwrap()
        .unwrap()
        .0
        .into_inner();
    common::assert_checks_clean(case, &image, "cancelled nonempty rename budget five");
}
