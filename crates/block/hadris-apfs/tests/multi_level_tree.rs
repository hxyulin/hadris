#![cfg(all(feature = "sync", feature = "alloc"))]

mod common;

use common::*;
use hadris_apfs::sync::Container;
use hadris_io::Cursor;
use hadris_storage::sync::SeekBlockDevice;
use hadris_storage::{BlockCount, BlockGeometry, BlockSize};

fn open(image: &[u8]) -> Container<SeekBlockDevice<Cursor<'_>>> {
    let geometry = BlockGeometry::new(
        BlockSize::new(BLOCK as u32).unwrap(),
        BlockCount(IMAGE_BLOCKS as u64),
    );
    Container::open(SeekBlockDevice::new(Cursor::new(image), geometry)).unwrap()
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
