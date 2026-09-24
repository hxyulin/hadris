//! Writes a bootable hybrid image: El Torito for BIOS and UEFI from optical
//! media, and a hybrid MBR and GPT for USB sticks.
//!
//! ```text
//! cargo run -p hadris-iso --example create_bootable_iso -- bootable.iso
//! ```

use hadris_fs::tree::{Content, Tree};
use hadris_iso::{
    BootEntry, BootInfo, ElTorito, HybridBoot, IsoLevel, IsoOptions, JolietLevel, Platform,
    RockRidge, VolumeIdentifiers,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bootable.iso".into());

    let mut tree = Tree::new();
    tree.add_file("boot/bios.img", Content::bytes(vec![0u8; 2048]))?;
    tree.add_file("boot/efi.img", Content::bytes(vec![0u8; 1 << 20]))?;
    tree.add_file("README.txt", Content::bytes("A hadris-iso example image\n"))?;

    let options = IsoOptions::default()
        .with_volume(VolumeIdentifiers::new("HADRIS_BOOT"))
        .with_level(IsoLevel::L3)
        .with_joliet(JolietLevel::L3)
        .with_rock_ridge(RockRidge::default())
        .with_el_torito(
            ElTorito::new(
                BootEntry::new("boot/bios.img")
                    .with_load_size(4)
                    .with_boot_info_table(BootInfo::Standard),
            )
            .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi))
            .with_catalog_path("boot/boot.cat"),
        )
        .with_hybrid(HybridBoot::hybrid());

    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    let report = hadris_iso::sync::write(file, &tree, &options)?;
    println!("Wrote {path}: {} bytes", report.size_bytes());
    for warning in report.warnings() {
        println!("warning: {warning}");
    }
    Ok(())
}
