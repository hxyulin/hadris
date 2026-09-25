//! El Torito boot catalogs: Hadris output, xorriso output, and QEMU boots.

use std::fs;
use std::time::Duration;

use hadris_fs::{Content, Node, Tree};
use hadris_iso::{BootEntry, ElTorito, Emulation, IsoId, IsoOptions, Platform};
use hadris_tests::harness::qemu;
use hadris_tests::iso::hadris::write_tree;
use hadris_tests::iso::xorriso;
use tempfile::TempDir;

use super::{find_boot_catalog, open, validation_checksum};

/// The line the boot code prints, chosen so firmware chatter cannot match it.
const BOOT_MARKER: &str = "HADRIS-OK";

/// x86 code that writes the boot marker and a newline to COM1 and halts.
const SERIAL_OK_BOOT_CODE: [u8; 36] = [
    0xBA, 0xF8, 0x03, // mov dx, 0x3F8
    0xB0, 0x48, // mov al, 'H'
    0xEE, // out dx, al
    0xB0, 0x41, // mov al, 'A'
    0xEE, // out dx, al
    0xB0, 0x44, // mov al, 'D'
    0xEE, // out dx, al
    0xB0, 0x52, // mov al, 'R'
    0xEE, // out dx, al
    0xB0, 0x49, // mov al, 'I'
    0xEE, // out dx, al
    0xB0, 0x53, // mov al, 'S'
    0xEE, // out dx, al
    0xB0, 0x2D, // mov al, '-'
    0xEE, // out dx, al
    0xB0, 0x4F, // mov al, 'O'
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
    let mut tree = Tree::new();
    tree.insert("boot.bin", Node::file(Content::bytes(boot_data)))
        .unwrap();
    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "BOOT_TEST")
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("boot.bin").with_load_size(4))
                .with_catalog_path("boot.catalog"),
        );
    write_tree(&tree, &options).expect("Failed to create bootable ISO with hadris-iso")
}

#[test]
fn test_hadris_multisection_boot_catalog() {
    let mut tree = Tree::new();
    tree.insert("bios.img", Node::file(Content::bytes(vec![0x11; 2048])))
        .unwrap();
    tree.insert("ppc.img", Node::file(Content::bytes(vec![0x22; 2048])))
        .unwrap();
    tree.insert("uefi.img", Node::file(Content::bytes(vec![0x33; 4096])))
        .unwrap();
    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "MULTIBOOT")
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("bios.img").with_load_size(4))
                .with_entry(
                    BootEntry::bios("ppc.img")
                        .with_platform(Platform::PowerPc)
                        .with_load_size(4),
                )
                .with_entry(BootEntry::uefi("uefi.img").with_load_size(8))
                .with_catalog_path("boot.catalog"),
        );
    let output = write_tree(&tree, &options).unwrap();

    let (_, catalog_lba) = find_boot_catalog(&output).expect("boot record volume descriptor");
    let catalog = &output[catalog_lba * 2048..];
    assert_eq!(catalog[64], 0x90);
    assert_eq!(catalog[65], Platform::PowerPc.id());
    assert_eq!(u16::from_le_bytes([catalog[66], catalog[67]]), 1);
    assert_eq!(catalog[128], 0x91);
    assert_eq!(catalog[129], Platform::Efi.id());
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
    let mut tree = Tree::new();
    tree.insert(
        "floppy.img",
        Node::file(Content::bytes(vec![0x44u8; 1_474_560])),
    )
    .unwrap();
    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "FLOPPYBOOT")
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("floppy.img").with_emulation(Emulation::Floppy144))
                .with_catalog_path("boot.catalog"),
        );
    let output = write_tree(&tree, &options).unwrap();

    let (_, catalog_lba) = find_boot_catalog(&output).expect("boot record volume descriptor");
    let catalog = &output[catalog_lba * 2048..];
    assert_eq!(catalog[32], 0x88, "entry must be bootable");
    assert_eq!(
        catalog[33],
        Emulation::Floppy144.media_type(),
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

    let catalog = open(iso_data)
        .boot_catalog()
        .expect("hadris-iso should parse the xorriso boot catalog")
        .expect("the image has a boot catalog");
    assert!(catalog.validation().is_valid());
    assert!(catalog.default_entry().is_bootable());
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
    if !xorriso::require() || !qemu::require() {
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

    let stdout = qemu::boot_serial_output(&iso_path, BOOT_MARKER, Duration::from_secs(30))
        .expect("QEMU command failed to run");
    assert!(
        stdout.contains(BOOT_MARKER),
        "xorriso ISO did not print the boot marker in QEMU; serial output: {stdout:?}"
    );
}

#[test]
#[ignore = "requires QEMU system emulation"]
fn test_qemu_boot_hadris_iso() {
    if !qemu::require() {
        return;
    }
    let temp_dir = TempDir::new().unwrap();
    let iso_path = temp_dir.path().join("hadris_boot.iso");
    let iso_data = hadris_bootable_image(padded_boot_image(&SERIAL_OK_BOOT_CODE));
    fs::write(&iso_path, &iso_data).expect("Failed to write ISO file");

    let stdout = qemu::boot_serial_output(&iso_path, BOOT_MARKER, Duration::from_secs(30))
        .expect("QEMU command failed to run");
    if !stdout.contains(BOOT_MARKER) {
        println!("QEMU stdout: {stdout}");
        if let Some((sector, catalog_lba)) = find_boot_catalog(&iso_data) {
            let catalog_offset = catalog_lba * 2048;
            let default = &iso_data[catalog_offset + 32..catalog_offset + 64];
            let load_rba = u32::from_le_bytes([default[8], default[9], default[10], default[11]]);
            println!("boot record at sector {sector}, catalog LBA {catalog_lba}");
            println!("default entry boot indicator {:#04x}", default[0]);
            println!("default load RBA {load_rba}");
            let boot_offset = load_rba as usize * 2048;
            println!(
                "boot image first 16 bytes: {:02x?}",
                &iso_data[boot_offset..boot_offset + 16]
            );
        }
        panic!("hadris ISO did not print the boot marker in QEMU");
    }
}
