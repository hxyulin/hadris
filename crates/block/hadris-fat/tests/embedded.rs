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
use hadris_fat::embedded::{Dir, MountToken, Options};
use hadris_fat_raw::fold_unicode;
use hadris_fs::{
    Attributes, DateTime, DirCursor, ErrorKind, FileType, OpenOptions, SeekFrom, SetAttr,
};
use hadris_storage::{BlockSize, MemDevice};

type Dev = MemDevice<Vec<u8>>;

fn cases() -> impl Iterator<Item = Case> {
    CASES.into_iter().filter(|case| case.block == 512)
}

fn mount(token: &mut MountToken, case: Case, image: Vec<u8>) -> Fat<'_, Dev> {
    Fat::mount(common::device(case, image), token).unwrap()
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
        let mut mount_token_0 = MountToken::new();
        let mut fat = mount(&mut mount_token_0, case, built.clone());
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
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, case, common::build(case));
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
        let mut mount_token_0 = MountToken::new();
        let mut fat = mount(&mut mount_token_0, case, common::blank(case));
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
fn file_growth_and_truncation_cover_every_cluster_size() {
    use hadris_fat::FatOptions;

    for sector in [512, 4096] {
        for shift in 9..=15 {
            let cluster = 1usize << shift;
            if cluster < sector as usize {
                continue;
            }
            let case = Case { sector, ..CASES[0] };
            let before =
                common::formatted(case, FatOptions::new().with_cluster_size(cluster as u32))
                    .into_inner()
                    .into_inner();
            let mut token = MountToken::new();
            let mut fs = mount(&mut token, case, before);
            assert_eq!(fs.info().cluster_shift(), shift);
            let root = fs.root();
            let file = fs
                .open(
                    root,
                    "BOUND.BIN",
                    OpenOptions::new().read().write().create(),
                )
                .unwrap();
            let mut expected = Vec::new();
            for len in [
                0,
                1,
                cluster - 1,
                cluster,
                cluster + 1,
                2 * cluster - 1,
                2 * cluster,
                2 * cluster + 1,
                cluster,
                cluster - 1,
            ] {
                fs.set_len(&file, len as u64).unwrap();
                expected.resize(len, 0);
                if len != 0 {
                    fs.seek(&file, SeekFrom::Start((len - 1) as u64)).unwrap();
                    fs.write(&file, &[9]).unwrap();
                    expected[len - 1] = 9;
                }
                fs.seek(&file, SeekFrom::Start(0)).unwrap();
                let mut data = vec![0xCC; len + 17];
                assert_eq!(fs.read(&file, &mut data).unwrap(), len);
                assert_eq!(&data[..len], expected);
                assert_eq!(&data[len..], &[0xCC; 17]);
            }
            let end = 3 * cluster + 7;
            fs.seek(&file, SeekFrom::Start(end as u64)).unwrap();
            fs.write(&file, b"tail").unwrap();
            expected.resize(end, 0);
            expected.extend_from_slice(b"tail");
            fs.seek(&file, SeekFrom::Start(u32::MAX as u64 - 1))
                .unwrap();
            assert_eq!(
                fs.write(&file, &[1, 2]).unwrap_err().kind(),
                ErrorKind::NoSpace
            );
            assert_eq!(
                fs.set_len(&file, u32::MAX as u64).unwrap_err().kind(),
                ErrorKind::NoSpace
            );
            fs.seek(&file, SeekFrom::Start(u32::MAX as u64)).unwrap();
            assert_eq!(
                fs.write(&file, &[1]).unwrap_err().kind(),
                ErrorKind::FileTooLarge
            );
            fs.close(file).unwrap();
            let bytes = fs.unmount().unwrap().into_inner();
            common::assert_checks_clean(
                case,
                &bytes,
                &format!("sector {sector} cluster {cluster}"),
            );
            let mut hosted = common::mount(case, &bytes);
            assert_eq!(common::read(&mut hosted, "/BOUND.BIN"), expected);
        }
    }
}

#[test]
fn remove_dir_all_empties_a_tree() {
    let case = CASES[2];
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, case, common::blank(case));
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
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, case, common::blank(case));
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
    let mut mount_token_0 = MountToken::new();
    let mut one = mount(&mut mount_token_0, case, common::blank(case));
    let mut mount_token_1 = MountToken::new();
    let mut two = mount(&mut mount_token_1, case, common::blank(case));
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

    let mut mount_token_2 = MountToken::new();
    let mut small: Fat<Dev, 2> = Fat::mount(
        common::device(case, common::blank(case)),
        &mut mount_token_2,
    )
    .unwrap();
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
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, case, common::blank(case));
    let root = fat.root();
    write_file(&mut fat, root, "\u{E9}t\u{E9}.txt", b"summer");
    assert_eq!(
        fat.metadata(root, "\u{C9}T\u{C9}.TXT").unwrap_err().kind(),
        ErrorKind::NotFound
    );
    let bytes = image(fat);
    let options = Options::new().with_fold(fold_unicode);
    let mut mount_token_1 = MountToken::new();
    let mut fat: Fat<Dev> =
        Fat::mount_with(common::device(case, bytes), &mut mount_token_1, options).unwrap();
    let root = fat.root();
    assert_eq!(read_file(&mut fat, root, "\u{C9}T\u{C9}.TXT"), b"summer");
}

#[test]
fn refuses_other_block_sizes_and_honours_read_only() {
    let case = CASES[4];
    let mut mount_token_0 = MountToken::new();
    let err = Fat::<Dev>::mount(
        common::device(case, common::blank(case)),
        &mut mount_token_0,
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);

    let case = CASES[0];
    let options = Options::new().read_only();
    let mut mount_token_1 = MountToken::new();
    let mut fat: Fat<Dev> = Fat::mount_with(
        common::device(case, common::build(case)),
        &mut mount_token_1,
        options,
    )
    .unwrap();
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

    let mut mount_token_0 = MountToken::new();
    let mut fat: Fat<Dev> = Fat::mount_with(
        common::device(case, common::blank(case)),
        &mut mount_token_0,
        options,
    )
    .unwrap();
    let root = fat.root();
    let dir = fat.create_dir_all(root, "x/y").unwrap();
    write_file(&mut fat, dir, "data.bin", &data);
    fat.rename(dir, "data.bin", root, "Data File.bin").unwrap();
    fat.remove_dir_all(root, "x").unwrap();
    let sync = image(fat);

    let asynced = block_on(async {
        let mut mount_token_1 = MountToken::new();
        let mut fat: AsyncFat<Dev> = AsyncFat::mount_with(
            common::device(case, common::blank(case)),
            &mut mount_token_1,
            options,
        )
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
#[cfg_attr(
    feature = "tracing",
    ignore = "resource budgets apply without hosted tracing spans"
)]
fn state_and_futures_stay_small() {
    use hadris_fat::embedded::r#async::Fat as AsyncFat;

    assert!(size_of::<Fat<(), 4>>() < 2048);
    assert!(size_of::<hadris_fat::embedded::r#async::Fat<(), 4>>() < 2048);
    let case = CASES[0];
    let empty = || MemDevice::new(Vec::new(), BlockSize::new(512).unwrap());
    let mut mount_token_0 = MountToken::new();
    let mount = AsyncFat::<Dev>::mount(empty(), &mut mount_token_0);
    assert!(size_of_val(&mount) < 2048, "mount {}", size_of_val(&mount));
    let mut mount_token_1 = MountToken::new();
    let mut fat: AsyncFat<Dev> = block_on(AsyncFat::mount(
        common::device(case, common::blank(case)),
        &mut mount_token_1,
    ))
    .unwrap();
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

const RENAME_SOURCE: &str = "original long name.bin";
const RENAME_TARGET: &str = "renamed entry with another long name.bin";

fn rename_fixture(case: Case, directory: bool) -> Vec<u8> {
    let mut token = MountToken::new();
    let mut fat = mount(&mut token, case, common::blank(case));
    let root = fat.root();
    let source = fat.create_dir(root, "PARENT_A").unwrap();
    fat.create_dir(root, "PARENT_B").unwrap();
    let parent = if directory {
        fat.create_dir(source, RENAME_SOURCE).unwrap()
    } else {
        source
    };
    let name = if directory {
        "child.bin"
    } else {
        RENAME_SOURCE
    };
    let file = fat
        .open(parent, name, OpenOptions::new().write().create())
        .unwrap();
    fat.write(&file, &common::payload(9000, 17)).unwrap();
    fat.close(file).unwrap();
    fat.unmount().unwrap().into_inner()
}

fn rename_stale_fixture(case: Case, directory: bool, cross: bool) -> Vec<u8> {
    use common::FsPaths;
    let mut image = rename_fixture(case, directory);
    let mut token = MountToken::new();
    let mut fat = mount(&mut token, case, image.clone());
    let source = fat.open_dir(fat.root(), "PARENT_A").unwrap();
    let target = if cross {
        fat.open_dir(fat.root(), "PARENT_B").unwrap()
    } else {
        source
    };
    let file_parent = if directory {
        fat.open_dir(source, RENAME_SOURCE).unwrap()
    } else {
        source
    };
    let file_name = if directory {
        "child.bin"
    } else {
        RENAME_SOURCE
    };
    let file = fat
        .open(file_parent, file_name, OpenOptions::new().write().append())
        .unwrap();
    fat.write(&file, b"before").unwrap();
    fat.close(file).unwrap();
    fat.rename(source, RENAME_SOURCE, target, RENAME_TARGET)
        .unwrap();
    let renamed = fat.unmount().unwrap().into_inner();
    let mut fs = common::mount(case, &renamed);
    let parent = if cross { "PARENT_B" } else { "PARENT_A" };
    let node = fs
        .resolve_path(&format!("/{parent}/{RENAME_TARGET}"))
        .unwrap();
    let mut record = [hadris_fs::Extent::new(0, 0)];
    assert_eq!(fs.records(node, &mut record).unwrap(), 1);
    let at = record[0].offset() as usize;
    let expected = &renamed[at..at + 32];
    let mut fs = common::mount(case, &image);
    let node = fs.resolve_path(parent).unwrap();
    assert_eq!(fs.extents(node, 0, &mut record).unwrap(), 1);
    let first = record[0].offset() as usize;
    let end = (first..first + record[0].len() as usize)
        .step_by(32)
        .find(|&at| image[at] == 0)
        .unwrap();
    let destination = end + RENAME_TARGET.encode_utf16().count().div_ceil(13) * 32;
    image[destination..destination + 32].copy_from_slice(expected);
    image[destination + 32..destination + 64].copy_from_slice(expected);
    image[destination + 32..destination + 43].copy_from_slice(b"HIDDEN  BIN");
    common::assert_checks_clean(case, &image, "hidden stale rename entries");
    image
}

fn check_rename_image(case: Case, image: &[u8], directory: bool, cross: bool, suffix: &[u8]) {
    use common::FsPaths;
    common::assert_checks_clean(case, image, "interrupted embedded rename");
    let mut fs = common::mount(case, image);
    let old = format!("/PARENT_A/{RENAME_SOURCE}");
    let new = format!(
        "/{}/{RENAME_TARGET}",
        if cross { "PARENT_B" } else { "PARENT_A" }
    );
    let source = fs.resolve_path(&old).is_ok();
    let target = fs.resolve_path(&new).is_ok();
    assert_ne!(source, target, "exactly one rename owner must remain");
    let mut path = if target { new } else { old };
    if directory {
        path.push_str("/child.bin");
    }
    let mut expected = common::payload(9000, 17);
    expected.extend_from_slice(suffix);
    assert_eq!(common::read(&mut fs, &path), expected);
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

    type Vol<'m> = AsyncFat<'m, Yielding, 16>;
    #[test]
    fn rename_and_its_recovery_can_be_cancelled_at_every_await() {
        for case in cases() {
            for directory in [false, true] {
                for cross in [false, true] {
                    let image = rename_stale_fixture(case, directory, cross);
                    let mut recovery_cut = None;
                    let mut rollback_cut = None;
                    let mut finished = false;
                    for budget in 0..200 {
                        let mut token = MountToken::new();
                        let mut fat: Vol = block_on(AsyncFat::mount(
                            Yielding(common::device(case, image.clone())),
                            &mut token,
                        ))
                        .unwrap();
                        let source = block_on(fat.open_dir(fat.root(), "PARENT_A")).unwrap();
                        let target = if cross {
                            block_on(fat.open_dir(fat.root(), "PARENT_B")).unwrap()
                        } else {
                            source
                        };
                        let file_parent = if directory {
                            block_on(fat.open_dir(source, RENAME_SOURCE)).unwrap()
                        } else {
                            source
                        };
                        let file_name = if directory {
                            "child.bin"
                        } else {
                            RENAME_SOURCE
                        };
                        let file = block_on(fat.open(
                            file_parent,
                            file_name,
                            OpenOptions::new().read().write().append(),
                        ))
                        .unwrap();
                        block_on(fat.write(&file, b"before")).unwrap();
                        let result = run_for(
                            fat.rename(source, RENAME_SOURCE, target, RENAME_TARGET),
                            budget,
                        );
                        assert_eq!(
                            block_on(fat.metadata(target, "HIDDEN.BIN"))
                                .err()
                                .map(|error| error.kind()),
                            Some(ErrorKind::NotFound),
                            "{} directory={directory} cross={cross} budget={budget}",
                            case.name,
                        );
                        if result.is_none()
                            && block_on(fat.metadata(target, RENAME_TARGET)).is_err()
                        {
                            rollback_cut = Some(budget);
                        }
                        if result.is_none()
                            && block_on(fat.metadata(target, RENAME_TARGET)).is_ok()
                            && block_on(fat.metadata(source, RENAME_SOURCE)).is_ok()
                        {
                            recovery_cut = Some(budget);
                        }
                        block_on(fat.sync()).unwrap();
                        if directory {
                            let moved = block_on(fat.open_dir(target, RENAME_TARGET)).ok();
                            if let Some(moved) = moved {
                                assert_eq!(block_on(fat.open_dir(moved, "..")).unwrap(), target);
                            }
                        }
                        block_on(fat.write(&file, b"after")).unwrap();
                        block_on(fat.close(file)).unwrap();
                        let image = block_on(fat.unmount()).unwrap().0.into_inner();
                        check_rename_image(case, &image, directory, cross, b"beforeafter");
                        if let Some(result) = result {
                            result.unwrap();
                            finished = true;
                            break;
                        }
                    }
                    assert!(finished);
                    let recovery_cut =
                        recovery_cut.expect("must cancel after destination publication");
                    for recovery_cut in
                        [rollback_cut.expect("must interrupt rollback"), recovery_cut]
                    {
                        let mut finished = false;
                        for budget in 0..200 {
                            let mut token = MountToken::new();
                            let mut fat: Vol = block_on(AsyncFat::mount(
                                Yielding(common::device(case, image.clone())),
                                &mut token,
                            ))
                            .unwrap();
                            let source = block_on(fat.open_dir(fat.root(), "PARENT_A")).unwrap();
                            let target = if cross {
                                block_on(fat.open_dir(fat.root(), "PARENT_B")).unwrap()
                            } else {
                                source
                            };
                            let file_parent = if directory {
                                block_on(fat.open_dir(source, RENAME_SOURCE)).unwrap()
                            } else {
                                source
                            };
                            let file_name = if directory {
                                "child.bin"
                            } else {
                                RENAME_SOURCE
                            };
                            let file = block_on(fat.open(
                                file_parent,
                                file_name,
                                OpenOptions::new().read().write().append(),
                            ))
                            .unwrap();
                            block_on(fat.write(&file, b"before")).unwrap();
                            assert!(
                                run_for(
                                    fat.rename(source, RENAME_SOURCE, target, RENAME_TARGET),
                                    recovery_cut
                                )
                                .is_none()
                            );
                            let result = run_for(fat.sync(), budget);
                            block_on(fat.sync()).unwrap();
                            block_on(fat.write(&file, b"after")).unwrap();
                            block_on(fat.close(file)).unwrap();
                            let image = block_on(fat.unmount()).unwrap().0.into_inner();
                            check_rename_image(case, &image, directory, cross, b"beforeafter");
                            if let Some(result) = result {
                                result.unwrap();
                                finished = true;
                                break;
                            }
                        }
                        assert!(finished, "recovery must complete");
                    }
                }
            }
        }
    }

    async fn step(fat: &mut Vol<'_>, kind: u64, i: u64) -> Result<(), ErrorKind> {
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
            let mut mount_token_2 = MountToken::new();
            let mut fat: Vol = block_on(AsyncFat::mount(dev, &mut mount_token_2)).unwrap();
            let mut rng = Rng(0x5eed ^ case.size);
            for i in 0..400 {
                let kind = rng.below(6);
                let budget = rng.below(160) as usize;
                let _ = run_for(step(&mut fat, kind, i), budget);
                if i % 20 == 19 {
                    let dev = block_on(fat.unmount()).unwrap();
                    fat = block_on(AsyncFat::mount(dev, &mut mount_token_2)).unwrap();
                }
            }
            let bytes = block_on(fat.unmount()).unwrap().0.into_inner();
            common::assert_checks_clean(case, &bytes, case.name);
            common::fsck(&bytes, case.name);
        }
    }

    #[test]
    fn single_cluster_append_recovers_at_every_await() {
        for case in CASES[..3].iter().copied() {
            let blank = common::blank(case);
            let cluster = hadris_fat_raw::parse_boot(blank[..512].try_into().unwrap())
                .unwrap()
                .cluster_size() as usize;
            let mut token = MountToken::new();
            let mut fs = mount(&mut token, case, blank);
            let root = fs.root();
            write_file(&mut fs, root, "LOG.BIN", &vec![7; cluster]);
            let before = fs.unmount().unwrap().into_inner();
            for budget in 0..100 {
                let mut token = MountToken::new();
                let mut fs: Vol = block_on(AsyncFat::mount(
                    Yielding(common::device(case, before.clone())),
                    &mut token,
                ))
                .unwrap();
                let file =
                    block_on(fs.open(fs.root(), "LOG.BIN", OpenOptions::new().write().append()))
                        .unwrap();
                let result = run_for(fs.write(&file, &[9]), budget);
                block_on(fs.sync()).unwrap();
                block_on(fs.close(file)).unwrap();
                let image = block_on(fs.unmount()).unwrap().0.into_inner();
                common::assert_checks_clean(
                    case,
                    &image,
                    &format!("{} append budget {budget}", case.name),
                );
                let mut token = MountToken::new();
                let mut fs = mount(&mut token, case, image);
                let root = fs.root();
                let data = read_file(&mut fs, root, "LOG.BIN");
                assert_eq!(&data[..cluster], &vec![7; cluster]);
                assert_eq!(data.len(), cluster + usize::from(result.is_some()));
                if let Some(result) = result {
                    assert_eq!(result.unwrap(), 1);
                    assert_eq!(data[cluster], 9);
                    break;
                }
                assert!(budget < 99, "append never completed");
            }
        }
    }
}

#[test]
fn files_see_one_size() {
    let case = CASES[0];
    let mut mount_token_0 = MountToken::new();
    let mut fat = mount(&mut mount_token_0, case, common::blank(case));
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

    #[test]
    fn rename_and_its_recovery_retry_after_every_failed_write() {
        for case in cases() {
            for directory in [false, true] {
                for cross in [false, true] {
                    let image = rename_stale_fixture(case, directory, cross);
                    let mut recovery_cut = None;
                    let mut rollback_cut = None;
                    let mut finished = false;
                    for budget in 0..100 {
                        let left = Rc::new(Cell::new(usize::MAX));
                        let dev = Faulty {
                            inner: common::device(case, image.clone()),
                            budget: left.clone(),
                        };
                        let mut token = MountToken::new();
                        let mut fat: Fat<Faulty, 16> = Fat::mount(dev, &mut token).unwrap();
                        let source = fat.open_dir(fat.root(), "PARENT_A").unwrap();
                        let target = if cross {
                            fat.open_dir(fat.root(), "PARENT_B").unwrap()
                        } else {
                            source
                        };
                        let file_parent = if directory {
                            fat.open_dir(source, RENAME_SOURCE).unwrap()
                        } else {
                            source
                        };
                        let file_name = if directory {
                            "child.bin"
                        } else {
                            RENAME_SOURCE
                        };
                        let file = fat
                            .open(
                                file_parent,
                                file_name,
                                OpenOptions::new().read().write().append(),
                            )
                            .unwrap();
                        fat.write(&file, b"before").unwrap();
                        left.set(budget);
                        let result = fat.rename(source, RENAME_SOURCE, target, RENAME_TARGET);
                        assert_eq!(
                            fat.metadata(target, "HIDDEN.BIN")
                                .err()
                                .map(|error| error.kind()),
                            Some(ErrorKind::NotFound),
                            "{} directory={directory} cross={cross} budget={budget}",
                            case.name,
                        );
                        if result.is_err() && fat.metadata(target, RENAME_TARGET).is_err() {
                            rollback_cut = Some(budget);
                        }
                        if result.is_err()
                            && fat.metadata(target, RENAME_TARGET).is_ok()
                            && fat.metadata(source, RENAME_SOURCE).is_ok()
                        {
                            recovery_cut = Some(budget);
                        }
                        left.set(usize::MAX);
                        fat.sync().unwrap();
                        if directory {
                            if let Ok(moved) = fat.open_dir(target, RENAME_TARGET) {
                                assert_eq!(fat.open_dir(moved, "..").unwrap(), target);
                            }
                        }
                        fat.write(&file, b"after").unwrap();
                        fat.close(file).unwrap();
                        let image = fat.unmount().unwrap().inner.into_inner();
                        check_rename_image(case, &image, directory, cross, b"beforeafter");
                        match result {
                            Ok(()) => {
                                finished = true;
                                break;
                            }
                            Err(error) => assert_eq!(error.kind(), ErrorKind::Io),
                        }
                    }
                    assert!(finished);
                    let recovery_cut =
                        recovery_cut.expect("must fail after destination publication");
                    for recovery_cut in
                        [rollback_cut.expect("must interrupt rollback"), recovery_cut]
                    {
                        let mut finished = false;
                        for budget in 0..100 {
                            let left = Rc::new(Cell::new(usize::MAX));
                            let dev = Faulty {
                                inner: common::device(case, image.clone()),
                                budget: left.clone(),
                            };
                            let mut token = MountToken::new();
                            let mut fat: Fat<Faulty, 16> = Fat::mount(dev, &mut token).unwrap();
                            let source = fat.open_dir(fat.root(), "PARENT_A").unwrap();
                            let target = if cross {
                                fat.open_dir(fat.root(), "PARENT_B").unwrap()
                            } else {
                                source
                            };
                            let file_parent = if directory {
                                fat.open_dir(source, RENAME_SOURCE).unwrap()
                            } else {
                                source
                            };
                            let file_name = if directory {
                                "child.bin"
                            } else {
                                RENAME_SOURCE
                            };
                            let file = fat
                                .open(
                                    file_parent,
                                    file_name,
                                    OpenOptions::new().read().write().append(),
                                )
                                .unwrap();
                            fat.write(&file, b"before").unwrap();
                            left.set(recovery_cut);
                            assert_eq!(
                                fat.rename(source, RENAME_SOURCE, target, RENAME_TARGET)
                                    .unwrap_err()
                                    .kind(),
                                ErrorKind::Io
                            );
                            left.set(budget);
                            let result = fat.sync();
                            left.set(usize::MAX);
                            fat.sync().unwrap();
                            fat.write(&file, b"after").unwrap();
                            fat.close(file).unwrap();
                            let image = fat.unmount().unwrap().inner.into_inner();
                            check_rename_image(case, &image, directory, cross, b"beforeafter");
                            match result {
                                Ok(()) => {
                                    finished = true;
                                    break;
                                }
                                Err(error) => assert_eq!(error.kind(), ErrorKind::Io),
                            }
                        }
                        assert!(finished);
                    }
                }
            }
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
    /// up what it can, except a torn FAT12 entry.
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
                    let mut mount_token_1 = MountToken::new();
                    let mut fat: Fat<Faulty> = Fat::mount(dev, &mut mount_token_1).unwrap();
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
                    let mut mount_token_2 = MountToken::new();
                    let mut fat: Fat<Faulty> = Fat::mount(dev, &mut mount_token_2).unwrap();
                    let again = run(&mut fat, op);
                    assert_eq!(again.is_ok(), result.is_ok(), "{context}");
                    left.set(usize::MAX);
                    fat.sync().unwrap();
                    let synced = fat.into_inner().inner.into_inner();
                    // A FAT12 entry that straddles two device blocks takes
                    // two writes, and a cut between them leaves a torn link
                    // that recovery cannot trust.
                    let torn = case.kind == hadris_fat::FatKind::Fat12;
                    if result.is_ok() || rename || !torn {
                        assert_eq!(findings(case, &synced), [], "{context} then sync");
                    }
                    if result.is_ok() {
                        break;
                    }
                }
            }
        }
    }

    #[test]
    fn single_cluster_append_recovers_after_each_failed_write() {
        for case in CASES[..3].iter().copied() {
            let blank = common::blank(case);
            let cluster = hadris_fat_raw::parse_boot(blank[..512].try_into().unwrap())
                .unwrap()
                .cluster_size() as usize;
            let mut token = MountToken::new();
            let mut fs = mount(&mut token, case, blank);
            let root = fs.root();
            write_file(&mut fs, root, "LOG.BIN", &vec![7; cluster]);
            let before = fs.unmount().unwrap().into_inner();
            for budget in 0..10 {
                let left = Rc::new(Cell::new(usize::MAX));
                let dev = Faulty {
                    inner: common::device(case, before.clone()),
                    budget: left.clone(),
                };
                let mut token = MountToken::new();
                let mut fs: Fat<Faulty> = Fat::mount(dev, &mut token).unwrap();
                let file = fs
                    .open(fs.root(), "LOG.BIN", OpenOptions::new().write().append())
                    .unwrap();
                left.set(budget);
                let result = fs.write(&file, &[9]);
                left.set(usize::MAX);
                fs.sync().unwrap();
                fs.close(file).unwrap();
                let image = fs.unmount().unwrap().inner.into_inner();
                assert_eq!(
                    findings(case, &image),
                    [],
                    "{} append budget {budget}",
                    case.name
                );
                let mut token = MountToken::new();
                let mut fs = mount(&mut token, case, image);
                let root = fs.root();
                let data = read_file(&mut fs, root, "LOG.BIN");
                assert_eq!(&data[..cluster], &vec![7; cluster]);
                assert_eq!(data.len(), cluster + usize::from(result.is_ok()));
                if result.is_ok() {
                    assert_eq!(data[cluster], 9);
                    break;
                }
                assert!(budget < 9, "append never completed");
            }
        }
    }
}

#[test]
fn mount_identity_survives_moves_and_ignores_disk_serials() {
    let case = CASES[0];
    let mut tokens = [MountToken::new(), MountToken::new()];
    let [first_token, second_token] = &mut tokens;
    let image_with_serial = |serial| {
        let mut fs = common::mount(case, &common::build(case));
        fs.set_volume_serial(serial).unwrap();
        fs.into_inner().into_inner()
    };
    let mut first = mount(first_token, case, image_with_serial(0));
    let mut second = mount(second_token, case, image_with_serial(4099));
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
    assert_eq!(moved.read(&a, &mut buf).unwrap(), 9);
    assert_eq!(moved.read(&b, &mut buf).unwrap(), 0);
    moved.close(a).unwrap();
    moved.close(b).unwrap();
    second.close(foreign).unwrap();
    let dev = moved.unmount().unwrap();
    let mut remounted: Fat<_> = Fat::mount(dev, first_token).unwrap();
    let file = remounted
        .open(remounted.root(), "README.TXT", OpenOptions::new().read())
        .unwrap();
    assert_eq!(remounted.read(&file, &mut buf).unwrap(), 9);
    remounted.close(file).unwrap();
}

#[test]
fn unmount_after_a_refusal_fails_while_sizes_are_unwritten() {
    use common::script::Scripted;

    let case = CASES[0];
    let before = common::build(case);
    for dirty in [true, false] {
        let (dev, script) = Scripted::new(common::device(case, before.clone()));
        let mut token = MountToken::new();
        let mut fat: Fat<Scripted> = Fat::mount(dev, &mut token).unwrap();
        let root = fat.root();
        let file = fat
            .open(root, "README.TXT", OpenOptions::new().write())
            .unwrap();
        if dirty {
            fat.seek(&file, SeekFrom::End(0)).unwrap();
            assert_eq!(fat.write(&file, &[1u8; 5000]).unwrap(), 5000);
        }
        script.borrow_mut().refuse = true;
        assert_eq!(
            fat.create_dir(root, "x").unwrap_err().kind(),
            ErrorKind::ReadOnly
        );
        assert!(fat.is_read_only());
        if !dirty {
            fat.close(file).unwrap();
            assert_eq!(fat.sync().map_err(|err| err.kind()), Ok(()));
            fat.unmount().unwrap();
            continue;
        }
        drop(file);
        assert_eq!(fat.sync().unwrap_err().kind(), ErrorKind::ReadOnly);
        let err = fat.unmount().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ReadOnly);
        let image = err.into_device().into_image();
        let mut token = MountToken::new();
        let mut fresh = mount(&mut token, case, image);
        let root = fresh.root();
        assert_eq!(read_file(&mut fresh, root, "README.TXT"), b"hello fat");
    }
    let options = Options::new().read_only();
    let mut token = MountToken::new();
    let fat: Fat<Dev> = Fat::mount_with(common::device(case, before), &mut token, options).unwrap();
    fat.unmount().unwrap();
}

#[test]
fn a_file_may_share_the_label_name() {
    use hadris_fat::{FatOptions, VolumeLabel};
    for case in cases() {
        let options = FatOptions::new().with_label(VolumeLabel::new("MYDISK").unwrap());
        let before = common::formatted(case, options).into_inner().into_inner();
        let mut token = MountToken::new();
        let mut fat = mount(&mut token, case, before);
        let root = fat.root();
        write_file(&mut fat, root, "MYDISK", b"disk");
        assert_eq!(names(&mut fat, root), ["MYDISK"]);
        let mut buf = [0u8; 32];
        assert_eq!(fat.label(&mut buf).unwrap(), Some("MYDISK"));
        let after = image(fat);
        common::assert_checks_clean(case, &after, case.name);
        let mut fs = common::mount(case, &after);
        assert_eq!(common::names(&mut fs, "/"), ["MYDISK"]);
    }
}
