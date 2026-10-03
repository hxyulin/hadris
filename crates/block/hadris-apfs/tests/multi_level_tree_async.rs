#![cfg(all(feature = "async", feature = "alloc"))]

mod common;

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use common::*;
use hadris_apfs::r#async::Container;
use hadris_io::Cursor;
use hadris_storage::r#async::SeekBlockDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

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

fn check(image: &[u8]) {
    block_on(async {
        let geometry = BlockGeometry::new(
            BlockSize::new(BLOCK as u32).unwrap(),
            BlockCount(IMAGE_BLOCKS as u64),
        );
        let device = SeekBlockDevice::new(Cursor::new(image), geometry);
        let mut container = Container::open(device).await.unwrap();
        let superblock = container.superblock().clone();
        let volumes = container.volume_superblocks(&superblock).await.unwrap();
        let volume = &volumes[0];
        let entries = container
            .root_directory_owned_entries(volume)
            .await
            .unwrap();
        assert_eq!(entries.len() as u64, FILE_COUNT);
        for index in 0..FILE_COUNT {
            let entry = container
                .resolve_path(volume, &file_name(index))
                .await
                .unwrap()
                .expect("file present");
            let bytes = container
                .read_file(volume, entry.file_id, usize::MAX)
                .await
                .unwrap();
            assert_eq!(bytes, file_contents(index));
        }
    });
}

#[test]
fn multi_level_filesystem_tree_resolves_virtual_children() {
    check(&build_image());
}

#[test]
fn sealed_volume_tree_with_headerless_hashed_nodes_is_readable() {
    check(&build_image_variant(Variant::Sealed));
}

#[test]
fn encrypted_tree_reports_unsupported() {
    let image = build_image_variant(Variant::Encrypted);
    block_on(async {
        let geometry = BlockGeometry::new(
            BlockSize::new(BLOCK as u32).unwrap(),
            BlockCount(IMAGE_BLOCKS as u64),
        );
        let device = SeekBlockDevice::new(Cursor::new(&image), geometry);
        let mut container = Container::open(device).await.unwrap();
        let superblock = container.superblock().clone();
        let volumes = container.volume_superblocks(&superblock).await.unwrap();
        let error = container
            .root_directory_owned_entries(&volumes[0])
            .await
            .unwrap_err();
        assert_eq!(
            error,
            hadris_apfs::ApfsError::InvalidValue("encrypted B-tree nodes are not supported")
        );
    });
}
