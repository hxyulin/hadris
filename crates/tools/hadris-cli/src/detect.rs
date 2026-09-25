//! `hadris detect`: every format an image or device holds.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris::ImageFormat;
use hadris::host::FileDevice;

#[derive(clap::Args)]
pub struct Args {
    /// Path to the image or device
    image: PathBuf,
}

/// Prints each format `hadris::sync::detect` finds, most specific first,
/// with the damage a mount would report. Fails when nothing is recognized.
pub fn run(args: Args) -> Result<()> {
    let mut dev = FileDevice::open(&args.image)
        .with_context(|| format!("cannot open {}", args.image.display()))?;
    let found = hadris::sync::detect(&mut dev)
        .with_context(|| format!("cannot read {}", args.image.display()))?;
    if found.first().is_none() {
        bail!("no known format in {}", args.image.display());
    }
    for candidate in found.iter() {
        let name = name(candidate.format());
        match candidate.damage() {
            Some(err) => println!("{name} (damaged: {err})"),
            None => println!("{name}"),
        }
    }
    Ok(())
}

fn name(format: ImageFormat) -> String {
    match format {
        ImageFormat::Fat(kind) => format!("{kind:?}").to_uppercase(),
        ImageFormat::ExFat => "exFAT".into(),
        ImageFormat::Iso => "ISO 9660".into(),
        ImageFormat::Udf => "UDF".into(),
        ImageFormat::IsoUdfBridge => "ISO 9660 and UDF bridge".into(),
        ImageFormat::Cpio(format) => format!("cpio ({format:?})"),
        ImageFormat::Mbr => "MBR partition table".into(),
        ImageFormat::Gpt => "GPT partition table".into(),
        ImageFormat::Ntfs => "NTFS".into(),
        other => format!("{other:?}"),
    }
}
