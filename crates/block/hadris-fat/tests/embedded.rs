//! The embedded `Fat`: reading what `FatFs` wrote, writing what `FatFs`,
//! `check` and the host `fsck` accept, handles, the fold option, sync and
//! async parity, and dropped async futures.

#[path = "common/fatfs.rs"]
mod common;
#[path = "common/cancel.rs"]
mod shared;

use std::ops::ControlFlow;

use common::{CASES, Case, INNER_FILES, LONG_NAME, UNICODE_NAME, block_on};
use hadris_fat::embedded::sync::Fat;
use hadris_fat::embedded::{Dir, Options};
use hadris_fat_raw::fold_unicode;
use hadris_fs::{
    Attributes, DateTime, DirCursor, ErrorKind, FileType, OpenOptions, SeekFrom, SetAttr,
};
use hadris_storage::{BlockSize, MemDevice};

type Dev = MemDevice<Vec<u8>>;

fn cases() -> impl Iterator<Item = Case> {
    CASES.into_iter().filter(|case| case.block == 512)
}

fn mount(case: Case, image: Vec<u8>) -> Fat<Dev> {
    Fat::mount(common::device(case, image)).unwrap()
}

fn names<const N: usize>(fat: &mut Fat<Dev, N>, dir: Dir) -> Vec<String> {
    let mut out = Vec::new();
    fat.list(dir, DirCursor::START, |entry| {
        out.push(entry.chars().collect());
        ControlFlow::Continue(())
    })
    .unwrap();
    out
}

fn read_file<const N: usize>(fat: &mut Fat<Dev, N>, dir: Dir, name: &str) -> Vec<u8> {
    let file = fat.open(dir, name, OpenOptions::new().read()).unwrap();
    let mut out = Vec::new();
    let mut chunk = [0u8; 700];
    loop {
        let n = fat.read(&file, &mut chunk).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..n]);
    }
    fat.close(file).unwrap();
    out
}

fn write_file<const N: usize>(fat: &mut Fat<Dev, N>, dir: Dir, name: &str, data: &[u8]) {
    let file = fat
        .open(dir, name, OpenOptions::new().write().create().truncate())
        .unwrap();
    for chunk in data.chunks(1000) {
        assert_eq!(fat.write(&file, chunk).unwrap(), chunk.len());
    }
    fat.close(file).unwrap();
}

fn image(fat: Fat<Dev>) -> Vec<u8> {
    fat.unmount().unwrap().into_inner()
}

#[test]
fn reads_what_fatfs_wrote() {
    for case in cases() {
        let built = common::build(case);
        let mut fs = common::mount(case, &built);
        let mut fat = mount(case, built.clone());
        let root = fat.root();
        assert_eq!(
            names(&mut fat, root),
            common::names(&mut fs, "/"),
            "{}",
            case.name
        );
        for name in [
            "README.TXT",
            LONG_NAME,
            UNICODE_NAME,
            "frag.bin",
            "empty.dat",
        ] {
            assert_eq!(
                read_file(&mut fat, root, name),
                common::read(&mut fs, &format!("/{name}")),
                "{}: {name}",
                case.name
            );
        }
        assert_eq!(read_file(&mut fat, root, "readme.txt"), b"hello fat");
        let nested = fat.open_dir(root, "nested dir").unwrap();
        let inner = fat.open_dir(nested, "INNER").unwrap();
        assert_eq!(names(&mut fat, inner).len(), INNER_FILES);
        assert_eq!(
            read_file(&mut fat, inner, "deep.bin"),
            common::read(&mut fs, "/Nested Dir/inner/deep.bin")
        );
        assert_eq!(fat.open_dir(inner, "..").unwrap(), nested);
        assert_eq!(fat.open_dir(nested, "..").unwrap(), root);
        let mut buf = [0u8; 16];
        assert_eq!(fat.label(&mut buf).unwrap(), Some("HADRIS"));
        let meta = fat.metadata(root, "hidden.sys").unwrap();
        assert!(meta.attributes().contains(Attributes::HIDDEN));
        assert_eq!(
            fat.open_dir(root, "README.TXT").unwrap_err().kind(),
            ErrorKind::NotADirectory
        );
        assert_eq!(
            fat.open(root, "missing", OpenOptions::new().read())
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        let stats = fat.stats().unwrap();
        assert!(stats.free_blocks() < stats.total_blocks());
    }
}

#[test]
fn lists_from_a_cursor_and_opens_listed_nodes() {
    let case = CASES[0];
    let mut fat = mount(case, common::build(case));
    let root = fat.root();
    let all = names(&mut fat, root);
    let mut seen = Vec::new();
    let mut cursor = DirCursor::START;
    loop {
        let mut next = None;
        fat.list(root, cursor, |entry| {
            seen.push(entry.chars().collect::<String>());
            next = Some((entry.next_cursor(), entry.node(), entry.file_type()));
            ControlFlow::Break(())
        })
        .unwrap();
        let Some((after, node, kind)) = next else {
            break;
        };
        if kind == FileType::File {
            let file = fat.open_node(node, OpenOptions::new().read()).unwrap();
            fat.close(file).unwrap();
        }
        cursor = after;
    }
    assert_eq!(seen, all);
}

#[test]
fn writes_what_fatfs_check_and_fsck_accept() {
    for case in cases() {
        let mut fat = mount(case, common::blank(case));
        let root = fat.root();
        let deep = fat.create_dir_all(root, "a/b/c").unwrap();
        assert_eq!(fat.create_dir_all(root, "/a//b/./c/").unwrap(), deep);
        let big = common::payload(70_000, 9);
        write_file(&mut fat, deep, "big file.bin", &big);
        write_file(&mut fat, root, "SHORT.TXT", b"short");
        write_file(&mut fat, root, "mixed Case name.txt", b"long");

        let log = fat
            .open(
                root,
                "log.txt",
                OpenOptions::new().write().create().append(),
            )
            .unwrap();
        fat.write(&log, b"one ").unwrap();
        fat.write(&log, b"two").unwrap();
        fat.close(log).unwrap();

        let file = fat
            .open(
                root,
                "sized.bin",
                OpenOptions::new().read().write().create(),
            )
            .unwrap();
        fat.set_len(&file, 9000).unwrap();
        fat.seek(&file, SeekFrom::Start(8000)).unwrap();
        fat.write(&file, b"end").unwrap();
        fat.set_len(&file, 8003).unwrap();
        assert_eq!(fat.seek(&file, SeekFrom::End(0)).unwrap(), 8003);
        fat.seek(&file, SeekFrom::Start(20_000)).unwrap();
        fat.write(&file, b"far").unwrap();
        fat.close(file).unwrap();

        let b = fat.open_dir(fat.root(), "a").unwrap();
        let b = fat.open_dir(b, "b").unwrap();
        fat.rename(b, "c", root, "moved c").unwrap();
        fat.rename(root, "SHORT.TXT", root, "short.txt").unwrap();
        fat.create_dir(root, "gone").unwrap();
        fat.remove_dir(root, "gone").unwrap();
        write_file(&mut fat, root, "temp.txt", b"temp");
        fat.remove_file(root, "temp.txt").unwrap();
        fat.set_attr(
            root,
            "short.txt",
            &SetAttr::new().with_attributes(Attributes::HIDDEN),
        )
        .unwrap();
        let bytes = image(fat);

        common::assert_checks_clean(case, &bytes, case.name);
        common::fsck(&bytes, case.name);
        let mut fs = common::mount(case, &bytes);
        assert_eq!(common::read(&mut fs, "/moved c/big file.bin"), big);
        assert_eq!(common::read(&mut fs, "/log.txt"), b"one two");
        assert_eq!(common::read(&mut fs, "/short.txt"), b"short");
        let sized = common::read(&mut fs, "/sized.bin");
        assert_eq!(sized.len(), 20_003);
        assert_eq!(&sized[8000..8003], b"end");
        assert!(sized[8003..20_000].iter().all(|&b| b == 0));
        assert_eq!(common::names(&mut fs, "/a/b"), Vec::<String>::new());
        assert!(!common::names(&mut fs, "/").contains(&"gone".to_owned()));
        let names = common::names(&mut fs, "/");
        assert!(names.contains(&"short.txt".to_owned()), "{names:?}");
        assert!(names.contains(&"mixed Case name.txt".to_owned()));
    }
}

#[test]
fn remove_dir_all_empties_a_tree() {
    let case = CASES[2];
    let mut fat = mount(case, common::blank(case));
    let root = fat.root();
    let free = fat.stats().unwrap().free_blocks();
    let mut dir = fat.create_dir(root, "top").unwrap();
    for level in 0..20 {
        for i in 0..3 {
            write_file(
                &mut fat,
                dir,
                &format!("file {i}.bin"),
                &common::payload(3000, i),
            );
        }
        fat.create_dir(dir, "empty").unwrap();
        dir = fat.create_dir(dir, &format!("level {level}")).unwrap();
    }
    fat.remove_dir_all(root, "top").unwrap();
    assert_eq!(fat.stats().unwrap().free_blocks(), free);
    assert_eq!(names(&mut fat, root), Vec::<String>::new());
    let bytes = image(fat);
    common::assert_checks_clean(case, &bytes, "remove_dir_all");
    common::fsck(&bytes, "remove_dir_all");
}

#[test]
fn directory_rules() {
    let case = CASES[1];
    let mut fat = mount(case, common::blank(case));
    let root = fat.root();
    let dir = fat.create_dir(root, "dir").unwrap();
    write_file(&mut fat, dir, "inside", b"x");
    let kind = |result: Result<(), hadris_fs::Error<_>>| result.unwrap_err().kind();
    assert_eq!(
        kind(fat.remove_dir(root, "dir")),
        ErrorKind::DirectoryNotEmpty
    );
    assert_eq!(kind(fat.remove_file(root, "dir")), ErrorKind::IsADirectory);
    assert_eq!(
        kind(fat.remove_dir(dir, "inside")),
        ErrorKind::NotADirectory
    );
    assert_eq!(
        fat.create_dir(root, "DIR").unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );
    assert_eq!(
        fat.create_dir(root, "bad?name").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(fat.rename(root, "dir", dir, "self")),
        ErrorKind::InvalidInput
    );
    write_file(&mut fat, root, "other", b"y");
    assert_eq!(
        kind(fat.rename(root, "other", root, "DIR")),
        ErrorKind::AlreadyExists
    );
    let open = fat.open(dir, "inside", OpenOptions::new().read()).unwrap();
    assert_eq!(kind(fat.remove_file(dir, "inside")), ErrorKind::Busy);
    fat.close(open).unwrap();
    fat.remove_file(dir, "inside").unwrap();
    fat.remove_dir(root, "dir").unwrap();
}

#[test]
fn handles_are_checked() {
    let case = CASES[0];
    let mut one = mount(case, common::blank(case));
    let mut two = mount(case, common::blank(case));
    let root = one.root();
    write_file(&mut one, root, "a", b"a");
    write_file(&mut two, root, "a", b"a");
    let file = one.open(root, "a", OpenOptions::new().read()).unwrap();
    let other = two.open(root, "a", OpenOptions::new().read()).unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(
        two.read(&file, &mut buf).unwrap_err().kind(),
        ErrorKind::InvalidHandle
    );
    assert_eq!(
        one.write(&file, b"x").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    one.close(file).unwrap();
    two.close(other).unwrap();

    let mut small: Fat<Dev, 2> = Fat::mount(common::device(case, common::blank(case))).unwrap();
    let root = small.root();
    let create = OpenOptions::new().write().create();
    let first = small.open(root, "first", create).unwrap();
    small.write(&first, &common::payload(5000, 1)).unwrap();
    drop(first);
    let second = small.open(root, "second", create).unwrap();
    assert_eq!(
        small.open(root, "third", create).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    small.close(second).unwrap();
    let bytes = small.unmount().unwrap().into_inner();
    let mut fs = common::mount(case, &bytes);
    assert_eq!(common::read(&mut fs, "/first"), common::payload(5000, 1));
    common::assert_checks_clean(case, &bytes, "dropped handle");
}

#[test]
fn unicode_folding_is_opt_in() {
    let case = CASES[0];
    let mut fat = mount(case, common::blank(case));
    let root = fat.root();
    write_file(&mut fat, root, "\u{E9}t\u{E9}.txt", b"summer");
    assert_eq!(
        fat.metadata(root, "\u{C9}T\u{C9}.TXT").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    let bytes = image(fat);
    let options = Options::new().with_fold(fold_unicode);
    let mut fat: Fat<Dev> = Fat::mount_with(common::device(case, bytes), options).unwrap();
    let root = fat.root();
    assert_eq!(read_file(&mut fat, root, "\u{C9}T\u{C9}.TXT"), b"summer");
}

#[test]
fn refuses_other_block_sizes_and_honours_read_only() {
    let case = CASES[4];
    let err = Fat::<Dev>::mount(common::device(case, common::blank(case))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);

    let case = CASES[0];
    let options = Options::new().read_only();
    let mut fat: Fat<Dev> =
        Fat::mount_with(common::device(case, common::build(case)), options).unwrap();
    let root = fat.root();
    assert_eq!(
        fat.create_dir(root, "x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(
        fat.open(root, "README.TXT", OpenOptions::new().write())
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );
    assert_eq!(read_file(&mut fat, root, "README.TXT"), b"hello fat");
}

fn fixed_clock() -> DateTime {
    DateTime::from_unix_seconds(1_700_000_000).unwrap()
}

/// The same operations in both modes give the same image.
#[test]
fn async_matches_sync() {
    use hadris_fat::embedded::r#async::Fat as AsyncFat;

    let case = CASES[2];
    let options = Options::new().with_clock(fixed_clock);
    let data = common::payload(20_000, 7);

    let mut fat: Fat<Dev> =
        Fat::mount_with(common::device(case, common::blank(case)), options).unwrap();
    let root = fat.root();
    let dir = fat.create_dir_all(root, "x/y").unwrap();
    write_file(&mut fat, dir, "data.bin", &data);
    fat.rename(dir, "data.bin", root, "Data File.bin").unwrap();
    fat.remove_dir_all(root, "x").unwrap();
    let sync = image(fat);

    let asynced = block_on(async {
        let mut fat: AsyncFat<Dev> =
            AsyncFat::mount_with(common::device(case, common::blank(case)), options)
                .await
                .unwrap();
        let root = fat.root();
        let dir = fat.create_dir_all(root, "x/y").await.unwrap();
        let file = fat
            .open(dir, "data.bin", OpenOptions::new().write().create())
            .await
            .unwrap();
        for chunk in data.chunks(1000) {
            fat.write(&file, chunk).await.unwrap();
        }
        fat.close(file).await.unwrap();
        fat.rename(dir, "data.bin", root, "Data File.bin")
            .await
            .unwrap();
        fat.remove_dir_all(root, "x").await.unwrap();
        fat.unmount().await.unwrap().into_inner()
    });
    assert_eq!(sync, asynced);
}

#[test]
fn state_and_futures_stay_small() {
    use hadris_fat::embedded::r#async::Fat as AsyncFat;

    assert!(size_of::<Fat<(), 4>>() < 2048);
    assert!(size_of::<hadris_fat::embedded::r#async::Fat<(), 4>>() < 2048);
    let case = CASES[0];
    let empty = || MemDevice::new(Vec::new(), BlockSize::new(512).unwrap());
    let mount = AsyncFat::<Dev>::mount(empty());
    assert!(size_of_val(&mount) < 2048, "mount {}", size_of_val(&mount));
    let mut fat: AsyncFat<Dev> =
        block_on(AsyncFat::mount(common::device(case, common::blank(case)))).unwrap();
    let root = fat.root();
    let create = fat.create_dir(root, "a long directory name");
    assert!(
        size_of_val(&create) < 3072,
        "create_dir {}",
        size_of_val(&create)
    );
    drop(create);
    let rename = fat.rename(root, "a", root, "b");
    assert!(
        size_of_val(&rename) < 3072,
        "rename {}",
        size_of_val(&rename)
    );
}

mod cancel {
    use super::*;
    use core::future::Future;
    use hadris_fat::embedded::r#async::Fat as AsyncFat;
    use hadris_io::{Error, ErrorType};
    use hadris_storage::BlockIndex;
    use hadris_storage::local::BlockDevice;

    use super::shared::{Rng, run_for};

    /// A memory device whose transfers each yield once.
    struct Yielding(Dev);

    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = ();

        fn poll(
            mut self: core::pin::Pin<&mut Self>,
            cx: &mut core::task::Context<'_>,
        ) -> core::task::Poll<()> {
            if self.0 {
                return core::task::Poll::Ready(());
            }
            self.0 = true;
            cx.waker().wake_by_ref();
            core::task::Poll::Pending
        }
    }

    impl ErrorType for Yielding {
        type Error = core::convert::Infallible;
    }

    impl BlockDevice for Yielding {
        fn block_size(&self) -> BlockSize {
            BlockDevice::block_size(&self.0)
        }

        fn block_count(&self) -> u64 {
            BlockDevice::block_count(&self.0)
        }

        fn writable(&self) -> bool {
            BlockDevice::writable(&self.0)
        }

        async fn read_blocks(
            &mut self,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Result<(), Error<Self::Error>> {
            YieldOnce(false).await;
            BlockDevice::read_blocks(&mut self.0, first, buf).await
        }

        async fn write_blocks(
            &mut self,
            first: BlockIndex,
            buf: &[u8],
        ) -> Result<(), Error<Self::Error>> {
            YieldOnce(false).await;
            BlockDevice::write_blocks(&mut self.0, first, buf).await
        }
    }

    type Vol = AsyncFat<Yielding, 16>;

    async fn step(fat: &mut Vol, kind: u64, i: u64) -> Result<(), ErrorKind> {
        let root = fat.root();
        let dir_name = format!("directory {}", i % 3);
        let file = format!("file number {}.bin", i % 5);
        let dir = match fat.open_dir(root, &dir_name).await {
            Ok(dir) => dir,
            Err(_) if kind == 0 => fat
                .create_dir(root, &dir_name)
                .await
                .map_err(|err| err.kind())?,
            Err(err) => return Err(err.kind()),
        };
        let data: Vec<u8> = (0..(i as usize % 7) * 1500 + 700)
            .map(|b| (b as u8).wrapping_add(i as u8))
            .collect();
        match kind {
            0 | 1 => {
                let options = OpenOptions::new().write().create();
                let options = if kind == 1 {
                    options.append()
                } else {
                    options.truncate()
                };
                let handle = fat
                    .open(dir, &file, options)
                    .await
                    .map_err(|err| err.kind())?;
                fat.write(&handle, &data).await.map_err(|err| err.kind())?;
                fat.close(handle).await.map_err(|err| err.kind())
            }
            2 => {
                let handle = fat
                    .open(dir, &file, OpenOptions::new().write())
                    .await
                    .map_err(|err| err.kind())?;
                fat.set_len(&handle, (i % 4) * 900)
                    .await
                    .map_err(|err| err.kind())?;
                fat.close(handle).await.map_err(|err| err.kind())
            }
            3 => fat
                .rename(dir, &file, root, &format!("moved {i}.bin"))
                .await
                .map_err(|err| err.kind()),
            4 => fat.remove_file(dir, &file).await.map_err(|err| err.kind()),
            _ => fat
                .remove_dir_all(root, &dir_name)
                .await
                .map_err(|err| err.kind()),
        }
    }

    /// Every future dropped at a random await point leaves a volume that
    /// `check` and `fsck` find clean after the next `sync`.
    #[test]
    fn dropped_futures_leave_a_clean_volume() {
        for case in [CASES[0], CASES[2]] {
            let dev = Yielding(common::device(case, common::blank(case)));
            let mut fat: Vol = block_on(AsyncFat::mount(dev)).unwrap();
            let mut rng = Rng(0x5eed ^ case.size);
            for i in 0..400 {
                let kind = rng.below(6);
                let budget = rng.below(160) as usize;
                let _ = run_for(step(&mut fat, kind, i), budget);
                if i % 20 == 19 {
                    let dev = block_on(fat.unmount()).unwrap();
                    fat = block_on(AsyncFat::mount(dev)).unwrap();
                }
            }
            let bytes = block_on(fat.unmount()).unwrap().0.into_inner();
            common::assert_checks_clean(case, &bytes, case.name);
            common::fsck(&bytes, case.name);
        }
    }
}

#[test]
fn files_see_one_size() {
    let case = CASES[0];
    let mut fat = mount(case, common::blank(case));
    let root = fat.root();
    let writer = fat
        .open(root, "shared", OpenOptions::new().write().create())
        .unwrap();
    let reader = fat.open(root, "shared", OpenOptions::new().read()).unwrap();
    fat.write(&writer, b"hello").unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(fat.read(&reader, &mut buf).unwrap(), 5);
    assert_eq!(&buf[..5], b"hello");
    assert_eq!(fat.metadata(root, "shared").unwrap().len(), 5);
    fat.close(writer).unwrap();
    fat.close(reader).unwrap();
}

mod interrupted {
    use super::*;
    use hadris_fat::Detail;
    use hadris_io::Error;
    use hadris_storage::BlockIndex;
    use hadris_storage::sync::BlockDevice;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A device that fails writes after `budget` writes.
    struct Faulty {
        inner: Dev,
        budget: Rc<Cell<usize>>,
    }

    impl hadris_io::ErrorType for Faulty {
        type Error = std::io::Error;
    }

    impl BlockDevice for Faulty {
        fn block_size(&self) -> BlockSize {
            self.inner.block_size()
        }

        fn block_count(&self) -> u64 {
            self.inner.block_count()
        }

        fn writable(&self) -> bool {
            self.inner.writable()
        }

        fn read_blocks(
            &mut self,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Result<(), Error<Self::Error>> {
            self.inner
                .read_blocks(first, buf)
                .map_err(|err| err.map_device(|never| match never {}))
        }

        fn write_blocks(
            &mut self,
            first: BlockIndex,
            buf: &[u8],
        ) -> Result<(), Error<Self::Error>> {
            if self.budget.get() == 0 {
                return Err(Error::device(std::io::Error::other("cut"), "write failed"));
            }
            self.budget.set(self.budget.get() - 1);
            self.inner
                .write_blocks(first, buf)
                .map_err(|err| err.map_device(|never| match never {}))
        }
    }

    fn run(fat: &mut Fat<Faulty>, op: u32) -> Result<(), hadris_fs::Error<std::io::Error>> {
        let root = fat.root();
        let write = OpenOptions::new().write();
        match op {
            0 => fat.create_dir(root, "a new directory").map(|_| ()),
            1 => fat.remove_file(root, LONG_NAME),
            2 => fat.rename(root, LONG_NAME, root, "Renamed file.txt"),
            3 => {
                let nested = fat.open_dir(root, "Nested Dir")?;
                fat.rename(nested, "inner", root, "Moved inner")
            }
            4 => {
                let file = fat.open(root, "new file.bin", write.create())?;
                fat.write(&file, &[7u8; 20_000])?;
                fat.close(file)
            }
            5 => {
                let file = fat.open(root, "frag.bin", write)?;
                fat.seek(&file, SeekFrom::Start(50_000))?;
                fat.write(&file, &[3u8; 20_000])?;
                fat.close(file)
            }
            6 => {
                let file = fat.open(root, "frag.bin", write)?;
                fat.set_len(&file, 1)?;
                fat.close(file)
            }
            7 => {
                let file = fat.open(root, "frag.bin", write)?;
                fat.set_len(&file, 90_000)?;
                fat.close(file)
            }
            _ => fat.remove_dir_all(root, "Nested Dir"),
        }
    }

    fn findings(case: Case, image: &[u8]) -> Vec<Detail> {
        let mut dev = common::device(case, image.to_vec());
        common::check_dev(&mut dev, 8192)
            .1
            .into_iter()
            .map(|found| found.detail)
            .collect()
    }

    /// Cutting each operation after each of its writes leaves only what a
    /// power cut may leave, and `sync` on the same `Fat` afterwards cleans
    /// up what it can: everything but a rename cut between its two entries
    /// and a torn FAT12 entry.
    #[test]
    fn cut_operations_leave_repairable_volumes() {
        use Detail as K;
        for case in [CASES[0], CASES[2]] {
            let before = common::build(case);
            for op in 0..9 {
                let rename = matches!(op, 2 | 3);
                let allowed: &[Detail] = match op {
                    2 | 3 => &[K::CrossLink, K::OrphanLfn, K::DotEntries, K::LostClusters],
                    4..=7 => &[K::LostClusters, K::SizeMismatch, K::OrphanLfn],
                    _ => &[K::LostClusters, K::OrphanLfn],
                };
                for budget in 0.. {
                    let left = Rc::new(Cell::new(budget));
                    let dev = Faulty {
                        inner: common::device(case, before.clone()),
                        budget: left.clone(),
                    };
                    let mut fat: Fat<Faulty> = Fat::mount(dev).unwrap();
                    let result = run(&mut fat, op);
                    let context = format!("{} op {op} budget {budget}", case.name);
                    let cut = fat.into_inner();
                    let image = cut.inner.into_inner();
                    for detail in findings(case, &image) {
                        assert!(
                            allowed.contains(&detail)
                                || [K::FatCopiesDiffer, K::FreeCount].contains(&detail),
                            "{context}: {detail:?}"
                        );
                    }
                    let left = Rc::new(Cell::new(budget));
                    let dev = Faulty {
                        inner: common::device(case, before.clone()),
                        budget: left.clone(),
                    };
                    let mut fat: Fat<Faulty> = Fat::mount(dev).unwrap();
                    let again = run(&mut fat, op);
                    assert_eq!(again.is_ok(), result.is_ok(), "{context}");
                    left.set(usize::MAX);
                    fat.sync().unwrap();
                    let synced = fat.into_inner().inner.into_inner();
                    // A FAT12 entry that straddles two device blocks takes
                    // two writes, and a cut between them leaves a torn link
                    // that recovery cannot trust.
                    let torn = case.kind == hadris_fat::FatKind::Fat12;
                    if result.is_ok() || !(rename || torn) {
                        assert_eq!(findings(case, &synced), [], "{context} then sync");
                    }
                    if result.is_ok() {
                        break;
                    }
                }
            }
        }
    }
}
