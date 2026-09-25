//! Copies every file of an ISO image into a host directory.
//!
//! ```text
//! cargo run -p hadris-iso --example extract_files -- image.iso out/
//! ```

use hadris_iso::Namespace;
use hadris_iso::sync::IsoImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(image), Some(target)) = (args.next(), args.next()) else {
        return Err("usage: extract_files <image.iso> <directory>".into());
    };
    let iso = IsoImage::open(hadris_storage::host::FileDevice::open(image)?)?;
    let view = iso.into_view(Namespace::Preferred)?;
    let namespace = view.namespace();
    let vol = hadris_fs::sync::Volume::new(view);
    let tree = hadris_fs::sync::read_tree(&vol, "/")?;
    std::fs::create_dir_all(&target)?;
    let report = hadris_fs::host::write_tree(&target, &tree)?;
    println!("Extracted the {namespace:?} tree into {target}");
    for warning in report.warnings() {
        println!("warning: {warning}");
    }
    Ok(())
}
