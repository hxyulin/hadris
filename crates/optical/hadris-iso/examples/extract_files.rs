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
    let mut iso = IsoImage::open(std::fs::File::open(image)?)?;
    let mut view = iso.view(Namespace::Preferred)?;
    std::fs::create_dir_all(&target)?;
    hadris_fs::sync::extract_to_host(&mut view, "/", &target)?;
    println!("Extracted the {:?} tree into {target}", view.namespace());
    Ok(())
}
