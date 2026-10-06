//! Prints what an ISO image holds: its trees, volume name, boot catalog and
//! the root directory of its most capable tree.
//!
//! ```text
//! cargo run -p hadris-iso --example read_iso -- image.iso
//! ```

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, MountOptions};
use hadris_iso::IsoId;
use hadris_iso::sync::IsoFs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: read_iso <image.iso>")?;
    let mut iso = IsoFs::mount(
        hadris_storage::host::FileDevice::open(path)?,
        MountOptions::new(),
    )?;

    let info = iso.info();
    println!(
        "Volume: {}",
        String::from_utf8_lossy(info.id(IsoId::Volume))
    );
    println!(
        "Blocks: {} of {} bytes",
        info.volume_space_size(),
        info.block_size()
    );
    println!("Trees: {:?}", iso.namespaces().iter().collect::<Vec<_>>());
    let mut buf = [0u8; 2048];
    if let Some(catalog) = iso.boot_catalog(&mut buf)? {
        for entry in catalog.entries() {
            let image = iso.boot_image(&entry);
            println!(
                "Boot: {:?} {:?}, {} bytes at byte {}",
                entry.platform(),
                entry.emulation(),
                image.len(),
                image.offset()
            );
        }
    }

    let view = &mut iso;
    println!("Root of the {:?} tree:", view.namespace());
    let root = view.root();
    let mut cursor = DirCursor::START;
    while let Some(entry) = view.readdir(root, cursor)? {
        cursor = entry.next_cursor();
        let meta = entry.metadata();
        println!(
            "  {:?} {:>10} {}",
            meta.file_type(),
            meta.len(),
            String::from_utf8_lossy(entry.name().as_bytes())
        );
    }
    Ok(())
}
