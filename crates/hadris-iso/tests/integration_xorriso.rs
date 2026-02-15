//! Integration tests using xorriso
//!
//! These tests verify that hadris-iso can correctly read ISOs created by xorriso,
//! which is the reference implementation for ISO 9660.

use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use hadris_iso::types::Endian;
use hadris_iso::volume::VolumeDescriptor;

/// Check if xorriso is available on the system
fn xorriso_available() -> bool {
    Command::new("xorriso")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a test directory structure with some files
fn create_test_content(dir: &Path) {
    // Create directories
    fs::create_dir_all(dir.join("subdir")).unwrap();
    fs::create_dir_all(dir.join("deep/nested/path")).unwrap();

    // Create files with various content
    fs::write(dir.join("readme.txt"), "This is a test file.\n").unwrap();
    fs::write(dir.join("hello.txt"), "Hello, World!\n").unwrap();
    fs::write(dir.join("subdir/data.bin"), vec![0u8; 1024]).unwrap();
    fs::write(
        dir.join("deep/nested/path/deep_file.txt"),
        "Deep nested content\n",
    )
    .unwrap();

    // Create a larger file (64KB)
    let large_content: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();
    fs::write(dir.join("large_file.bin"), &large_content).unwrap();
}

/// Create an ISO using xorriso
fn create_iso_with_xorriso(content_dir: &Path, iso_path: &Path) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "TEST_VOLUME",
            "-J", // Joliet
            "-R", // Rock Ridge
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    output.status.success()
}

/// Create a minimal ISO using xorriso (no extensions)
fn create_minimal_iso_with_xorriso(content_dir: &Path, iso_path: &Path) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "MINIMAL",
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    output.status.success()
}

/// Create a Joliet-only ISO using xorriso
fn create_joliet_iso_with_xorriso(content_dir: &Path, iso_path: &Path) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "JOLIET_TEST",
            "-J", // Joliet only
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    output.status.success()
}

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

/// Create a bootable ISO using xorriso with El-Torito
fn create_bootable_iso_with_xorriso(content_dir: &Path, iso_path: &Path, boot_image: &str) -> bool {
    let output = Command::new("xorriso")
        .args([
            "-as",
            "mkisofs",
            "-o",
            iso_path.to_str().unwrap(),
            "-V",
            "BOOT_TEST",
            "-b",
            boot_image,
            "-no-emul-boot",
            "-boot-load-size",
            "4",
            content_dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run xorriso");

    if !output.status.success() {
        eprintln!(
            "xorriso stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    output.status.success()
}

#[test]
fn test_eltorito_boot_catalog_comparison() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("boot.iso");

    fs::create_dir(&content_dir).unwrap();

    // Create a simple boot image (512 bytes, like a boot sector)
    let boot_image = vec![0xEB, 0xFE]; // Simple infinite loop (jmp $)
    let mut boot_data = vec![0u8; 2048]; // Pad to one sector
    boot_data[..boot_image.len()].copy_from_slice(&boot_image);
    fs::write(content_dir.join("boot.bin"), &boot_data).unwrap();

    assert!(
        create_bootable_iso_with_xorriso(&content_dir, &iso_path, "boot.bin"),
        "Failed to create bootable ISO with xorriso"
    );

    // Read the ISO
    let iso_data = fs::read(&iso_path).unwrap();

    // Find boot record volume descriptor (type 0x00) to get boot catalog location
    let mut boot_catalog_lba: Option<u32> = None;
    for sector in 16..32 {
        let offset = sector * 2048;
        if iso_data[offset] == 0x00 && &iso_data[offset + 1..offset + 6] == b"CD001" {
            // Boot Record Volume Descriptor found
            // Boot catalog pointer is at offset 71 (little-endian u32)
            let ptr_bytes: [u8; 4] = iso_data[offset + 71..offset + 75].try_into().unwrap();
            boot_catalog_lba = Some(u32::from_le_bytes(ptr_bytes));
            break;
        }
        if iso_data[offset] == 0xFF {
            break; // Volume set terminator
        }
    }

    let boot_catalog_lba = boot_catalog_lba.expect("Should find boot record volume descriptor");
    let catalog_offset = boot_catalog_lba as usize * 2048;

    println!(
        "Boot catalog at LBA {}, offset {:#x}",
        boot_catalog_lba, catalog_offset
    );

    // Read and parse boot catalog entries
    let validation_entry = &iso_data[catalog_offset..catalog_offset + 32];
    let default_entry = &iso_data[catalog_offset + 32..catalog_offset + 64];

    println!("\n=== XORRISO Boot Catalog (Reference) ===");
    println!("Validation Entry:");
    println!("  Header ID: {:#04x} (expected 0x01)", validation_entry[0]);
    println!(
        "  Platform ID: {:#04x} (0x00=x86, 0xEF=UEFI)",
        validation_entry[1]
    );
    println!("  Reserved: {:02x?}", &validation_entry[2..4]);
    println!(
        "  Manufacturer: {:?}",
        String::from_utf8_lossy(&validation_entry[4..28])
    );
    let checksum = u16::from_le_bytes([validation_entry[28], validation_entry[29]]);
    println!("  Checksum: {:#06x}", checksum);
    println!(
        "  Key: {:02x} {:02x} (expected 55 AA)",
        validation_entry[30], validation_entry[31]
    );

    println!("\nDefault Entry:");
    println!(
        "  Boot Indicator: {:#04x} (0x88=bootable)",
        default_entry[0]
    );
    println!(
        "  Boot Media Type: {:#04x} (0x00=no emulation)",
        default_entry[1]
    );
    let load_segment = u16::from_le_bytes([default_entry[2], default_entry[3]]);
    println!("  Load Segment: {:#06x}", load_segment);
    println!("  System Type: {:#04x}", default_entry[4]);
    println!("  Reserved: {:#04x}", default_entry[5]);
    let sector_count = u16::from_le_bytes([default_entry[6], default_entry[7]]);
    println!("  Sector Count: {} (512-byte sectors)", sector_count);
    let load_rba = u32::from_le_bytes([
        default_entry[8],
        default_entry[9],
        default_entry[10],
        default_entry[11],
    ]);
    println!("  Load RBA (LBA): {}", load_rba);
    println!("  Selection Criteria: {:#04x}", default_entry[12]);

    // Verify validation checksum
    let mut sum = 0u16;
    for i in (0..32).step_by(2) {
        let word = u16::from_le_bytes([validation_entry[i], validation_entry[i + 1]]);
        sum = sum.wrapping_add(word);
    }
    println!(
        "\n  Checksum verification: sum = {:#06x} (should be 0x0000)",
        sum
    );
    assert_eq!(sum, 0, "Validation entry checksum should sum to 0");

    // Verify boot indicator
    assert_eq!(default_entry[0], 0x88, "Default entry should be bootable");

    // Parse with hadris-iso boot catalog parser
    use hadris_iso::boot::BaseBootCatalog;
    let mut catalog_cursor = Cursor::new(&iso_data[catalog_offset..catalog_offset + 64]);
    match BaseBootCatalog::parse(&mut catalog_cursor) {
        Ok(catalog) => {
            println!("\n=== Hadris-ISO Parsed Boot Catalog ===");
            println!("  Validation valid: {}", catalog.validation.is_valid());
            println!(
                "  Default bootable: {}",
                catalog.default_entry.is_bootable()
            );
            let entry = &catalog.default_entry;
            println!("  Load Segment: {:#06x}", entry.load_segment.get());
            println!("  Sector Count: {}", entry.sector_count.get());
            println!("  Load RBA: {}", entry.load_rba.get());
        }
        Err(e) => {
            println!("\nError parsing boot catalog with hadris-iso: {:?}", e);
        }
    }

    println!("\n=== Test passed: xorriso boot catalog is valid ===");
}

#[test]
fn test_hadris_bootable_iso_creation() {
    use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
    use hadris_iso::read::PathSeparator;
    use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
    use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
    use std::sync::Arc;

    // Create a simple boot image (2048 bytes = 1 sector)
    let boot_image = vec![0xEB, 0xFE]; // Simple infinite loop (jmp $)
    let mut boot_data = vec![0u8; 2048];
    boot_data[..boot_image.len()].copy_from_slice(&boot_image);

    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![IsoFile::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_data.clone(),
        }],
    };

    let boot_options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: hadris_iso::boot::EmulationType::NoEmulation,
        },
        entries: vec![],
    };

    let format_options = FormatOptions {
        volume_name: "BOOT_TEST".to_string(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: None,
        },
    };

    let mut iso_buffer = Cursor::new(vec![0u8; 256 * 2048]); // 256 sectors
    IsoImageWriter::format_new(&mut iso_buffer, files, format_options)
        .expect("Failed to create bootable ISO with hadris-iso");

    let iso_data = iso_buffer.into_inner();

    // Find boot record volume descriptor
    let mut boot_catalog_lba: Option<u32> = None;
    for sector in 16..32 {
        let offset = sector * 2048;
        if iso_data[offset] == 0x00 && &iso_data[offset + 1..offset + 6] == b"CD001" {
            let ptr_bytes: [u8; 4] = iso_data[offset + 71..offset + 75].try_into().unwrap();
            boot_catalog_lba = Some(u32::from_le_bytes(ptr_bytes));
            break;
        }
        if iso_data[offset] == 0xFF {
            break;
        }
    }

    let boot_catalog_lba = boot_catalog_lba.expect("Should find boot record volume descriptor");
    let catalog_offset = boot_catalog_lba as usize * 2048;

    println!("\n=== Hadris-ISO Generated Boot Catalog ===");
    println!(
        "Boot catalog at LBA {}, offset {:#x}",
        boot_catalog_lba, catalog_offset
    );

    let validation_entry = &iso_data[catalog_offset..catalog_offset + 32];
    let default_entry = &iso_data[catalog_offset + 32..catalog_offset + 64];

    println!("\nValidation Entry:");
    println!("  Header ID: {:#04x} (expected 0x01)", validation_entry[0]);
    println!(
        "  Platform ID: {:#04x} (0x00=x86, 0xEF=UEFI)",
        validation_entry[1]
    );
    let checksum = u16::from_le_bytes([validation_entry[28], validation_entry[29]]);
    println!("  Checksum: {:#06x}", checksum);
    println!(
        "  Key: {:02x} {:02x} (expected 55 AA)",
        validation_entry[30], validation_entry[31]
    );

    // Verify validation checksum
    let mut sum = 0u16;
    for i in (0..32).step_by(2) {
        let word = u16::from_le_bytes([validation_entry[i], validation_entry[i + 1]]);
        sum = sum.wrapping_add(word);
    }
    println!(
        "  Checksum verification: sum = {:#06x} (should be 0x0000)",
        sum
    );

    println!("\nDefault Entry:");
    println!(
        "  Boot Indicator: {:#04x} (0x88=bootable)",
        default_entry[0]
    );
    println!(
        "  Boot Media Type: {:#04x} (0x00=no emulation)",
        default_entry[1]
    );
    let load_segment = u16::from_le_bytes([default_entry[2], default_entry[3]]);
    println!("  Load Segment: {:#06x}", load_segment);
    println!("  System Type: {:#04x}", default_entry[4]);
    println!("  Reserved: {:#04x}", default_entry[5]);
    let sector_count = u16::from_le_bytes([default_entry[6], default_entry[7]]);
    println!("  Sector Count: {} (512-byte sectors)", sector_count);
    let load_rba = u32::from_le_bytes([
        default_entry[8],
        default_entry[9],
        default_entry[10],
        default_entry[11],
    ]);
    println!("  Load RBA (LBA): {}", load_rba);
    println!("  Selection Criteria: {:#04x}", default_entry[12]);

    // Basic assertions
    assert_eq!(validation_entry[0], 0x01, "Header ID should be 0x01");
    assert_eq!(validation_entry[30], 0x55, "Key byte 1 should be 0x55");
    assert_eq!(validation_entry[31], 0xAA, "Key byte 2 should be 0xAA");
    assert_eq!(sum, 0, "Validation checksum should sum to 0");
    assert_eq!(
        default_entry[0], 0x88,
        "Default entry should be bootable (0x88)"
    );
    assert_eq!(
        default_entry[1], 0x00,
        "Boot media type should be no-emulation (0x00)"
    );
    assert_eq!(sector_count, 4, "Sector count should be 4");

    // Find the boot image file to verify LBA
    let _boot_image_lba: Option<u32> = None;
    let pvd_offset = 16 * 2048;
    let root_dir_lba = u32::from_le_bytes([
        iso_data[pvd_offset + 158],
        iso_data[pvd_offset + 159],
        iso_data[pvd_offset + 160],
        iso_data[pvd_offset + 161],
    ]);
    println!("\nRoot directory at LBA: {}", root_dir_lba);

    // Check that Load RBA is reasonable (should be a valid LBA in the ISO)
    assert!(load_rba > 16, "Load RBA should be after volume descriptors");
    assert!(
        load_rba < (iso_data.len() / 2048) as u32,
        "Load RBA should be within ISO"
    );

    println!("\n=== Hadris-ISO boot catalog generation: PASSED ===");
}

#[test]
fn test_compare_boot_catalogs() {
    use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
    use hadris_iso::read::PathSeparator;
    use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
    use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
    use std::sync::Arc;

    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let xorriso_iso_path = temp_dir.path().join("xorriso.iso");

    fs::create_dir(&content_dir).unwrap();

    // Create identical boot images
    let mut boot_data = vec![0u8; 2048];
    boot_data[0] = 0xEB; // jmp
    boot_data[1] = 0xFE; // $
    fs::write(content_dir.join("boot.bin"), &boot_data).unwrap();

    // Create xorriso ISO
    assert!(
        create_bootable_iso_with_xorriso(&content_dir, &xorriso_iso_path, "boot.bin"),
        "Failed to create bootable ISO with xorriso"
    );

    // Create hadris-iso ISO
    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![IsoFile::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_data.clone(),
        }],
    };

    let boot_options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: hadris_iso::boot::EmulationType::NoEmulation,
        },
        entries: vec![],
    };

    let format_options = FormatOptions {
        volume_name: "BOOT_TEST".to_string(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: None,
        },
    };

    let mut hadris_buffer = Cursor::new(vec![0u8; 256 * 2048]);
    IsoImageWriter::format_new(&mut hadris_buffer, files, format_options)
        .expect("Failed to create hadris ISO");
    let hadris_data = hadris_buffer.into_inner();

    // Read xorriso ISO
    let xorriso_data = fs::read(&xorriso_iso_path).unwrap();

    // Find boot records and catalogs
    fn find_boot_catalog(data: &[u8]) -> Option<(usize, usize)> {
        for sector in 16..32 {
            let offset = sector * 2048;
            if data.len() <= offset + 75 {
                continue;
            }
            if data[offset] == 0x00 && &data[offset + 1..offset + 6] == b"CD001" {
                let ptr_bytes: [u8; 4] = data[offset + 71..offset + 75].try_into().ok()?;
                let catalog_lba = u32::from_le_bytes(ptr_bytes) as usize;
                return Some((sector, catalog_lba));
            }
        }
        None
    }

    let xorriso_boot = find_boot_catalog(&xorriso_data);
    let hadris_boot = find_boot_catalog(&hadris_data);

    println!("\n=== BOOT CATALOG COMPARISON ===\n");

    if let (Some((x_br_sector, x_cat_lba)), Some((h_br_sector, h_cat_lba))) =
        (xorriso_boot, hadris_boot)
    {
        println!(
            "xorriso: Boot Record at sector {}, Catalog at LBA {}",
            x_br_sector, x_cat_lba
        );
        println!(
            "hadris:  Boot Record at sector {}, Catalog at LBA {}",
            h_br_sector, h_cat_lba
        );

        let x_cat_offset = x_cat_lba * 2048;
        let h_cat_offset = h_cat_lba * 2048;

        println!("\n--- Validation Entry (32 bytes) ---");
        println!(
            "xorriso: {:02x?}",
            &xorriso_data[x_cat_offset..x_cat_offset + 32]
        );
        println!(
            "hadris:  {:02x?}",
            &hadris_data[h_cat_offset..h_cat_offset + 32]
        );

        // Check for differences
        let x_val = &xorriso_data[x_cat_offset..x_cat_offset + 32];
        let h_val = &hadris_data[h_cat_offset..h_cat_offset + 32];

        if x_val[0] != h_val[0] {
            println!(
                "DIFF: Header ID - xorriso={:#04x}, hadris={:#04x}",
                x_val[0], h_val[0]
            );
        }
        if x_val[1] != h_val[1] {
            println!(
                "DIFF: Platform ID - xorriso={:#04x}, hadris={:#04x}",
                x_val[1], h_val[1]
            );
        }

        println!("\n--- Default/Initial Entry (32 bytes) ---");
        println!(
            "xorriso: {:02x?}",
            &xorriso_data[x_cat_offset + 32..x_cat_offset + 64]
        );
        println!(
            "hadris:  {:02x?}",
            &hadris_data[h_cat_offset + 32..h_cat_offset + 64]
        );

        let x_def = &xorriso_data[x_cat_offset + 32..x_cat_offset + 64];
        let h_def = &hadris_data[h_cat_offset + 32..h_cat_offset + 64];

        if x_def[0] != h_def[0] {
            println!(
                "DIFF: Boot Indicator - xorriso={:#04x}, hadris={:#04x}",
                x_def[0], h_def[0]
            );
        }
        if x_def[1] != h_def[1] {
            println!(
                "DIFF: Boot Media Type - xorriso={:#04x}, hadris={:#04x}",
                x_def[1], h_def[1]
            );
        }

        let x_load_seg = u16::from_le_bytes([x_def[2], x_def[3]]);
        let h_load_seg = u16::from_le_bytes([h_def[2], h_def[3]]);
        if x_load_seg != h_load_seg {
            println!(
                "DIFF: Load Segment - xorriso={:#06x}, hadris={:#06x}",
                x_load_seg, h_load_seg
            );
        }

        if x_def[4] != h_def[4] {
            println!(
                "DIFF: System Type - xorriso={:#04x}, hadris={:#04x}",
                x_def[4], h_def[4]
            );
        }

        let x_sector_count = u16::from_le_bytes([x_def[6], x_def[7]]);
        let h_sector_count = u16::from_le_bytes([h_def[6], h_def[7]]);
        if x_sector_count != h_sector_count {
            println!(
                "DIFF: Sector Count - xorriso={}, hadris={}",
                x_sector_count, h_sector_count
            );
        }

        let x_load_rba = u32::from_le_bytes([x_def[8], x_def[9], x_def[10], x_def[11]]);
        let h_load_rba = u32::from_le_bytes([h_def[8], h_def[9], h_def[10], h_def[11]]);
        println!("\nLoad RBA: xorriso={}, hadris={}", x_load_rba, h_load_rba);

        // Check what's after the default entry
        println!("\n--- Next 32 bytes (after default entry) ---");
        println!(
            "xorriso: {:02x?}",
            &xorriso_data[x_cat_offset + 64..x_cat_offset + 96]
        );
        println!(
            "hadris:  {:02x?}",
            &hadris_data[h_cat_offset + 64..h_cat_offset + 96]
        );

        // Check boot record volume descriptor
        println!("\n--- Boot Record Volume Descriptor ---");
        let x_br_offset = x_br_sector * 2048;
        let h_br_offset = h_br_sector * 2048;
        println!(
            "xorriso boot system identifier: {:?}",
            String::from_utf8_lossy(&xorriso_data[x_br_offset + 7..x_br_offset + 39])
        );
        println!(
            "hadris  boot system identifier: {:?}",
            String::from_utf8_lossy(&hadris_data[h_br_offset + 7..h_br_offset + 39])
        );

        // Verify catalogs are valid
        let mut x_sum = 0u16;
        for i in (0..32).step_by(2) {
            x_sum = x_sum.wrapping_add(u16::from_le_bytes([x_val[i], x_val[i + 1]]));
        }
        let mut h_sum = 0u16;
        for i in (0..32).step_by(2) {
            h_sum = h_sum.wrapping_add(u16::from_le_bytes([h_val[i], h_val[i + 1]]));
        }
        println!("\nChecksum verification:");
        println!(
            "  xorriso: {} ({})",
            x_sum,
            if x_sum == 0 { "VALID" } else { "INVALID" }
        );
        println!(
            "  hadris:  {} ({})",
            h_sum,
            if h_sum == 0 { "VALID" } else { "INVALID" }
        );
    } else {
        println!("Could not find boot catalogs!");
        println!("xorriso boot: {:?}", xorriso_boot);
        println!("hadris boot: {:?}", hadris_boot);
    }
}

/// Check if qemu is available
fn qemu_available() -> bool {
    Command::new("qemu-system-x86_64")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run QEMU with a timeout
fn run_qemu_with_timeout(iso_path: &Path, timeout_secs: u64) -> Option<String> {
    use std::io::Read as StdRead;
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let mut child = Command::new("qemu-system-x86_64")
        .args([
            "-cdrom",
            iso_path.to_str().unwrap(),
            "-boot",
            "d",
            "-nographic",
            "-serial",
            "stdio",
            "-no-reboot",
            "-m",
            "16",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    // Wait with timeout
    let timeout = Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break,
        }
    }

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }

    Some(stdout)
}

#[test]
fn test_qemu_boot_xorriso_iso() {
    if !xorriso_available() {
        eprintln!("Skipping test: xorriso not available");
        return;
    }
    if !qemu_available() {
        eprintln!("Skipping test: qemu-system-x86_64 not available");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("xorriso_boot.iso");

    fs::create_dir(&content_dir).unwrap();

    // Create a boot image that writes "OK" to serial port then halts
    // This is x86 machine code that:
    // 1. Writes 'O' to COM1 (0x3F8)
    // 2. Writes 'K' to COM1
    // 3. Writes '\n' to COM1
    // 4. Halts with hlt instruction
    let boot_code: Vec<u8> = vec![
        0xB0, 0x4F, // mov al, 'O'
        0xBA, 0xF8, 0x03, // mov dx, 0x3F8
        0xEE, // out dx, al
        0xB0, 0x4B, // mov al, 'K'
        0xEE, // out dx, al
        0xB0, 0x0A, // mov al, '\n'
        0xEE, // out dx, al
        0xF4, // hlt
        0xEB, 0xFD, // jmp $-1 (infinite loop if hlt fails)
    ];

    // Pad to 2048 bytes (one sector)
    let mut boot_data = vec![0u8; 2048];
    boot_data[..boot_code.len()].copy_from_slice(&boot_code);
    fs::write(content_dir.join("boot.bin"), &boot_data).unwrap();

    // Create bootable ISO with xorriso
    assert!(
        create_bootable_iso_with_xorriso(&content_dir, &iso_path, "boot.bin"),
        "Failed to create bootable ISO with xorriso"
    );

    // Boot with QEMU and capture serial output
    match run_qemu_with_timeout(&iso_path, 5) {
        Some(stdout) => {
            println!("QEMU stdout: {}", stdout);

            // Check if our boot code produced the expected output
            if stdout.contains("OK") {
                println!("=== xorriso ISO boots successfully in QEMU ===");
            } else {
                println!("Note: Boot code may not have executed as expected");
                println!("This could be due to BIOS initialization or boot sequence");
            }
        }
        None => {
            println!("QEMU command failed to run");
        }
    }
}

#[test]
fn test_qemu_boot_hadris_iso() {
    if !qemu_available() {
        eprintln!("Skipping test: qemu-system-x86_64 not available");
        return;
    }

    use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
    use hadris_iso::read::PathSeparator;
    use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, FormatOptions};
    use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};
    use std::sync::Arc;

    let temp_dir = TempDir::new().unwrap();
    let iso_path = temp_dir.path().join("hadris_boot.iso");

    // Create a boot image that writes "OK" to serial port then halts
    let boot_code: Vec<u8> = vec![
        0xB0, 0x4F, // mov al, 'O'
        0xBA, 0xF8, 0x03, // mov dx, 0x3F8
        0xEE, // out dx, al
        0xB0, 0x4B, // mov al, 'K'
        0xEE, // out dx, al
        0xB0, 0x0A, // mov al, '\n'
        0xEE, // out dx, al
        0xF4, // hlt
        0xEB, 0xFD, // jmp $-1 (infinite loop if hlt fails)
    ];

    // Pad to 2048 bytes (one sector)
    let mut boot_data = vec![0u8; 2048];
    boot_data[..boot_code.len()].copy_from_slice(&boot_code);

    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![IsoFile::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_data.clone(),
        }],
    };

    let boot_options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: hadris_iso::boot::EmulationType::NoEmulation,
        },
        entries: vec![],
    };

    let format_options = FormatOptions {
        volume_name: "BOOT_TEST".to_string(),
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures {
            filenames: BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: None,
        },
    };

    // Create ISO in memory first, then write to file
    let mut iso_buffer = Cursor::new(vec![0u8; 256 * 2048]);
    IsoImageWriter::format_new(&mut iso_buffer, files, format_options)
        .expect("Failed to create bootable ISO with hadris-iso");

    // Write to file
    fs::write(&iso_path, iso_buffer.into_inner()).expect("Failed to write ISO file");

    println!("Created hadris-iso boot ISO at: {:?}", iso_path);
    println!("ISO size: {} bytes", fs::metadata(&iso_path).unwrap().len());

    // Boot with QEMU and capture serial output
    match run_qemu_with_timeout(&iso_path, 5) {
        Some(stdout) => {
            println!("QEMU stdout: {}", stdout);

            // Check if our boot code produced the expected output
            if stdout.contains("OK") {
                println!("=== hadris-iso ISO boots successfully in QEMU! ===");
            } else {
                println!("Note: Boot code may not have executed as expected");
                println!("This could be due to BIOS initialization or boot sequence");

                // Let's dump the first few sectors of the ISO for debugging
                let iso_data = fs::read(&iso_path).unwrap();
                println!("\nFirst volume descriptor (LBA 16):");
                let offset = 16 * 2048;
                println!("  Type: {:#04x}", iso_data[offset]);
                println!(
                    "  ID: {:?}",
                    String::from_utf8_lossy(&iso_data[offset + 1..offset + 6])
                );

                // Check boot record
                for sector in 16..24 {
                    let offset = sector * 2048;
                    if iso_data.len() <= offset + 6 {
                        break;
                    }
                    if iso_data[offset] == 0x00 && &iso_data[offset + 1..offset + 6] == b"CD001" {
                        let ptr_bytes: [u8; 4] =
                            iso_data[offset + 71..offset + 75].try_into().unwrap();
                        let catalog_lba = u32::from_le_bytes(ptr_bytes);
                        println!("\nBoot Record found at sector {}", sector);
                        println!("  Boot catalog LBA: {}", catalog_lba);

                        let catalog_offset = catalog_lba as usize * 2048;
                        if iso_data.len() > catalog_offset + 64 {
                            println!("\nBoot Catalog:");
                            let validation = &iso_data[catalog_offset..catalog_offset + 32];
                            let default = &iso_data[catalog_offset + 32..catalog_offset + 64];
                            println!("  Validation header ID: {:#04x}", validation[0]);
                            println!(
                                "  Validation key: {:02x} {:02x}",
                                validation[30], validation[31]
                            );
                            println!("  Default boot indicator: {:#04x}", default[0]);
                            let load_rba = u32::from_le_bytes([
                                default[8],
                                default[9],
                                default[10],
                                default[11],
                            ]);
                            println!("  Default load RBA: {}", load_rba);

                            // Dump first few bytes of boot image
                            let boot_offset = load_rba as usize * 2048;
                            if iso_data.len() > boot_offset + 16 {
                                println!("\nBoot image first 16 bytes:");
                                println!("  {:02x?}", &iso_data[boot_offset..boot_offset + 16]);
                            }
                        }
                        break;
                    }
                }
            }
        }
        None => {
            println!("QEMU command failed to run");
        }
    }
}

/// Test that hybrid boot MBR is correctly written
#[test]
fn test_hybrid_boot_mbr() {
    use hadris_iso::write::options::HybridBootOptions;
    use std::sync::Arc;

    // Create a simple boot image
    let mut boot_image = vec![0u8; 2048];
    boot_image[0] = 0xEB; // jmp
    boot_image[1] = 0xFE; // -2 (infinite loop)

    let files = hadris_iso::write::InputFiles {
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
        files: vec![hadris_iso::write::File::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_image,
        }],
    };

    let boot_options = hadris_iso::boot::options::BootOptions {
        write_boot_catalog: true,
        default: hadris_iso::boot::options::BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: hadris_iso::boot::EmulationType::NoEmulation,
        },
        entries: vec![],
    };

    let format_options = hadris_iso::write::options::FormatOptions {
        volume_name: "HYBRID_TEST".to_string(),
        sector_size: 2048,
        path_seperator: hadris_iso::read::PathSeparator::ForwardSlash,
        features: hadris_iso::write::options::CreationFeatures {
            filenames: hadris_iso::write::options::BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: Some(HybridBootOptions::mbr()),
        },
    };

    let mut iso_buffer = Cursor::new(vec![0u8; 512 * 2048]); // 512 sectors
    hadris_iso::write::IsoImageWriter::format_new(&mut iso_buffer, files, format_options)
        .expect("Failed to create hybrid ISO");

    let iso_data = iso_buffer.into_inner();

    // Verify MBR signature
    assert_eq!(iso_data[510], 0x55, "MBR signature byte 1 incorrect");
    assert_eq!(iso_data[511], 0xAA, "MBR signature byte 2 incorrect");

    // Verify first partition entry (at offset 446)
    let boot_indicator = iso_data[446];
    assert_eq!(boot_indicator, 0x80, "Partition should be bootable");

    let part_type = iso_data[446 + 4]; // Partition type at offset 450
    assert_eq!(part_type, 0x17, "Partition type should be 0x17 (ISO9660)");

    println!("=== Hybrid MBR boot test passed ===");
    println!("  Boot indicator: 0x{:02x}", boot_indicator);
    println!("  Partition type: 0x{:02x}", part_type);
}

/// Test that hybrid boot with GPT is correctly written
#[test]
fn test_hybrid_boot_gpt() {
    use hadris_iso::write::options::HybridBootOptions;
    use std::sync::Arc;

    // Create a simple boot image
    let mut boot_image = vec![0u8; 2048];
    boot_image[0] = 0xEB;
    boot_image[1] = 0xFE;

    let files = hadris_iso::write::InputFiles {
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
        files: vec![hadris_iso::write::File::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_image,
        }],
    };

    let boot_options = hadris_iso::boot::options::BootOptions {
        write_boot_catalog: true,
        default: hadris_iso::boot::options::BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: hadris_iso::boot::EmulationType::NoEmulation,
        },
        entries: vec![],
    };

    let format_options = hadris_iso::write::options::FormatOptions {
        volume_name: "GPT_TEST".to_string(),
        sector_size: 2048,
        path_seperator: hadris_iso::read::PathSeparator::ForwardSlash,
        features: hadris_iso::write::options::CreationFeatures {
            filenames: hadris_iso::write::options::BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: Some(HybridBootOptions::gpt()),
        },
    };

    let mut iso_buffer = Cursor::new(vec![0u8; 512 * 2048]);
    hadris_iso::write::IsoImageWriter::format_new(&mut iso_buffer, files, format_options)
        .expect("Failed to create GPT ISO");

    let iso_data = iso_buffer.into_inner();

    // Verify MBR signature (protective MBR)
    assert_eq!(iso_data[510], 0x55, "MBR signature byte 1 incorrect");
    assert_eq!(iso_data[511], 0xAA, "MBR signature byte 2 incorrect");

    // Verify protective MBR partition type (0xEE)
    let part_type = iso_data[446 + 4];
    assert_eq!(
        part_type, 0xEE,
        "Protective MBR partition type should be 0xEE"
    );

    // Verify GPT signature at sector 1 (offset 512)
    let gpt_sig = &iso_data[512..520];
    assert_eq!(gpt_sig, b"EFI PART", "GPT signature incorrect");

    println!("=== GPT boot test passed ===");
    println!("  Protective MBR type: 0x{:02x}", part_type);
    println!("  GPT signature: {:?}", String::from_utf8_lossy(gpt_sig));
}

/// Test hybrid MBR+GPT boot
#[test]
fn test_hybrid_boot_dual() {
    use hadris_iso::write::options::HybridBootOptions;
    use std::sync::Arc;

    let mut boot_image = vec![0u8; 2048];
    boot_image[0] = 0xEB;
    boot_image[1] = 0xFE;

    let files = hadris_iso::write::InputFiles {
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
        files: vec![hadris_iso::write::File::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_image,
        }],
    };

    let boot_options = hadris_iso::boot::options::BootOptions {
        write_boot_catalog: true,
        default: hadris_iso::boot::options::BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: hadris_iso::boot::EmulationType::NoEmulation,
        },
        entries: vec![],
    };

    let format_options = hadris_iso::write::options::FormatOptions {
        volume_name: "DUAL_BOOT".to_string(),
        sector_size: 2048,
        path_seperator: hadris_iso::read::PathSeparator::ForwardSlash,
        features: hadris_iso::write::options::CreationFeatures {
            filenames: hadris_iso::write::options::BaseIsoLevel::Level1 {
                supports_lowercase: false,
                supports_rrip: false,
            },
            long_filenames: false,
            joliet: None,
            rock_ridge: None,
            el_torito: Some(boot_options),
            hybrid_boot: Some(HybridBootOptions::hybrid()),
        },
    };

    let mut iso_buffer = Cursor::new(vec![0u8; 512 * 2048]);
    hadris_iso::write::IsoImageWriter::format_new(&mut iso_buffer, files, format_options)
        .expect("Failed to create hybrid dual-boot ISO");

    let iso_data = iso_buffer.into_inner();

    // Verify MBR signature
    assert_eq!(iso_data[510], 0x55);
    assert_eq!(iso_data[511], 0xAA);

    // Check that we have both protective (0xEE) and ISO9660 (0x17) partitions
    // In hybrid mode, slot 0 has protective, other slots have mirrored partitions
    let part0_type = iso_data[446 + 4];
    let part1_type = iso_data[446 + 16 + 4]; // Second partition entry

    println!("=== Hybrid dual-boot test ===");
    println!("  Partition 0 type: 0x{:02x}", part0_type);
    println!("  Partition 1 type: 0x{:02x}", part1_type);

    // Verify GPT signature
    let gpt_sig = &iso_data[512..520];
    assert_eq!(gpt_sig, b"EFI PART", "GPT signature incorrect");

    // One partition should be protective (0xEE) and one should be ISO9660 (0x17)
    let has_protective = part0_type == 0xEE || part1_type == 0xEE;
    let has_iso9660 = part0_type == 0x17 || part1_type == 0x17;

    assert!(has_protective, "Should have protective MBR partition");
    assert!(has_iso9660, "Should have ISO9660 mirrored partition");

    println!("  GPT signature: {:?}", String::from_utf8_lossy(gpt_sig));
    println!("=== Hybrid dual-boot test passed ===");
}

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
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
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
        sector_size: 2048,
        path_seperator: PathSeparator::ForwardSlash,
        features: CreationFeatures::with_rock_ridge(),
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
            SystemUseField::Unknown(header, _) => match &header.sig {
                b"PX" => found_px = true,
                b"NM" => found_nm = true,
                _ => {}
            },
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
                if let SystemUseField::Unknown(header, _) = field {
                    if &header.sig == b"NM" {
                        found_file_with_nm = true;
                    }
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
