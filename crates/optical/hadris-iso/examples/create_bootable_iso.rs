//! Writes a bootable hybrid image: El Torito for BIOS and UEFI from optical
//! media, and a hybrid MBR and GPT for USB sticks.
//!
//! ```text
//! cargo run -p hadris-iso --example create_bootable_iso -- bootable.iso
//! ```

use hadris_fs::{Content, Node, Tree};
use hadris_iso::{BootEntry, BootInfo, ElTorito, Hybrid, IsoId, IsoLevel, IsoOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "bootable.iso".into());

    let mut tree = Tree::new();
    tree.insert("boot/bios.img", Node::file(Content::bytes(vec![0u8; 2048])))?;
    tree.insert(
        "boot/efi.img",
        Node::file(Content::bytes(vec![0u8; 1 << 20])),
    )?;
    tree.insert(
        "README.txt",
        Node::file(Content::bytes("A hadris-iso example image\n")),
    )?;

    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "HADRIS_BOOT")
        .with_level(IsoLevel::L3)
        .with_joliet()
        .with_rock_ridge()
        .with_el_torito(
            ElTorito::new()
                .with_entry(
                    BootEntry::bios("boot/bios.img")
                        .with_load_size(4)
                        .with_boot_info(BootInfo::Table),
                )
                .with_entry(BootEntry::uefi("boot/efi.img"))
                .with_catalog_path("boot/boot.cat"),
        )
        .with_hybrid(Hybrid::gpt_hybrid_mbr());

    let file = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    let report = hadris_iso::sync::write(
        hadris_storage::host::FileDevice::new(file)?,
        &tree,
        &options,
    )?;
    println!("Wrote {path}: {} bytes", report.size());
    for warning in report.warnings() {
        println!("warning: {warning}");
    }
    Ok(())
}
