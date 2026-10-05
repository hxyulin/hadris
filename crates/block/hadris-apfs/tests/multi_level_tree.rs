#![cfg(all(feature = "read", feature = "sync", feature = "alloc"))]

mod common;
#[path = "common/failing_device.rs"]
mod failing_device;

use failing_device::FailingDevice;

use common::*;
use hadris_apfs::sync::Container;
use hadris_io::Cursor;
use hadris_storage::sync::StreamDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

fn open(image: &[u8]) -> Container<StreamDevice<hadris_storage::ReadOnly<Cursor<'_>>>> {
    let geometry = BlockGeometry::new(
        BlockSize::new(BLOCK as u32).unwrap(),
        BlockCount::new(IMAGE_BLOCKS as u64),
    );
    Container::open(StreamDevice::with_block_count(
        hadris_storage::ReadOnly::new(Cursor::new(image)),
        geometry.logical_block_size(),
        geometry.block_count().get(),
    ))
    .unwrap()
}

fn check_reads_every_file(image: &[u8]) {
    let mut container = open(image);
    let superblock = container.superblock().clone();
    let volumes = container.volume_superblocks(&superblock).unwrap();
    assert_eq!(volumes.len(), 1);
    let volume = &volumes[0];

    let entries = container.root_directory_owned_entries(volume).unwrap();
    let mut names: Vec<_> = entries.iter().map(|entry| entry.name.clone()).collect();
    names.sort();
    let expected: Vec<_> = (0..FILE_COUNT).map(file_name).collect();
    assert_eq!(names, expected);

    for index in 0..FILE_COUNT {
        let entry = container
            .resolve_path(volume, &file_name(index))
            .unwrap()
            .expect("file present");
        let bytes = container
            .read_file(volume, entry.file_id, usize::MAX)
            .unwrap();
        assert_eq!(bytes, file_contents(index));
    }
}

#[test]
fn multi_level_filesystem_tree_resolves_virtual_children() {
    check_reads_every_file(&build_image());
}

#[test]
fn sealed_volume_tree_with_headerless_hashed_nodes_is_readable() {
    check_reads_every_file(&build_image_variant(Variant::Sealed));
}

#[test]
fn encrypted_tree_reports_unsupported() {
    let image = build_image_variant(Variant::Encrypted);
    let mut container = open(&image);
    let superblock = container.superblock().clone();
    let volumes = container.volume_superblocks(&superblock).unwrap();
    let error = container
        .root_directory_owned_entries(&volumes[0])
        .unwrap_err();
    assert_eq!(error.kind(), hadris_io::ErrorKind::Unsupported);
}

#[test]
fn physical_walk_does_not_follow_virtual_links() {
    let image = build_image();
    let mut container = open(&image);
    assert!(container.btree_leaf_entries(FS_ROOT_BLOCK).is_err());
}

#[test]
fn a_child_named_twice_is_rejected() {
    let image = build_image_variant(Variant::Revisit);
    let mut container = open(&image);
    let superblock = container.superblock().clone();
    let volumes = container.volume_superblocks(&superblock).unwrap();
    assert_eq!(
        container
            .root_directory_owned_entries(&volumes[0])
            .unwrap_err()
            .kind(),
        hadris_io::ErrorKind::Corrupt
    );
}

#[test]
fn holes_read_as_zeros() {
    let image = build_image();
    let mut container = open(&image);
    let expected = holey_contents(1);
    let size = expected.len() as u64;
    let extents = holey_extents(1);

    let mut buf = vec![0xff_u8; expected.len() + 100];
    let n = container
        .read_extents_at(&extents, size, 0, &mut buf)
        .unwrap();
    assert_eq!(&buf[..n], expected);

    let mut buf = [0xff_u8; 10];
    let n = container
        .read_extents_at(&extents, size, 2 * BLOCK as u64 - 4, &mut buf)
        .unwrap();
    assert_eq!(&buf[..n], &expected[2 * BLOCK - 4..2 * BLOCK + 6]);
    assert_eq!(
        container
            .read_extents_at(&extents, size, size, &mut buf)
            .unwrap(),
        0
    );
}

#[test]
fn lookups_follow_the_volume_case_sensitivity() {
    let mut image = build_image();
    let upper = file_name(3).to_uppercase();
    {
        let mut container = open(&image);
        let superblock = container.superblock().clone();
        let volume = container.volume_superblocks(&superblock).unwrap().remove(0);
        assert!(container.resolve_path(&volume, &upper).unwrap().is_none());
    }
    set_volume_incompatible_features(&mut image, 1);
    let mut container = open(&image);
    let superblock = container.superblock().clone();
    let volume = container.volume_superblocks(&superblock).unwrap().remove(0);
    let entry = container.resolve_path(&volume, &upper).unwrap().unwrap();
    assert_eq!(
        container
            .read_file(&volume, entry.file_id, usize::MAX)
            .unwrap(),
        file_contents(3)
    );
}

#[test]
fn an_impossible_file_size_is_an_error() {
    let image = build_image_variant(Variant::HugeFile);
    let mut container = open(&image);
    let superblock = container.superblock().clone();
    let volume = container.volume_superblocks(&superblock).unwrap().remove(0);
    let entry = container
        .resolve_path(&volume, &file_name(0))
        .unwrap()
        .unwrap();
    assert_eq!(
        container
            .read_file(&volume, entry.file_id, usize::MAX)
            .unwrap_err()
            .kind(),
        hadris_io::ErrorKind::Corrupt
    );
    let head = container.read_file(&volume, entry.file_id, 8).unwrap();
    assert_eq!(head, file_contents(0)[..8]);
}

#[test]
fn storage_errors_keep_their_portable_kind() {
    use hadris_io::ErrorKind;
    use hadris_storage::MemDevice;

    let device = FailingDevice;
    assert!(matches!(
        Container::open(device),
        Err(error) if error.kind() == ErrorKind::InvalidInput
    ));

    let image = build_image();
    let mut container = open(&image);
    assert!(matches!(
        container.read_apfs_block(IMAGE_BLOCKS as u64, &mut [0; BLOCK]),
        Err(error) if error.kind() == ErrorKind::Corrupt
    ));

    let device = MemDevice::new(&[0u8; 8192][..], BlockSize::new(8192).unwrap());
    assert!(matches!(
        Container::open(device),
        Err(error) if error.kind() == ErrorKind::Unsupported
    ));
}

#[test]
fn encrypted_and_overflowing_extents_are_not_reported_as_sparse_success() {
    let image = build_image();
    let mut container = open(&image);
    let mut extent = holey_extents(0).remove(0);
    extent.cryptography_id = 7;
    let mut buf = [0xff; 8];
    assert_eq!(
        container
            .read_extents_at(&[extent], 8, 0, &mut buf)
            .unwrap_err()
            .kind(),
        hadris_io::ErrorKind::Unsupported
    );
    extent.cryptography_id = 0;
    extent.logical_address = u64::MAX - 2;
    assert_eq!(
        container
            .read_extents_at(&[extent], 8, 0, &mut buf)
            .unwrap_err()
            .kind(),
        hadris_io::ErrorKind::Corrupt
    );
}

#[test]
fn object_map_lookup_preserves_full_identifiers_and_deletions() {
    let high_oid = VOLUME_OID | (1 << 60);
    let image = object_map_versions_image(
        &[
            (VOLUME_OID, 1, 3, 0),
            (VOLUME_OID, 2, 0, 1),
            (VOLUME_OID, 3, 3, 0),
            (high_oid, 4, 23, 0),
        ],
        3,
    );
    let mut container = open(&image);
    let superblock = container.superblock().clone();
    let map = container.object_map(&superblock).unwrap();
    assert_eq!(
        container.object_map_lookup(map, VOLUME_OID, 0).unwrap(),
        None
    );
    assert_eq!(
        container
            .object_map_lookup(map, VOLUME_OID, 1)
            .unwrap()
            .unwrap()
            .address,
        3
    );
    assert_eq!(
        container.object_map_lookup(map, VOLUME_OID, 2).unwrap(),
        None
    );
    assert_eq!(
        container
            .object_map_lookup(map, VOLUME_OID, 4)
            .unwrap()
            .unwrap()
            .address,
        3
    );
    assert_eq!(
        container
            .object_map_lookup(map, high_oid, 4)
            .unwrap()
            .unwrap()
            .address,
        23
    );
    #[cfg(feature = "async")]
    {
        use core::future::Future;
        use core::task::{Context, Poll, Waker};
        let mut future = core::pin::pin!(async {
            let dev = hadris_storage::MemDevice::new(image, BlockSize::new(BLOCK as u32).unwrap());
            let mut container = hadris_apfs::r#async::Container::open(dev).await.unwrap();
            let superblock = container.superblock().clone();
            let map = container.object_map(&superblock).await.unwrap();
            assert_eq!(
                container
                    .object_map_lookup(map, VOLUME_OID, 2)
                    .await
                    .unwrap(),
                None
            );
            assert_eq!(
                container
                    .object_map_lookup(map, VOLUME_OID, 4)
                    .await
                    .unwrap()
                    .unwrap()
                    .address,
                3
            );
            assert_eq!(
                container
                    .object_map_lookup(map, high_oid, 4)
                    .await
                    .unwrap()
                    .unwrap()
                    .address,
                23
            );
        });
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut cx), Poll::Ready(())));
    }
}

#[test]
fn volume_enumeration_resolves_one_live_version_per_checkpoint() {
    for deleted in [false, true] {
        let image = object_map_versions_image(
            &[
                (VOLUME_OID, 1, 3, 0),
                (
                    VOLUME_OID,
                    2,
                    if deleted { 0 } else { 3 },
                    u32::from(deleted),
                ),
                (VOLUME_OID, 3, 23, 0),
                (VOLUME_OID | (1 << 60), 1, 23, 0),
            ],
            2,
        );
        let mut container = open(&image);
        let superblock = container.superblock().clone();
        let volumes = container.volume_superblocks(&superblock).unwrap();
        assert_eq!(volumes.len(), usize::from(!deleted));
        if !deleted {
            let dev = hadris_storage::MemDevice::new(
                image.clone(),
                BlockSize::new(BLOCK as u32).unwrap(),
            );
            hadris_apfs::sync::ApfsFs::mount(dev, hadris_fs::MountOptions::new()).unwrap();
        }
        #[cfg(feature = "async")]
        {
            use core::future::Future;
            use core::task::{Context, Poll, Waker};
            let mut future = core::pin::pin!(async {
                let dev =
                    hadris_storage::MemDevice::new(image, BlockSize::new(BLOCK as u32).unwrap());
                let mut container = hadris_apfs::r#async::Container::open(dev).await.unwrap();
                let superblock = container.superblock().clone();
                let volumes = container.volume_superblocks(&superblock).await.unwrap();
                assert_eq!(volumes.len(), usize::from(!deleted));
            });
            let mut cx = Context::from_waker(Waker::noop());
            assert!(matches!(future.as_mut().poll(&mut cx), Poll::Ready(())));
        }
    }
}
