//! Directory records, file extents, and multi-sector directories.

use std::fs;

use hadris_fs::{Content, Node, Tree};
use hadris_iso::{IsoOptions, Namespace, VolumeIdentifiers};
use hadris_tests::iso::hadris::write_tree;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{first_extent, list, open_file_ns, open_ns, xorriso_sample_image};

#[test]
fn test_read_directory_structure() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_minimal) else {
        return;
    };
    let mut view = open_file_ns(&iso_path, Namespace::Primary);
    let entries: Vec<String> = list(&mut view, "/")
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();

    assert!(entries.iter().any(|n| n.to_uppercase().contains("SUBDIR")));
    assert!(entries.iter().any(|n| n.to_uppercase().contains("DEEP")));
    assert!(
        entries
            .iter()
            .any(|n| n.to_uppercase().contains("README") || n.to_uppercase().contains("TXT"))
    );
}

#[test]
fn test_iso_file_content() {
    if !xorriso::require() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("content.iso");
    fs::create_dir(&content_dir).unwrap();
    let test_content = b"Test content for verification\n";
    fs::write(content_dir.join("test.txt"), test_content).unwrap();
    xorriso::create_minimal(&content_dir, &iso_path).unwrap();

    let iso_data = fs::read(&iso_path).unwrap();
    let mut view = open_ns(iso_data.clone(), Namespace::Primary);
    let mut found_file = false;
    for (name, node, meta) in list(&mut view, "/") {
        if name.to_uppercase().contains("TEST") && !meta.file_type().is_dir() {
            found_file = true;
            let extent = first_extent(&mut view, node);
            let offset = extent.offset() as usize;
            assert_eq!(
                &iso_data[offset..offset + extent.len() as usize],
                test_content
            );
            break;
        }
    }
    assert!(found_file, "Should have found the test file");
}

#[test]
fn test_large_file() {
    if !xorriso::require() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("large.iso");
    fs::create_dir(&content_dir).unwrap();
    let large_content: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
    fs::write(content_dir.join("large.bin"), &large_content).unwrap();
    xorriso::create_minimal(&content_dir, &iso_path).unwrap();

    let mut view = open_file_ns(&iso_path, Namespace::Primary);
    let mut found_file = false;
    for (name, _, meta) in list(&mut view, "/") {
        if name.to_uppercase().contains("LARGE") && !meta.file_type().is_dir() {
            found_file = true;
            assert_eq!(meta.len(), 1024 * 1024, "Large file should be 1MB");
            break;
        }
    }
    assert!(found_file, "Should have found the large file");
}

/// A root directory with 100 files spans several logical sectors; every
/// record must survive the sector boundaries.
#[test]
fn test_multi_sector_directory() {
    const NUM_FILES: usize = 100;

    let mut tree = Tree::new();
    for i in 0..NUM_FILES {
        tree.insert(
            format!("FILE{i:03}.TXT"),
            Node::file(Content::bytes(
                format!("Content of file {i}\n").into_bytes(),
            )),
        )
        .unwrap();
    }
    let options = IsoOptions::default().with_volume(VolumeIdentifiers::new("MULTISECTOR"));
    let bytes = write_tree(&tree, &options).expect("Failed to create ISO");

    let mut view = open_ns(bytes, Namespace::Primary);
    let file_names: Vec<String> = list(&mut view, "/")
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();
    assert_eq!(
        file_names.len(),
        NUM_FILES,
        "Expected {NUM_FILES} files but found {}. Names found: {file_names:?}",
        file_names.len()
    );
}
