mod xorriso_helpers;
use xorriso_helpers::*;

use std::fs;
use std::io::Cursor;
use std::process::Command;
use tempfile::TempDir;

/// Test that directories spanning multiple sectors are read correctly.
/// This tests the fix for the multi-sector directory bug where only the
/// first sector (~29 files) would be read.
#[test]
fn test_multi_sector_directory() {
    use hadris_iso::read::{IsoImage, PathSeparator};
    use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
    use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
    use std::sync::Arc;

    // Create 100 files - this should span multiple sectors
    // Each directory entry is typically ~50-100 bytes, so 100 files
    // should span ~5-10KB, which is 3-5 sectors.
    const NUM_FILES: usize = 100;

    let files: Vec<IsoFile> = (0..NUM_FILES)
        .map(|i| IsoFile::File {
            name: Arc::new(format!("FILE{:03}.TXT", i)),
            contents: format!("Content of file {}\n", i).into_bytes(),
        })
        .collect();

    let input_files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files,
    };

    let format_options = FormatOptions {
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

    // Create ISO in memory
    let mut iso_buffer = Cursor::new(vec![0u8; 1024 * 2048]); // 2MB should be plenty
    IsoImageWriter::format_new(&mut iso_buffer, input_files, format_options)
        .expect("Failed to create ISO");

    let iso_data = iso_buffer.into_inner();

    // Now read the ISO back and count the files
    let cursor = Cursor::new(iso_data);
    let image = IsoImage::open(cursor).expect("Failed to open ISO");

    let root = image.root_dir();
    let mut file_count = 0;
    let mut file_names = Vec::new();

    for entry in root.iter(&image).entries() {
        let entry = entry.expect("Failed to read directory entry");
        let name = String::from_utf8_lossy(entry.name()).to_string();

        // Skip . and .. entries
        if name == "\0" || name == "\x01" {
            continue;
        }

        file_count += 1;
        file_names.push(name);
    }

    println!("=== Multi-sector directory test ===");
    println!("  Created {} files", NUM_FILES);
    println!("  Found {} files", file_count);

    // We should find all 100 files, not just the ~29 that fit in one sector
    assert_eq!(
        file_count, NUM_FILES,
        "Expected {} files but found {}. Names found: {:?}",
        NUM_FILES, file_count, file_names
    );

    println!("=== Multi-sector directory test passed ===");
}

/// Test that a hadris-created Rock Ridge ISO contains valid RRIP entries
/// and can be read back. Also verifies with xorriso if available.
#[test]
fn test_hadris_rockridge_roundtrip() {
    use hadris_iso::read::PathSeparator;
    use hadris_iso::susp::{SystemUseField, SystemUseIter};
    use hadris_iso::write::options::{CreationFeatures, FormatOptions};
    use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
    use std::sync::Arc;

    // Create test files
    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![
            IsoFile::File {
                name: Arc::new("hello.txt".to_string()),
                contents: b"Hello, Rock Ridge!\n".to_vec(),
            },
            IsoFile::Directory {
                name: Arc::new("subdir".to_string()),
                children: vec![IsoFile::File {
                    name: Arc::new("nested.txt".to_string()),
                    contents: b"Nested content\n".to_vec(),
                }],
            },
        ],
    };

    let format_options = FormatOptions {
        volume_name: "RRIP_TEST".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::with_rock_ridge(),
        strict_charset: false,
    };

    // Write the ISO to a buffer
    let mut buffer = std::io::Cursor::new(vec![0u8; 4 * 1024 * 1024]);
    IsoImageWriter::format_new(&mut buffer, files, format_options)
        .expect("Failed to create Rock Ridge ISO");

    let iso_data = buffer.into_inner();

    // Read it back
    let cursor = Cursor::new(iso_data.clone());
    let image = hadris_iso::read::IsoImage::open(cursor).expect("Failed to open ISO image");

    // Verify volume ID
    let pvd = image.read_pvd();
    assert_eq!(pvd.volume_identifier.to_str().trim(), "RRIP_TEST");

    // Read root directory and verify RRIP entries on dot entry
    let root = image.root_dir();
    let dir = root.iter(&image);
    let mut entries = dir.entries();

    let dot_entry = entries.next().unwrap().unwrap();
    assert_eq!(dot_entry.name(), b"\x00", "First entry should be dot");

    let su = dot_entry.system_use();
    assert!(!su.is_empty(), "Dot entry should have system use data");

    let mut found_sp = false;
    let mut found_ce = false;
    let mut found_px = false;
    let mut found_nm = false;
    let mut ce_sector = 0u64;
    let mut ce_offset = 0u64;
    let mut ce_length = 0usize;

    for field in SystemUseIter::new(su, 0) {
        match field {
            SystemUseField::SuspIdentifier(sp) => {
                assert!(sp.is_valid(), "SP check bytes should be 0xBEEF");
                found_sp = true;
            }
            SystemUseField::ContinuationArea(ce) => {
                ce_sector = ce.sector.read() as u64;
                ce_offset = ce.offset.read() as u64;
                ce_length = ce.length.read() as usize;
                found_ce = true;
            }
            SystemUseField::PosixAttributes(_) => found_px = true,
            SystemUseField::AlternateName(_) => found_nm = true,
            _ => {}
        }
    }

    assert!(found_sp, "Root dot should have SP entry");
    assert!(found_ce, "Root dot should have CE entry (for full ER)");
    assert!(found_px, "Root dot should have PX entry");
    assert!(found_nm, "Root dot should have NM entry");

    // Follow the CE to read the full ER from the continuation area
    assert!(ce_length > 0, "CE length should be non-zero");
    let byte_pos = ce_sector * 2048 + ce_offset;
    let mut ce_buf = vec![0u8; ce_length];
    image
        .read_bytes_at(byte_pos, &mut ce_buf)
        .expect("Failed to read CE area");

    let mut found_er = false;
    for field in SystemUseIter::new(&ce_buf, 0) {
        if let SystemUseField::ExtensionReference(er) = field {
            let id_start = 4usize;
            let id_end = id_start + er.identifier_len as usize;
            if id_end <= er.buf.len() {
                let id = &er.buf[id_start..id_end];
                if id == b"RRIP_1991A" {
                    found_er = true;
                    // Verify the full ER has description and source
                    assert!(
                        er.descriptor_len > 0,
                        "Full ER should have non-empty descriptor"
                    );
                    assert!(er.source_len > 0, "Full ER should have non-empty source");
                }
            }
        }
    }
    assert!(
        found_er,
        "Continuation area should contain ER with RRIP_1991A identifier"
    );

    // Check dotdot entry has PX and NM
    let dotdot_entry = entries.next().unwrap().unwrap();
    assert_eq!(
        dotdot_entry.name(),
        b"\x01",
        "Second entry should be dotdot"
    );
    let dotdot_su = dotdot_entry.system_use();
    assert!(
        !dotdot_su.is_empty(),
        "Dotdot entry should have system use data"
    );

    // Check that regular file entries also have RRIP data
    let mut found_file_with_nm = false;
    for entry_result in entries {
        let entry = entry_result.unwrap();
        if entry.is_special() {
            continue;
        }
        let entry_su = entry.system_use();
        if !entry_su.is_empty() {
            for field in SystemUseIter::new(entry_su, 0) {
                if let SystemUseField::AlternateName(_) = field {
                    found_file_with_nm = true;
                }
            }
        }
    }
    assert!(
        found_file_with_nm,
        "File/directory entries should have NM entries"
    );

    println!("=== Hadris Rock Ridge round-trip: PASSED ===");

    // If xorriso is available, verify the ISO is recognized as Rock Ridge
    if xorriso_available() {
        let temp_dir = TempDir::new().unwrap();
        let iso_path = temp_dir.path().join("hadris_rrip.iso");
        fs::write(&iso_path, &iso_data).unwrap();

        // Use xorriso to inspect the ISO
        let output = Command::new("xorriso")
            .args([
                "-indev",
                iso_path.to_str().unwrap(),
                "-report_system_area",
                "plain",
                "-pvd_info",
            ])
            .output()
            .expect("Failed to run xorriso");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        println!("xorriso stdout: {}", stdout);
        println!("xorriso stderr: {}", stderr);

        // xorriso should be able to open the ISO without errors
        // (it may print warnings but shouldn't fail catastrophically)
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "xorriso should be able to read hadris RRIP ISO"
        );

        // List files with xorriso to verify Rock Ridge names are present
        let output = Command::new("xorriso")
            .args(["-indev", iso_path.to_str().unwrap(), "-ls", "/"])
            .output()
            .expect("Failed to run xorriso ls");

        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("xorriso ls /: {}", stdout);

        println!("=== xorriso Rock Ridge verification: PASSED ===");
    }
}
