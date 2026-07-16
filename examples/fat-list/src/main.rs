use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_fat::FatVolume;

fn main() -> Result<()> {
    let image_path = image_path()?;
    let image = File::open(&image_path)
        .with_context(|| format!("failed to open {}", image_path.display()))?;
    let volume = FatVolume::open(image)
        .with_context(|| format!("failed to open FAT volume {}", image_path.display()))?;

    let root = volume.root_dir();
    let mut entries = root.entries();
    while let Some(entry) = entries.next_entry() {
        let entry = entry.context("failed to read a FAT directory entry")?;
        let file = entry
            .as_entry()
            .context("unsupported FAT directory entry type")?;
        let kind = if file.is_directory() { "dir " } else { "file" };
        println!("{kind} {:>10} {}", file.len(), file.name());
    }

    Ok(())
}

fn image_path() -> Result<PathBuf> {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(path) = args.next() else {
        bail!("usage: {} <fat-image>", PathBuf::from(program).display());
    };
    if args.next().is_some() {
        bail!("expected exactly one FAT image path");
    }
    Ok(path.into())
}
