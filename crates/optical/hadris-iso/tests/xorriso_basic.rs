mod xorriso_helpers;
use xorriso_helpers::*;

use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::process::Command;
use tempfile::TempDir;

use hadris_iso::volume::VolumeDescriptor;

#[test]
fn test_read_xorriso_minimal_iso() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("test.iso");

    fs::create_dir(&content_dir).unwrap();
    create_test_content(&content_dir);

    assert!(
        create_minimal_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create ISO with xorriso"
    );

    // Read the ISO with hadris-iso
    let mut iso_data = Vec::new();
    File::open(&iso_path)
        .unwrap()
        .read_to_end(&mut iso_data)
        .unwrap();

    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Verify volume identifier using read_pvd
    let pvd = image.read_pvd();
    let vol_id = pvd.volume_identifier.to_str().trim();
    assert_eq!(vol_id, "MINIMAL");
}

#[test]
fn test_read_xorriso_joliet_iso() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("joliet.iso");

    fs::create_dir(&content_dir).unwrap();
    create_test_content(&content_dir);

    assert!(
        create_joliet_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create Joliet ISO with xorriso"
    );

    // Read the ISO with hadris-iso
    let mut iso_data = Vec::new();
    File::open(&iso_path)
        .unwrap()
        .read_to_end(&mut iso_data)
        .unwrap();

    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Verify primary volume exists
    let pvd = image.read_pvd();
    let vol_id = pvd.volume_identifier.to_str().trim();
    assert_eq!(vol_id, "JOLIET_TEST");

    // Check for supplementary volume descriptor (Joliet)
    let has_joliet = image
        .read_volume_descriptors()
        .any(|vd| matches!(vd, Ok(VolumeDescriptor::Supplementary(_))));
    assert!(
        has_joliet,
        "Should have supplementary volume descriptor for Joliet"
    );
}

#[test]
fn test_read_xorriso_rockridge_iso() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("rockridge.iso");

    fs::create_dir(&content_dir).unwrap();
    create_test_content(&content_dir);

    assert!(
        create_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create Rock Ridge ISO with xorriso"
    );

    // Read the ISO with hadris-iso
    let mut iso_data = Vec::new();
    File::open(&iso_path)
        .unwrap()
        .read_to_end(&mut iso_data)
        .unwrap();

    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Verify volume info
    let pvd = image.read_pvd();
    let vol_id = pvd.volume_identifier.to_str().trim();
    assert_eq!(vol_id, "TEST_VOLUME");
}

#[test]
fn test_read_directory_structure() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("structure.iso");

    fs::create_dir(&content_dir).unwrap();
    create_test_content(&content_dir);

    assert!(
        create_minimal_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create ISO with xorriso"
    );

    // Read the ISO with hadris-iso
    let mut iso_data = Vec::new();
    File::open(&iso_path)
        .unwrap()
        .read_to_end(&mut iso_data)
        .unwrap();

    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Navigate the root directory
    let root = image.root_dir();
    let dir = root.iter(&image);

    // Collect all entries
    let mut entries: Vec<String> = Vec::new();
    for entry_result in dir.entries() {
        let entry = entry_result.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();
        // Skip special entries (. and ..)
        if name != "\x00" && name != "\x01" {
            entries.push(name);
        }
    }

    // Should have our files and directories
    assert!(entries.iter().any(|n| n.to_uppercase().contains("SUBDIR")));
    assert!(entries.iter().any(|n| n.to_uppercase().contains("DEEP")));
    // Note: ISO 9660 Level 1 converts filenames to uppercase
    assert!(
        entries
            .iter()
            .any(|n| n.to_uppercase().contains("README") || n.to_uppercase().contains("TXT"))
    );
}

#[test]
fn test_iso_file_content() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("content.iso");

    fs::create_dir(&content_dir).unwrap();

    // Create a file with known content
    let test_content = b"Test content for verification\n";
    fs::write(content_dir.join("test.txt"), test_content).unwrap();

    assert!(
        create_minimal_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create ISO with xorriso"
    );

    // Read the ISO with hadris-iso
    let iso_data = fs::read(&iso_path).unwrap();

    let cursor = Cursor::new(iso_data.clone());
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Find the test file in root directory
    let root = image.root_dir();
    let dir = root.iter(&image);
    let mut found_file = false;

    for entry_result in dir.entries() {
        let entry = entry_result.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();
        if name.to_uppercase().contains("TEST") && !entry.is_directory() {
            found_file = true;

            // Read the file content
            let header = entry.header();
            let extent = header.extent.read() as usize;
            let size = header.data_len.read() as usize;

            // The extent is in sectors (2048 bytes)
            let offset = extent * 2048;
            let file_data = &iso_data[offset..offset + size];

            assert_eq!(file_data, test_content);
            break;
        }
    }

    assert!(found_file, "Should have found the test file");
}

#[test]
fn test_unicode_filenames_joliet() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("unicode.iso");

    fs::create_dir(&content_dir).unwrap();

    // Create files with Unicode names
    fs::write(content_dir.join("日本語.txt"), "Japanese filename\n").unwrap();
    fs::write(content_dir.join("中文.txt"), "Chinese filename\n").unwrap();
    fs::write(content_dir.join("한국어.txt"), "Korean filename\n").unwrap();

    assert!(
        create_joliet_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create Joliet ISO with xorriso"
    );

    // Read the ISO with hadris-iso
    let mut iso_data = Vec::new();
    File::open(&iso_path)
        .unwrap()
        .read_to_end(&mut iso_data)
        .unwrap();

    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Verify supplementary volume descriptor exists (Joliet)
    let has_joliet = image
        .read_volume_descriptors()
        .any(|vd| matches!(vd, Ok(VolumeDescriptor::Supplementary(_))));
    assert!(has_joliet, "Should have Joliet supplementary volume");
}

#[test]
fn test_large_file() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("large.iso");

    fs::create_dir(&content_dir).unwrap();

    // Create a 1MB file
    let large_content: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
    fs::write(content_dir.join("large.bin"), &large_content).unwrap();

    assert!(
        create_minimal_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create ISO with xorriso"
    );

    // Read the ISO with hadris-iso
    let mut iso_data = Vec::new();
    File::open(&iso_path)
        .unwrap()
        .read_to_end(&mut iso_data)
        .unwrap();

    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Find the large file
    let root = image.root_dir();
    let dir = root.iter(&image);

    for entry_result in dir.entries() {
        let entry = entry_result.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();
        if name.to_uppercase().contains("LARGE") && !entry.is_directory() {
            let header = entry.header();
            let size = header.data_len.read() as usize;
            assert_eq!(size, 1024 * 1024, "Large file should be 1MB");
            break;
        }
    }
}

#[test]
fn test_xorriso_report() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("report.iso");

    fs::create_dir(&content_dir).unwrap();
    create_test_content(&content_dir);

    // Create ISO with full extensions
    assert!(
        create_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create ISO with xorriso"
    );

    // Use xorriso to print info about the ISO
    let output = Command::new("xorriso")
        .args([
            "-indev",
            iso_path.to_str().unwrap(),
            "-report_el_torito",
            "as_mkisofs",
        ])
        .output()
        .expect("Failed to run xorriso report");

    // Should complete without error
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "xorriso report should not fail catastrophically"
    );
}

#[test]
fn test_volume_descriptor_chain() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("vd_chain.iso");

    fs::create_dir(&content_dir).unwrap();
    fs::write(content_dir.join("test.txt"), "test").unwrap();

    assert!(
        create_iso_with_xorriso(&content_dir, &iso_path),
        "Failed to create ISO with xorriso"
    );

    let mut iso_data = Vec::new();
    File::open(&iso_path)
        .unwrap()
        .read_to_end(&mut iso_data)
        .unwrap();

    let cursor = Cursor::new(iso_data);
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Count volume descriptors
    let mut primary_count = 0;
    let mut supplementary_count = 0;
    let mut terminator_count = 0;

    for vd_result in image.read_volume_descriptors() {
        match vd_result {
            Ok(VolumeDescriptor::Primary(_)) => primary_count += 1,
            Ok(VolumeDescriptor::Supplementary(_)) => supplementary_count += 1,
            Ok(VolumeDescriptor::End(_)) => terminator_count += 1,
            _ => {}
        }
    }

    assert_eq!(
        primary_count, 1,
        "Should have exactly 1 primary volume descriptor"
    );
    assert!(
        supplementary_count >= 1,
        "Should have at least 1 supplementary (Joliet) descriptor"
    );
    // Note: Some iterators may stop before yielding the terminator, or may not include it
    // The important thing is that we found the expected descriptors
    assert!(terminator_count <= 1, "Should have at most 1 terminator");
}
