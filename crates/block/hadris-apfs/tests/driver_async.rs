#![cfg(all(feature = "read", feature = "async", feature = "alloc"))]

mod common;

use common::*;
use hadris_apfs::{VolumeSelector, async_::ApfsFs};
use hadris_fs::{DirCursor, ErrorKind, FileType, MountOptions, Name, OpenMode};
use hadris_storage::{BlockSize, MemDevice};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct ThreadWaker(std::thread::Thread);
impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
fn device(image: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(image, BlockSize::new(BLOCK as u32).unwrap())
}
fn require_send<T: Send>(value: T) -> T {
    value
}

#[test]
fn async_contracts_and_send_futures_match_sync() {
    block_on(require_send(async {
        for image in [build_image(), build_image_variant(Variant::Sealed)] {
            let mut fs = ApfsFs::mount(device(image), MountOptions::new())
                .await
                .unwrap();
            hadris_fs::async_::contract::check_read_only(&mut fs)
                .await
                .unwrap();
        }
    }));
}

#[test]
fn async_vfs_hardlinks_symlinks_sparse_reads_and_cursors() {
    block_on(require_send(async {
        let mut fs = ApfsFs::mount(device(use_case_image()), MountOptions::new())
            .await
            .unwrap();
        let root = fs.root();
        let original = fs.lookup(root, Name::new("file0.txt")).await.unwrap();
        let alias = fs.lookup(root, Name::new("alias.txt")).await.unwrap();
        assert_eq!(alias, original);
        assert_eq!(fs.stat(alias).await.unwrap().nlink(), 2);
        fs.open(alias, OpenMode::Read).await.unwrap();
        fs.forget(alias, u64::MAX);
        let mut buf = [0; 64];
        assert_eq!(fs.read(alias, 4, &mut buf[..4]).await.unwrap(), 4);
        assert_eq!(&buf[..4], b"ents");
        fs.close(alias).await.unwrap();
        let link = fs.lookup(root, Name::new("link")).await.unwrap();
        assert_eq!(fs.stat(link).await.unwrap().file_type(), FileType::Symlink);
        assert_eq!(fs.readlink(link, &mut buf).await.unwrap(), b"file0.txt");
        let sparse = fs.lookup(root, Name::new("sparse.txt")).await.unwrap();
        let expected = holey_contents(0);
        let mut out = vec![0xff; expected.len() + 64];
        let n = fs.read(sparse, 0, &mut out).await.unwrap();
        assert_eq!(&out[..n], expected);
        assert_eq!(fs.read(sparse, u64::MAX, &mut buf).await.unwrap(), 0);
        assert_eq!(
            fs.write(sparse, 0, b"new").await.unwrap_err().kind(),
            ErrorKind::ReadOnly
        );
        let mut cursor = DirCursor::START;
        let mut count = 0;
        while let Some(entry) = fs.readdir(root, cursor).await.unwrap() {
            let resumed = entry.next_cursor();
            assert_ne!(resumed, cursor);
            cursor = resumed;
            count += 1;
        }
        assert_eq!(count, 9);
        assert!(fs.readdir(root, cursor).await.unwrap().is_none());
    }));
}

#[test]
fn async_multi_volume_mount_and_failure_recovery() {
    block_on(require_send(async {
        let image = multi_volume_image();
        let Err(error) = ApfsFs::mount(device(image.clone()), MountOptions::new()).await else {
            panic!("an ambiguous container mounted");
        };
        assert_eq!(error.into_device().into_inner(), image);
        let mut fs = ApfsFs::mount_volume(
            device(image),
            MountOptions::new(),
            VolumeSelector::Name("Other"),
        )
        .await
        .unwrap();
        let root = fs.root();
        assert!(fs.lookup(root, Name::new("file5.txt")).await.is_ok());
    }));
}

#[test]
fn async_damaged_records_list_and_fail_only_their_own_node() {
    block_on(require_send(async {
        for case in [
            "dangling",
            "type",
            "extent",
            "mode",
            "inode",
            "symlink_stream",
            "duplicate",
            "utf8",
            "long",
        ] {
            let mut fs = ApfsFs::mount(device(invalid_directory_image(case)), MountOptions::new())
                .await
                .unwrap();
            let root = fs.root();
            let mut cursor = DirCursor::START;
            let mut nodes = Vec::new();
            while let Some(entry) = fs.readdir(root, cursor).await.unwrap() {
                cursor = entry.next_cursor();
                nodes.push(entry.node());
            }
            let damaged = matches!(case, "dangling" | "type" | "extent" | "mode" | "inode");
            for index in 2..FILE_COUNT {
                let node = fs.lookup(root, Name::new(&file_name(index))).await.unwrap();
                assert!(nodes.contains(&node), "{case}");
                let mut buf = [0; 64];
                let n = fs.read(node, 0, &mut buf).await.unwrap();
                assert_eq!(&buf[..n], file_contents(index));
            }
            if damaged {
                let node = fs.lookup(root, Name::new("file0.txt")).await.unwrap();
                assert_eq!(fs.stat(node).await.unwrap_err().kind(), ErrorKind::Corrupt);
            }
        }
    }));
}
