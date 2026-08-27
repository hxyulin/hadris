//! Directory records, file extents, and multi-sector directories.

use std::fs;
use std::io::Cursor;
use std::sync::Arc;

use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{open, open_file, xorriso_sample_image};

#[test]
fn test_read_directory_structure() {
    let Some((_temp, iso_path)) = xorriso_sample_image(xorriso::create_minimal) else {
        return;
    };
    let image = open_file(&iso_path);
    let root = image.root_dir();
    let mut entries: Vec<String> = Vec::new();
    for entry_result in root.iter(&image).entries() {
        let entry = entry_result.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();
        if name != "\x00" && name != "\x01" {
            entries.push(name);
        }
    }

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
    let image = open(iso_data.clone());
    let root = image.root_dir();
    let mut found_file = false;
    for entry_result in root.iter(&image).entries() {
        let entry = entry_result.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();
        if name.to_uppercase().contains("TEST") && !entry.is_directory() {
            found_file = true;
            let header = entry.header();
            let extent = header.extent.read() as usize;
            let size = header.data_len.read() as usize;
            let offset = extent * 2048;
            assert_eq!(&iso_data[offset..offset + size], test_content);
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

    let image = open_file(&iso_path);
    let root = image.root_dir();
    let mut found_file = false;
    for entry_result in root.iter(&image).entries() {
        let entry = entry_result.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();
        if name.to_uppercase().contains("LARGE") && !entry.is_directory() {
            found_file = true;
            let size = entry.header().data_len.read() as usize;
            assert_eq!(size, 1024 * 1024, "Large file should be 1MB");
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

    let files: Vec<IsoFile> = (0..NUM_FILES)
        .map(|i| IsoFile::File {
            name: Arc::new(format!("FILE{i:03}.TXT")),
            contents: format!("Content of file {i}\n").into_bytes(),
        })
        .collect();
    let input_files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files,
    };
    let format_options = IsoFormatOptions {
        volume_name: "MULTISECTOR".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: None,
            hybrid_boot: None,
        },
        strict_charset: false,
    };
    let mut iso_buffer = Cursor::new(vec![0u8; 1024 * 2048]);
    IsoImageWriter::create(&mut iso_buffer, input_files, format_options)
        .expect("Failed to create ISO");

    let image = IsoImage::open(Cursor::new(iso_buffer.into_inner())).expect("Failed to open ISO");
    let root = image.root_dir();
    let mut file_names = Vec::new();
    for entry in root.iter(&image).entries() {
        let entry = entry.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();
        if name == "\0" || name == "\x01" {
            continue;
        }
        file_names.push(name);
    }
    assert_eq!(
        file_names.len(),
        NUM_FILES,
        "Expected {NUM_FILES} files but found {}. Names found: {file_names:?}",
        file_names.len()
    );
}
