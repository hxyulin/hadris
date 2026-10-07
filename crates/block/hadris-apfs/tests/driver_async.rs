#![cfg(all(feature = "read", feature = "async", feature = "alloc"))]

mod common;

use common::*;
use hadris_apfs::{VolumeSelector, r#async::ApfsFs};
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
            hadris_fs::r#async::contract::check_read_only(&mut fs)
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
fn async_mount_refuses_malformed_directory_and_extent_metadata() {
    block_on(require_send(async {
        for case in ["dangling", "type", "duplicate", "extent"] {
            let image = invalid_directory_image(case);
            let Err(error) = ApfsFs::mount(device(image.clone()), MountOptions::new()).await else {
                panic!("malformed {case} mounted");
            };
            assert_eq!(error.kind(), ErrorKind::Corrupt, "{case}");
            assert_eq!(error.into_device().into_inner(), image);
        }
    }));
}

#[test]
fn async_invalid_utf8_directory_names_are_corrupt_and_return_the_device() {
    block_on(require_send(async {
        let image = invalid_directory_image("utf8");
        let Err(error) = ApfsFs::mount(device(image.clone()), MountOptions::new()).await else {
            panic!("invalid UTF-8 directory name mounted");
        };
        assert_eq!(error.kind(), ErrorKind::Corrupt);
        assert_eq!(error.into_device().into_inner(), image);
    }));
}
