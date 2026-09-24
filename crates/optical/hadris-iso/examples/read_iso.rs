//! Prints what an ISO image holds: its trees, volume name, boot catalog and
//! the root directory of its most capable tree.
//!
//! ```text
//! cargo run -p hadris-iso --example read_iso -- image.iso
//! ```

use hadris_fs::{DirCursor, NameBuf};
use hadris_iso::Namespace;
use hadris_iso::sync::IsoImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: read_iso <image.iso>")?;
    let mut iso = IsoImage::open(std::fs::File::open(path)?)?;

    let pvd = iso.primary_descriptor()?;
    println!(
        "Volume: {}",
        String::from_utf8_lossy(pvd.volume_identifier.trimmed())
    );
    println!(
        "Blocks: {} of {} bytes",
        iso.volume_blocks(),
        iso.block_size()
    );
    println!("Trees: {:?}", iso.namespaces().iter().collect::<Vec<_>>());
    if let Some(catalog) = iso.boot_catalog()? {
        for entry in catalog.entries() {
            println!(
                "Boot: {:?} {:?} at block {}",
                entry.platform(),
                entry.emulation(),
                entry.load_block()
            );
        }
    }

    let mut view = iso.view(Namespace::Preferred)?;
    println!("Root of the {:?} tree:", view.namespace());
    let root = view.root();
    let mut cursor = DirCursor::start();
    let mut name = NameBuf::new();
    while let Some(entry) = view.read_dir_entry(root, &mut cursor, &mut name)? {
        let meta = view.node_metadata(entry.node())?;
        println!(
            "  {:?} {:>10} {}",
            meta.file_type(),
            meta.len(),
            String::from_utf8_lossy(name.as_bytes())
        );
    }
    Ok(())
}
