//! Hybrid MBR/GPT boot sectors written alongside the ISO 9660 image.

use std::io::Cursor;
use std::sync::Arc;

use hadris_iso::boot::EmulationType;
use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
use hadris_iso::read::PathSeparator;
use hadris_iso::write::options::{
    BaseIsoLevel, CreationFeatures, HybridBootOptions, IsoFormatOptions,
};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};

fn hybrid_image(volume_name: &str, hybrid_boot: HybridBootOptions) -> Vec<u8> {
    let mut boot_image = vec![0u8; 2048];
    boot_image[0] = 0xEB;
    boot_image[1] = 0xFE;
    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![IsoFile::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_image,
        }],
    };
    let boot_options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(std::num::NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: vec![],
    };
    let format_options = IsoFormatOptions {
        volume_name: volume_name.to_string(),
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
            el_torito: Some(boot_options),
            hybrid_boot: Some(hybrid_boot),
        },
        strict_charset: false,
    };
    let mut iso_buffer = Cursor::new(vec![0u8; 512 * 2048]);
    IsoImageWriter::create(&mut iso_buffer, files, format_options)
        .expect("Failed to create hybrid ISO");
    iso_buffer.into_inner()
}

#[test]
fn test_hybrid_boot_mbr() {
    let iso_data = hybrid_image("HYBRID_TEST", HybridBootOptions::mbr());
    assert_eq!(iso_data[510], 0x55, "MBR signature byte 1 incorrect");
    assert_eq!(iso_data[511], 0xAA, "MBR signature byte 2 incorrect");
    assert_eq!(iso_data[446], 0x80, "Partition should be bootable");
    assert_eq!(
        iso_data[446 + 4],
        0x17,
        "Partition type should be 0x17 (ISO9660)"
    );
}

#[test]
fn test_hybrid_boot_gpt() {
    let iso_data = hybrid_image("GPT_TEST", HybridBootOptions::gpt());
    assert_eq!(iso_data[510], 0x55, "MBR signature byte 1 incorrect");
    assert_eq!(iso_data[511], 0xAA, "MBR signature byte 2 incorrect");
    assert_eq!(
        iso_data[446 + 4],
        0xEE,
        "Protective MBR partition type should be 0xEE"
    );
    assert_eq!(&iso_data[512..520], b"EFI PART", "GPT signature incorrect");
}

#[test]
fn test_hybrid_boot_dual() {
    let iso_data = hybrid_image("DUAL_BOOT", HybridBootOptions::hybrid());
    assert_eq!(iso_data[510], 0x55);
    assert_eq!(iso_data[511], 0xAA);
    assert_eq!(&iso_data[512..520], b"EFI PART", "GPT signature incorrect");

    let part0_type = iso_data[446 + 4];
    let part1_type = iso_data[446 + 16 + 4];
    let has_protective = part0_type == 0xEE || part1_type == 0xEE;
    let has_iso9660 = part0_type == 0x17 || part1_type == 0x17;
    assert!(has_protective, "Should have protective MBR partition");
    assert!(has_iso9660, "Should have ISO9660 mirrored partition");
}
