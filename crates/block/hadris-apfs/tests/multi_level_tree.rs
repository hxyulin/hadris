#![cfg(all(feature = "sync", feature = "alloc"))]

mod common;

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
    assert_eq!(
        error,
        hadris_apfs::ApfsError::InvalidValue("encrypted B-tree nodes are not supported")
    );
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
            .unwrap_err(),
        hadris_apfs::ApfsError::InvalidValue("B-tree node revisited")
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
            .unwrap_err(),
        hadris_apfs::ApfsError::InvalidValue("file is too large to read into memory")
    );
    let head = container.read_file(&volume, entry.file_id, 8).unwrap();
    assert_eq!(head, file_contents(0)[..8]);
}

#[test]
fn storage_errors_keep_their_portable_kind() {
    use hadris_apfs::ApfsError;
    use hadris_io::ErrorKind;
    use hadris_storage::MemDevice;

    let device = MemDevice::new(&[0u8; 512][..], BlockSize::new(512).unwrap());
    assert!(matches!(
        Container::open(device),
        Err(ApfsError::Io(ErrorKind::InvalidInput))
    ));

    let image = build_image();
    let mut container = open(&image);
    assert!(matches!(
        container.read_apfs_block(IMAGE_BLOCKS as u64, &mut [0; BLOCK]),
        Err(ApfsError::Io(ErrorKind::InvalidInput))
    ));

    let device = MemDevice::new(&[0u8; 8192][..], BlockSize::new(8192).unwrap());
    assert!(matches!(
        Container::open(device),
        Err(ApfsError::InvalidValue("device block size"))
    ));
}
