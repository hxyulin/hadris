#![cfg(all(feature = "read", feature = "async", feature = "alloc"))]

mod common;
#[path = "common/failing_device.rs"]
mod failing_device;

use failing_device::FailingDevice;

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use common::*;
use hadris_apfs::r#async::Container;
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
            BlockCount::new(IMAGE_BLOCKS as u64),
        );
        let device = hadris_storage::MemDevice::new(image, geometry.logical_block_size());
        let mut container = Container::open(device).await.unwrap();
        let superblock = container.superblock().clone();
        let volumes = container.volume_superblocks(&superblock).await.unwrap();
        let volume = &volumes[0];
        let entries = container
            .root_directory_owned_entries(volume)
            .await
            .unwrap();
        assert_eq!(entries.len() as u64, FILE_COUNT);
        assert!(
            container
                .resolve_path(volume, "/file0.txt/")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            container
                .resolve_path(volume, "/file0.txt/..")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            container
                .resolve_path(volume, "/./file0.txt")
                .await
                .unwrap()
                .unwrap()
                .file_id,
            container
                .resolve_path(volume, "/file0.txt")
                .await
                .unwrap()
                .unwrap()
                .file_id
        );
        for path in ["", "/", "/.", "/../../"] {
            let root = container.resolve_path(volume, path).await.unwrap().unwrap();
            assert_eq!(
                root.file_id,
                hadris_apfs::types::filesystem::INODE_ROOT_DIRECTORY
            );
            assert_eq!(root.file_type(), hadris_apfs::types::filesystem::DT_DIR);
            assert_eq!(root.name, "/");
        }
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
            BlockCount::new(IMAGE_BLOCKS as u64),
        );
        let device =
            hadris_storage::MemDevice::new(image.as_slice(), geometry.logical_block_size());
        let mut container = Container::open(device).await.unwrap();
        let superblock = container.superblock().clone();
        let volumes = container.volume_superblocks(&superblock).await.unwrap();
        let error = container
            .root_directory_owned_entries(&volumes[0])
            .await
            .unwrap_err();
        assert_eq!(error.kind(), hadris_io::ErrorKind::Unsupported);
    });
}

#[test]
fn holes_read_as_zeros() {
    let image = build_image();
    block_on(async {
        let geometry = BlockGeometry::new(
            BlockSize::new(BLOCK as u32).unwrap(),
            BlockCount::new(IMAGE_BLOCKS as u64),
        );
        let device =
            hadris_storage::MemDevice::new(image.as_slice(), geometry.logical_block_size());
        let mut container = Container::open(device).await.unwrap();
        let expected = holey_contents(2);
        let mut buf = vec![0xff_u8; expected.len()];
        let n = container
            .read_extents_at(&holey_extents(2), expected.len() as u64, 0, &mut buf)
            .await
            .unwrap();
        assert_eq!(&buf[..n], expected);
    });
}

#[test]
fn a_child_named_twice_is_rejected() {
    let image = build_image_variant(Variant::Revisit);
    block_on(async {
        let geometry = BlockGeometry::new(
            BlockSize::new(BLOCK as u32).unwrap(),
            BlockCount::new(IMAGE_BLOCKS as u64),
        );
        let device =
            hadris_storage::MemDevice::new(image.as_slice(), geometry.logical_block_size());
        let mut container = Container::open(device).await.unwrap();
        let superblock = container.superblock().clone();
        let volumes = container.volume_superblocks(&superblock).await.unwrap();
        assert_eq!(
            container
                .root_directory_owned_entries(&volumes[0])
                .await
                .unwrap_err()
                .kind(),
            hadris_io::ErrorKind::Corrupt
        );
    });
}

#[test]
fn storage_errors_keep_their_portable_kind() {
    use hadris_io::ErrorKind;
    use hadris_storage::MemDevice;

    block_on(async {
        let device = FailingDevice;
        assert!(matches!(
            Container::open(device).await,
            Err(error) if error.kind() == ErrorKind::InvalidInput
        ));

        let image = build_image();
        let device = MemDevice::new(&image[..], BlockSize::new(BLOCK as u32).unwrap());
        let mut container = Container::open(device).await.unwrap();
        assert!(matches!(
            container
                .read_apfs_block(IMAGE_BLOCKS as u64, &mut [0; BLOCK])
                .await,
            Err(error) if error.kind() == ErrorKind::Corrupt
        ));

        let device = MemDevice::new(&[0u8; 8192][..], BlockSize::new(8192).unwrap());
        assert!(matches!(
            Container::open(device).await,
            Err(error) if error.kind() == ErrorKind::Unsupported
        ));
    });
}

#[test]
fn encrypted_and_overflowing_extents_are_not_reported_as_sparse_success() {
    block_on(async {
        let image = build_image();
        let device =
            hadris_storage::MemDevice::new(&image[..], BlockSize::new(BLOCK as u32).unwrap());
        let mut container = Container::open(device).await.unwrap();
        let mut extent = holey_extents(0).remove(0);
        extent.cryptography_id = 7;
        let mut buf = [0xff; 8];
        assert_eq!(
            container
                .read_extents_at(&[extent], 8, 0, &mut buf)
                .await
                .unwrap_err()
                .kind(),
            hadris_io::ErrorKind::Unsupported
        );
        extent.cryptography_id = 0;
        extent.logical_address = u64::MAX - 2;
        assert_eq!(
            container
                .read_extents_at(&[extent], 8, 0, &mut buf)
                .await
                .unwrap_err()
                .kind(),
            hadris_io::ErrorKind::Corrupt
        );
    });
}
