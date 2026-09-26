//! The embedded `ExFat`: reading what `ExFatFs` wrote, contiguous
//! allocations and short valid data lengths, damaged sets, handles, the fold
//! option, and sync and async parity.

#[path = "common/exfat.rs"]
mod common;

use std::ops::ControlFlow;

use common::{FsPaths, Geometry, block_on};
use hadris_fat::exfat::embedded::sync::ExFat;
use hadris_fat::exfat::embedded::{Dir, MountToken, Options};
use hadris_fat_raw::fold_unicode;
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, ErrorKind, FileType, Metadata, OpenOptions, SeekFrom};
use hadris_storage::{BlockSize, MemDevice};

type Dev = common::Device;

fn mount<'m>(token: &'m mut MountToken, image: &[u8]) -> ExFat<'m, Dev> {
    ExFat::mount(common::device(image.to_vec(), 512), token).unwrap()
}

fn names(fat: &mut ExFat<Dev>, dir: Dir) -> Vec<String> {
    let mut out = Vec::new();
    fat.list(dir, DirCursor::START, |entry| {
        out.push(entry.chars().collect());
        ControlFlow::Continue(())
    })
    .unwrap();
    out
}

fn read_all(fat: &mut ExFat<Dev>, dir: Dir, name: &str) -> Vec<u8> {
    let file = fat.open(dir, name, OpenOptions::new().read()).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 700];
    loop {
        let n = fat.read(&file, &mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    fat.close(file).unwrap();
    out
}

fn same(a: &Metadata, b: &Metadata) {
    assert_eq!(a.file_type(), b.file_type());
    assert_eq!(a.len(), b.len());
    assert_eq!(a.permissions(), b.permissions());
    assert_eq!(a.attributes(), b.attributes());
    assert_eq!(a.created(), b.created());
    assert_eq!(a.modified(), b.modified());
    assert_eq!(a.accessed(), b.accessed());
}

/// Compares the tree at `path` with what `ExFatFs` reads.
fn compare(fat: &mut ExFat<Dev>, fs: &mut common::Fs, dir: Dir, path: &str) {
    let listed = names(fat, dir);
    assert_eq!(listed, common::names(fs, path), "{path}");
    for name in listed {
        let full = format!("{}/{name}", path.trim_end_matches('/'));
        let meta = fat.metadata(dir, &name).unwrap();
        same(&meta, &fs.metadata(&full).unwrap());
        if meta.file_type() == FileType::Dir {
            let sub = fat.open_dir(dir, &name).unwrap();
            compare(fat, fs, sub, &full);
        } else {
            assert_eq!(read_all(fat, dir, &name), common::read(fs, &full), "{full}");
        }
    }
}

#[test]
fn reads_what_exfatfs_wrote() {
    let image = common::build();
    let mut fs = common::mount(&image);
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, &image);
    assert!(!fat.was_dirty());
    let root = fat.root();
    compare(&mut fat, &mut fs, root, "/");

    let mut label = [0u8; 64];
    assert_eq!(fat.label(&mut label).unwrap(), Some("Hadris"));
    assert_eq!(
        fat.label(&mut [0u8; 3]).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let stats = fat.stats().unwrap();
    let expected = fs.statfs().unwrap();
    assert_eq!(stats.total_blocks(), expected.total_blocks());
    assert_eq!(stats.free_blocks(), expected.free_blocks());
    assert_eq!(stats.block_size(), expected.block_size());

    let nested = fat.open_dir(root, "nested dir").unwrap();
    assert_eq!(fat.open_dir(nested, ".").unwrap(), nested);
    let inner = fat.open_dir(nested, "INNER").unwrap();
    assert_eq!(names(&mut fat, inner).len(), 301);
    assert_eq!(
        read_all(&mut fat, inner, "deep.bin"),
        common::payload(70_000, 5)
    );
    assert_eq!(read_all(&mut fat, root, "readme.txt"), b"hello exfat");
}

#[test]
fn rules_and_errors() {
    let image = common::build();
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, &image);
    let root = fat.root();
    macro_rules! kind {
        ($result:expr) => {
            $result.map(|_| ()).unwrap_err().kind()
        };
    }
    let nested = fat.open_dir(root, "Nested Dir").unwrap();
    assert_eq!(kind!(fat.open_dir(nested, "..")), ErrorKind::Unsupported);
    assert_eq!(
        kind!(fat.open_dir(root, "README.TXT")),
        ErrorKind::NotADirectory
    );
    assert_eq!(kind!(fat.open_dir(root, "missing")), ErrorKind::NotFound);
    assert_eq!(kind!(fat.open_dir(root, "a/b")), ErrorKind::InvalidInput);
    assert_eq!(kind!(fat.open_dir(root, "")), ErrorKind::InvalidInput);
    assert_eq!(
        kind!(fat.open(root, "Nested Dir", OpenOptions::new().read())),
        ErrorKind::IsADirectory
    );
    for options in [
        OpenOptions::new().write(),
        OpenOptions::new().read().write().create(),
    ] {
        assert_eq!(
            kind!(fat.open(root, "README.TXT", options)),
            ErrorKind::ReadOnly
        );
    }
    assert_eq!(
        kind!(fat.open(root, "README.TXT", OpenOptions::new())),
        ErrorKind::InvalidInput
    );
}

#[test]
fn handles_are_checked() {
    let image = common::build();
    let mut mount_token_0 = MountToken::new();
    let mut fat: ExFat<Dev, 2> =
        ExFat::mount(common::device(image.clone(), 512), &mut mount_token_0).unwrap();
    let root = fat.root();
    let read = OpenOptions::new().read();
    let a = fat.open(root, "frag.bin", read).unwrap();
    let b = fat.open(root, "lower.txt", read).unwrap();
    assert_eq!(
        fat.open(root, "README.TXT", read).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(fat.seek(&a, SeekFrom::End(-4)).unwrap(), 40_096);
    let mut tail = [0u8; 8];
    assert_eq!(fat.read(&a, &mut tail).unwrap(), 4);
    assert_eq!(tail[..4], common::payload(40_000, 4)[39_996..]);
    assert_eq!(fat.seek(&a, SeekFrom::Start(100)).unwrap(), 100);
    assert_eq!(fat.read(&a, &mut tail).unwrap(), 8);
    assert_eq!(tail, common::payload(40_000, 4)[..8]);
    assert_eq!(
        fat.seek(&a, SeekFrom::Current(-200)).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    fat.close(a).unwrap();

    let c = fat.open(root, "README.TXT", read).unwrap();
    let mut mount_token_1 = MountToken::new();
    let mut other: ExFat<Dev, 2> =
        ExFat::mount(common::device(image, 512), &mut mount_token_1).unwrap();
    assert_eq!(
        other.read(&c, &mut tail).unwrap_err().kind(),
        ErrorKind::InvalidHandle
    );
    fat.close(b).unwrap();
    fat.close(c).unwrap();
}

#[test]
fn lists_from_a_cursor_and_opens_listed_nodes() {
    let image = common::build();
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, &image);
    let nested = fat.open_dir(fat.root(), "Nested Dir").unwrap();
    let inner = fat.open_dir(nested, "inner").unwrap();
    let all = names(&mut fat, inner);
    let mut seen = Vec::new();
    let mut cursor = DirCursor::START;
    loop {
        let mut batch = Vec::new();
        fat.list(inner, cursor, |entry| {
            batch.push((entry.chars().collect::<String>(), entry.node(), entry.len()));
            cursor = entry.next_cursor();
            if batch.len() == 7 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .unwrap();
        if batch.is_empty() {
            break;
        }
        seen.extend(batch);
    }
    assert_eq!(
        seen.iter()
            .map(|(name, ..)| name.clone())
            .collect::<Vec<_>>(),
        all
    );
    for (name, node, len) in seen.iter().step_by(37) {
        let file = fat.open_node(*node, OpenOptions::new().read()).unwrap();
        let mut data = vec![0u8; *len as usize];
        assert_eq!(fat.read(&file, &mut data).unwrap(), *len as usize);
        if name != "deep.bin" {
            assert_eq!(data, name.as_bytes());
        }
        fat.close(file).unwrap();
    }
}

#[test]
fn unicode_folding_is_opt_in() {
    let image = common::build();
    let name = "\u{C9}t\u{E9} \u{1F600}.txt";
    let upper = "\u{C9}T\u{C9} \u{1F600}.TXT";
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, &image);
    let root = fat.root();
    assert_eq!(read_all(&mut fat, root, name), b"unicode");
    assert_eq!(
        fat.metadata(root, upper).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    let dev = fat.unmount();
    let mut mount_token_1 = MountToken::new();
    let mut fat: ExFat<Dev> = ExFat::mount_with(
        dev,
        &mut mount_token_1,
        Options::new().with_fold(fold_unicode),
    )
    .unwrap();
    assert_eq!(read_all(&mut fat, root, upper), b"unicode");
}

#[test]
fn contiguous_files_and_short_valid_lengths() {
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    let data = common::payload(10_000, 7);
    let node = common::write(&mut fs, root, "contig.bin", &data);
    fs.forget(node, 1);
    let dir = common::mkdir(&mut fs, root, "cdir");
    for i in 0..40 {
        let text = format!("entry number {i:02} with a name that crosses clusters");
        let node = common::write(&mut fs, dir, &text, b"");
        fs.forget(node, 1);
    }
    fs.forget(dir, 1);
    let node = common::write(&mut fs, root, "vdl.bin", &[0xAA; 1000]);
    fs.truncate(node, 10_000).unwrap();
    fs.forget(node, 1);
    fs.sync().unwrap();
    let mut image = common::image(fs);
    let geo = Geometry::of(&image);
    for name in ["contig.bin", "cdir"] {
        let set = geo.set(&image, geo.root, name);
        geo.unchain(&mut image, &set);
    }

    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, &image);
    let root = fat.root();
    assert_eq!(read_all(&mut fat, root, "contig.bin"), data);
    let cdir = fat.open_dir(root, "cdir").unwrap();
    let listed = names(&mut fat, cdir);
    assert_eq!(listed.len(), 40);
    for name in &listed {
        assert_eq!(read_all(&mut fat, cdir, name), b"");
    }
    let vdl = read_all(&mut fat, root, "vdl.bin");
    assert_eq!(vdl.len(), 10_000);
    assert!(vdl[..1000].iter().all(|&b| b == 0xAA) && vdl[1000..].iter().all(|&b| b == 0));
}

#[test]
fn damaged_sets_are_skipped() {
    let mut image = common::build();
    let geo = Geometry::of(&image);
    let set = geo.set(&image, geo.root, "lower.txt");
    image[set[2] + 2] ^= 0x20;
    let spacer = geo.set(&image, geo.root, "spacer.bin");
    image[spacer[1]] = 0x40;
    geo.reseal(&mut image, &spacer);
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, &image);
    let root = fat.root();
    let listed = names(&mut fat, root);
    assert!(!listed.iter().any(|name| name.ends_with("ower.txt")));
    assert!(!listed.contains(&"spacer.bin".to_owned()));
    assert!(listed.contains(&"README.TXT".to_owned()));
    assert_eq!(
        fat.metadata(root, "lower.txt").unwrap_err().kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn refuses_other_block_sizes_and_non_exfat() {
    let image = common::build();
    let mut mount_token_0 = MountToken::new();
    let err = ExFat::<Dev>::mount(common::device(image, 4096), &mut mount_token_0).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    let mut mount_token_1 = MountToken::new();
    let err = ExFat::<Dev>::mount(common::device(vec![0u8; 1 << 20], 512), &mut mount_token_1)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotRecognized);
}

#[test]
fn async_matches_sync() {
    use hadris_fat::exfat::embedded::r#async::ExFat as AsyncFat;

    let image = common::build();
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, &image);
    let nested = fat.open_dir(fat.root(), "Nested Dir").unwrap();
    let inner = fat.open_dir(nested, "inner").unwrap();
    let sync_names = names(&mut fat, inner);
    let sync_data = read_all(&mut fat, inner, "deep.bin");

    let (async_names, async_data) = block_on(async {
        let mut mount_token_1 = MountToken::new();
        let mut fat: AsyncFat<Dev> =
            AsyncFat::mount(common::device(image.clone(), 512), &mut mount_token_1)
                .await
                .unwrap();
        let root = fat.root();
        let nested = fat.open_dir(root, "Nested Dir").await.unwrap();
        let inner = fat.open_dir(nested, "inner").await.unwrap();
        let mut names = Vec::new();
        fat.list(inner, DirCursor::START, |entry| {
            names.push(entry.chars().collect::<String>());
            ControlFlow::Continue(())
        })
        .await
        .unwrap();
        let file = fat
            .open(inner, "deep.bin", OpenOptions::new().read())
            .await
            .unwrap();
        let mut data = vec![0u8; 80_000];
        let n = fat.read(&file, &mut data).await.unwrap();
        data.truncate(n);
        fat.close(file).unwrap();
        (names, data)
    });
    assert_eq!(sync_names, async_names);
    assert_eq!(sync_data, async_data);
}

#[test]
fn state_and_futures_stay_small() {
    use hadris_fat::exfat::embedded::r#async::ExFat as AsyncFat;

    assert!(size_of::<ExFat<(), 4>>() < 2048);
    assert!(size_of::<AsyncFat<(), 4>>() < 2048);
    let empty = || MemDevice::new(Vec::new(), BlockSize::new(512).unwrap());
    let mut mount_token_0 = MountToken::new();
    let mount = AsyncFat::<Dev>::mount(empty(), &mut mount_token_0);
    assert!(size_of_val(&mount) < 2048, "mount {}", size_of_val(&mount));
    let mut mount_token_1 = MountToken::new();
    let mut fat: AsyncFat<Dev> = block_on(AsyncFat::mount(
        common::device(common::build(), 512),
        &mut mount_token_1,
    ))
    .unwrap();
    let root = fat.root();
    let open = fat.open(root, "README.TXT", OpenOptions::new().read());
    assert!(size_of_val(&open) < 3072, "open {}", size_of_val(&open));
    drop(open);
    let list = fat.list(root, DirCursor::START, |_| ControlFlow::Continue(()));
    assert!(size_of_val(&list) < 3072, "list {}", size_of_val(&list));
}

#[test]
fn mount_identity_survives_moves_and_ignores_disk_serials() {
    let mut tokens = [MountToken::new(), MountToken::new()];
    let [first_token, second_token] = &mut tokens;
    let image_with_serial = |serial| {
        let mut fs = common::mount(&common::build());
        fs.set_volume_serial(serial).unwrap();
        fs.into_inner().into_inner()
    };
    let mut first = mount(first_token, &image_with_serial(0));
    let mut second = mount(second_token, &image_with_serial(4099));
    let a = first
        .open(first.root(), "README.TXT", OpenOptions::new().read())
        .unwrap();
    let b = first
        .open(first.root(), "empty.dat", OpenOptions::new().read())
        .unwrap();
    let foreign = second
        .open(second.root(), "README.TXT", OpenOptions::new().read())
        .unwrap();
    let mut moved = first;
    let mut buf = [0; 16];
    assert_eq!(
        second.read(&a, &mut buf).unwrap_err().kind(),
        ErrorKind::InvalidHandle
    );
    assert_eq!(
        moved.read(&foreign, &mut buf).unwrap_err().kind(),
        ErrorKind::InvalidHandle
    );
    assert_eq!(moved.read(&a, &mut buf).unwrap(), 11);
    assert_eq!(moved.read(&b, &mut buf).unwrap(), 0);
    moved.close(a).unwrap();
    moved.close(b).unwrap();
    second.close(foreign).unwrap();
    let dev = moved.unmount();
    let mut remounted: ExFat<_> = ExFat::mount(dev, first_token).unwrap();
    let file = remounted
        .open(remounted.root(), "README.TXT", OpenOptions::new().read())
        .unwrap();
    assert_eq!(remounted.read(&file, &mut buf).unwrap(), 11);
    remounted.close(file).unwrap();
}
