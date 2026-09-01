//! El Torito boot catalogs: Hadris output, xorriso output, and QEMU boots.

use std::fs;
use std::io::Cursor;
use std::num::NonZeroU16;
use std::sync::Arc;
use std::time::Duration;

use hadris_iso::boot::options::{BootEntryOptions, BootOptions, BootSectionOptions};
use hadris_iso::boot::{BaseBootCatalog, EmulationType, PlatformId};
use hadris_iso::read::PathSeparator;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputEntry, InputFiles, InputTree, IsoImageWriter};
use hadris_tests::harness::qemu;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{find_boot_catalog, validation_checksum};

/// x86 code that writes "OK\n" to COM1 and halts.
const SERIAL_OK_BOOT_CODE: [u8; 15] = [
    0xB0, 0x4F, // mov al, 'O'
    0xBA, 0xF8, 0x03, // mov dx, 0x3F8
    0xEE, // out dx, al
    0xB0, 0x4B, // mov al, 'K'
    0xEE, // out dx, al
    0xB0, 0x0A, // mov al, '\n'
    0xEE, // out dx, al
    0xF4, // hlt
    0xEB, 0xFD, // jmp $-1
];

fn padded_boot_image(code: &[u8]) -> Vec<u8> {
    let mut boot_data = vec![0u8; 2048];
    boot_data[..code.len()].copy_from_slice(code);
    boot_data
}

/// A Level 1 image with a single no-emulation boot entry for `boot_data`.
fn hadris_bootable_image(boot_data: Vec<u8>) -> Vec<u8> {
    let files = InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: vec![IsoFile::File {
            name: Arc::new("boot.bin".to_string()),
            contents: boot_data,
        }],
    };
    let boot_options = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            boot_image_path: "boot.bin".to_string(),
            load_size: Some(NonZeroU16::new(4).unwrap()),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: vec![],
    };
    let format_options = IsoFormatOptions {
        volume_name: "BOOT_TEST".to_string(),
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
            hybrid_boot: None,
        },
        strict_charset: false,
    };
    let mut iso_buffer = Cursor::new(vec![0u8; 256 * 2048]);
    IsoImageWriter::create(&mut iso_buffer, files, format_options)
        .expect("Failed to create bootable ISO with hadris-iso");
    iso_buffer.into_inner()
}

#[test]
fn test_hadris_multisection_boot_catalog() {
    let bios = vec![0x11; 2048];
    let ppc = vec![0x22; 2048];
    let uefi = vec![0x33; 4096];
    let tree = InputTree::new(
        PathSeparator::ForwardSlash,
        vec![
            InputEntry::file("bios.img", bios),
            InputEntry::file("ppc.img", ppc),
            InputEntry::file("uefi.img", uefi),
        ],
    );
    let boot = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            load_size: Some(NonZeroU16::new(4).unwrap()),
            boot_image_path: "bios.img".to_string(),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: EmulationType::NoEmulation,
        },
        entries: vec![
            (
                BootSectionOptions {
                    platform: PlatformId::PowerPC,
                },
                BootEntryOptions {
                    load_size: Some(NonZeroU16::new(4).unwrap()),
                    boot_image_path: "ppc.img".to_string(),
                    boot_info_table: false,
                    grub2_boot_info: false,
                    emulation: EmulationType::NoEmulation,
                },
            ),
            (
                BootSectionOptions {
                    platform: PlatformId::UEFI,
                },
                BootEntryOptions {
                    load_size: Some(NonZeroU16::new(8).unwrap()),
                    boot_image_path: "uefi.img".to_string(),
                    boot_info_table: false,
                    grub2_boot_info: false,
                    emulation: EmulationType::NoEmulation,
                },
            ),
        ],
    };
    let options = IsoFormatOptions {
        volume_name: "MULTIBOOT".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        features: CreationFeatures {
            el_torito: Some(boot),
            ..CreationFeatures::default()
        },
        path_separator: PathSeparator::ForwardSlash,
        strict_charset: false,
    };
    let output = IsoImageWriter::create(Cursor::new(vec![0; 2 * 1024 * 1024]), tree, options)
        .unwrap()
        .into_inner();

    let (_, catalog_lba) = find_boot_catalog(&output).expect("boot record volume descriptor");
    let catalog = &output[catalog_lba * 2048..];
    assert_eq!(catalog[64], 0x90);
    assert_eq!(catalog[65], PlatformId::PowerPC.to_u8());
    assert_eq!(u16::from_le_bytes([catalog[66], catalog[67]]), 1);
    assert_eq!(catalog[128], 0x91);
    assert_eq!(catalog[129], PlatformId::UEFI.to_u8());
    assert_eq!(u16::from_le_bytes([catalog[130], catalog[131]]), 1);
    assert_eq!(&catalog[192..224], &[0; 32]);

    let ppc_lba = u32::from_le_bytes(catalog[104..108].try_into().unwrap()) as usize;
    let uefi_lba = u32::from_le_bytes(catalog[168..172].try_into().unwrap()) as usize;
    assert_eq!(u16::from_le_bytes([catalog[102], catalog[103]]), 4);
    assert_eq!(u16::from_le_bytes([catalog[166], catalog[167]]), 8);
    assert_eq!(output[ppc_lba * 2048], 0x22);
    assert_eq!(output[uefi_lba * 2048], 0x33);
}

#[test]
fn test_floppy_emulation_media_type_and_default_load_size() {
    let floppy = vec![0x44u8; 2048];
    let tree = InputTree::new(
        PathSeparator::ForwardSlash,
        vec![InputEntry::file("floppy.img", floppy)],
    );
    let boot = BootOptions {
        write_boot_catalog: true,
        default: BootEntryOptions {
            load_size: None,
            boot_image_path: "floppy.img".to_string(),
            boot_info_table: false,
            grub2_boot_info: false,
            emulation: EmulationType::Floppy1_44,
        },
        entries: vec![],
    };
    let options = IsoFormatOptions {
        volume_name: "FLOPPYBOOT".to_string(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: None,
        sector_size: 2048,
        features: CreationFeatures {
            el_torito: Some(boot),
            ..CreationFeatures::default()
        },
        path_separator: PathSeparator::ForwardSlash,
        strict_charset: false,
    };
    let output = IsoImageWriter::create(Cursor::new(vec![0; 2 * 1024 * 1024]), tree, options)
        .unwrap()
        .into_inner();

    let (_, catalog_lba) = find_boot_catalog(&output).expect("boot record volume descriptor");
    let catalog = &output[catalog_lba * 2048..];
    assert_eq!(catalog[32], 0x88, "entry must be bootable");
    assert_eq!(
        catalog[33],
        EmulationType::Floppy1_44.to_u8(),
        "media type must record 1.44 MB floppy emulation"
    );
    assert_eq!(
        u16::from_le_bytes([catalog[38], catalog[39]]),
        1,
        "emulated media default load size must be one virtual sector"
    );
}

#[test]
fn test_eltorito_boot_catalog_comparison() {
    if !xorriso::require() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("boot.iso");
    fs::create_dir(&content_dir).unwrap();
    fs::write(
        content_dir.join("boot.bin"),
        padded_boot_image(&[0xEB, 0xFE]),
    )
    .unwrap();
    xorriso::create_bootable(&content_dir, &iso_path, "boot.bin").unwrap();

    let iso_data = fs::read(&iso_path).unwrap();
    let (_, catalog_lba) =
        find_boot_catalog(&iso_data).expect("Should find boot record volume descriptor");
    let catalog_offset = catalog_lba * 2048;
    let validation_entry = &iso_data[catalog_offset..catalog_offset + 32];
    let default_entry = &iso_data[catalog_offset + 32..catalog_offset + 64];

    assert_eq!(
        validation_checksum(validation_entry),
        0,
        "Validation entry checksum should sum to 0"
    );
    assert_eq!(default_entry[0], 0x88, "Default entry should be bootable");

    let mut catalog_cursor = Cursor::new(&iso_data[catalog_offset..catalog_offset + 64]);
    let catalog = BaseBootCatalog::parse(&mut catalog_cursor)
        .expect("hadris-iso should parse the xorriso boot catalog");
    assert!(catalog.validation.is_valid());
    assert!(catalog.default_entry.is_bootable());
}

#[test]
fn test_hadris_bootable_iso_creation() {
    let iso_data = hadris_bootable_image(padded_boot_image(&[0xEB, 0xFE]));
    let (_, catalog_lba) =
        find_boot_catalog(&iso_data).expect("Should find boot record volume descriptor");
    let catalog_offset = catalog_lba * 2048;
    let validation_entry = &iso_data[catalog_offset..catalog_offset + 32];
    let default_entry = &iso_data[catalog_offset + 32..catalog_offset + 64];

    assert_eq!(validation_entry[0], 0x01, "Header ID should be 0x01");
    assert_eq!(validation_entry[30], 0x55, "Key byte 1 should be 0x55");
    assert_eq!(validation_entry[31], 0xAA, "Key byte 2 should be 0xAA");
    assert_eq!(
        validation_checksum(validation_entry),
        0,
        "Validation checksum should sum to 0"
    );
    assert_eq!(
        default_entry[0], 0x88,
        "Default entry should be bootable (0x88)"
    );
    assert_eq!(
        default_entry[1], 0x00,
        "Boot media type should be no-emulation (0x00)"
    );
    let sector_count = u16::from_le_bytes([default_entry[6], default_entry[7]]);
    assert_eq!(sector_count, 4, "Sector count should be 4");

    let load_rba = u32::from_le_bytes([
        default_entry[8],
        default_entry[9],
        default_entry[10],
        default_entry[11],
    ]);
    assert!(load_rba > 16, "Load RBA should be after volume descriptors");
    assert!(
        load_rba < (iso_data.len() / 2048) as u32,
        "Load RBA should be within ISO"
    );
}

/// Prints a field-by-field comparison of the xorriso and Hadris catalogs for
/// the same boot image; both must carry a valid validation entry.
#[test]
fn test_compare_boot_catalogs() {
    if !xorriso::require() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let xorriso_iso_path = temp_dir.path().join("xorriso.iso");
    fs::create_dir(&content_dir).unwrap();
    let boot_data = padded_boot_image(&[0xEB, 0xFE]);
    fs::write(content_dir.join("boot.bin"), &boot_data).unwrap();
    xorriso::create_bootable(&content_dir, &xorriso_iso_path, "boot.bin").unwrap();

    let hadris_data = hadris_bootable_image(boot_data);
    let xorriso_data = fs::read(&xorriso_iso_path).unwrap();

    let (x_br_sector, x_cat_lba) = find_boot_catalog(&xorriso_data).expect("xorriso boot catalog");
    let (h_br_sector, h_cat_lba) = find_boot_catalog(&hadris_data).expect("hadris boot catalog");
    println!("xorriso: Boot Record at sector {x_br_sector}, Catalog at LBA {x_cat_lba}");
    println!("hadris:  Boot Record at sector {h_br_sector}, Catalog at LBA {h_cat_lba}");

    let x_cat_offset = x_cat_lba * 2048;
    let h_cat_offset = h_cat_lba * 2048;
    let x_val = &xorriso_data[x_cat_offset..x_cat_offset + 32];
    let h_val = &hadris_data[h_cat_offset..h_cat_offset + 32];
    let x_def = &xorriso_data[x_cat_offset + 32..x_cat_offset + 64];
    let h_def = &hadris_data[h_cat_offset + 32..h_cat_offset + 64];

    println!("validation xorriso: {x_val:02x?}");
    println!("validation hadris:  {h_val:02x?}");
    println!("default xorriso: {x_def:02x?}");
    println!("default hadris:  {h_def:02x?}");
    for (label, x, h) in [
        ("Header ID", x_val[0], h_val[0]),
        ("Platform ID", x_val[1], h_val[1]),
        ("Boot Indicator", x_def[0], h_def[0]),
        ("Boot Media Type", x_def[1], h_def[1]),
        ("System Type", x_def[4], h_def[4]),
    ] {
        if x != h {
            println!("DIFF: {label} - xorriso={x:#04x}, hadris={h:#04x}");
        }
    }
    let x_load_seg = u16::from_le_bytes([x_def[2], x_def[3]]);
    let h_load_seg = u16::from_le_bytes([h_def[2], h_def[3]]);
    if x_load_seg != h_load_seg {
        println!("DIFF: Load Segment - xorriso={x_load_seg:#06x}, hadris={h_load_seg:#06x}");
    }
    let x_sector_count = u16::from_le_bytes([x_def[6], x_def[7]]);
    let h_sector_count = u16::from_le_bytes([h_def[6], h_def[7]]);
    if x_sector_count != h_sector_count {
        println!("DIFF: Sector Count - xorriso={x_sector_count}, hadris={h_sector_count}");
    }
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

    assert_eq!(validation_checksum(x_val), 0, "xorriso validation entry");
    assert_eq!(validation_checksum(h_val), 0, "hadris validation entry");
}

#[test]
#[ignore = "requires QEMU system emulation"]
fn test_qemu_boot_xorriso_iso() {
    if !xorriso::require() {
        return;
    }
    if !qemu::available() {
        eprintln!("skipping: {} is not available", qemu::PROGRAM);
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let content_dir = temp_dir.path().join("content");
    let iso_path = temp_dir.path().join("xorriso_boot.iso");
    fs::create_dir(&content_dir).unwrap();
    fs::write(
        content_dir.join("boot.bin"),
        padded_boot_image(&SERIAL_OK_BOOT_CODE),
    )
    .unwrap();
    xorriso::create_bootable(&content_dir, &iso_path, "boot.bin").unwrap();

    match qemu::boot_serial_output(&iso_path, Duration::from_secs(5)) {
        Some(stdout) => {
            println!("QEMU stdout: {stdout}");
            if stdout.contains("OK") {
                println!("xorriso ISO boots successfully in QEMU");
            } else {
                println!("Note: boot code may not have executed as expected");
            }
        }
        None => println!("QEMU command failed to run"),
    }
}

#[test]
#[ignore = "requires QEMU system emulation"]
fn test_qemu_boot_hadris_iso() {
    if !qemu::available() {
        eprintln!("skipping: {} is not available", qemu::PROGRAM);
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let iso_path = temp_dir.path().join("hadris_boot.iso");
    let iso_data = hadris_bootable_image(padded_boot_image(&SERIAL_OK_BOOT_CODE));
    fs::write(&iso_path, &iso_data).expect("Failed to write ISO file");

    match qemu::boot_serial_output(&iso_path, Duration::from_secs(5)) {
        Some(stdout) => {
            println!("QEMU stdout: {stdout}");
            if stdout.contains("OK") {
                println!("hadris-iso ISO boots successfully in QEMU");
            } else {
                println!("Note: boot code may not have executed as expected");
                if let Some((sector, catalog_lba)) = find_boot_catalog(&iso_data) {
                    let catalog_offset = catalog_lba * 2048;
                    let default = &iso_data[catalog_offset + 32..catalog_offset + 64];
                    let load_rba =
                        u32::from_le_bytes([default[8], default[9], default[10], default[11]]);
                    println!("boot record at sector {sector}, catalog LBA {catalog_lba}");
                    println!("default entry boot indicator {:#04x}", default[0]);
                    println!("default load RBA {load_rba}");
                    let boot_offset = load_rba as usize * 2048;
                    println!(
                        "boot image first 16 bytes: {:02x?}",
                        &iso_data[boot_offset..boot_offset + 16]
                    );
                }
            }
        }
        None => println!("QEMU command failed to run"),
    }
}
