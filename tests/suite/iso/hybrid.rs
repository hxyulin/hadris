//! Hybrid MBR/GPT boot sectors written alongside the ISO 9660 image.

use hadris_fs::{Content, Node, Tree};
use hadris_iso::{BootEntry, ElTorito, HybridBoot, IsoOptions, VolumeIdentifiers};
use hadris_tests::iso::hadris::write_tree;

fn hybrid_image(volume_name: &str, hybrid_boot: HybridBoot) -> Vec<u8> {
    let mut boot_image = vec![0u8; 2048];
    boot_image[0] = 0xEB;
    boot_image[1] = 0xFE;
    let mut tree = Tree::new();
    tree.insert("boot.bin", Node::file(Content::bytes(boot_image)))
        .unwrap();
    let options = IsoOptions::default()
        .with_volume(VolumeIdentifiers::new(volume_name))
        .with_el_torito(
            ElTorito::new(BootEntry::new("boot.bin").with_load_size(4))
                .with_catalog_path("boot.catalog"),
        )
        .with_hybrid(hybrid_boot);
    write_tree(&tree, &options).expect("Failed to create hybrid ISO")
}

#[test]
fn test_hybrid_boot_mbr() {
    let iso_data = hybrid_image("HYBRID_TEST", HybridBoot::mbr());
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
    let iso_data = hybrid_image("GPT_TEST", HybridBoot::gpt());
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
    let iso_data = hybrid_image("DUAL_BOOT", HybridBoot::hybrid());
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
