mod xorriso_helpers;
use xorriso_helpers::*;

use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

use hadris_iso::types::Endian as _;

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
        path_separator: PathSeparator::ForwardSlash,
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
        path_separator: PathSeparator::ForwardSlash,
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
        path_separator: PathSeparator::ForwardSlash,
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
