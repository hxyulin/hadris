mod xorriso_helpers;

use std::io::Cursor;
use std::sync::Arc;

#[test]
fn test_hybrid_boot_mbr() {
    use hadris_iso::write::options::HybridBootOptions;

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
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
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

#[test]
fn test_hybrid_boot_gpt() {
    use hadris_iso::write::options::HybridBootOptions;

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
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
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

#[test]
fn test_hybrid_boot_dual() {
    use hadris_iso::write::options::HybridBootOptions;

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
        path_separator: hadris_iso::read::PathSeparator::ForwardSlash,
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
