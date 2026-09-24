use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_fat::MountOptions;
use hadris_fat::sync::FatFs;
use hadris_fs::sync::Volume;
use hadris_storage::host::FileDevice;

fn main() -> Result<()> {
    let image_path = image_path()?;
    let image = FileDevice::open(&image_path)
        .with_context(|| format!("failed to open {}", image_path.display()))?;
    let volume = FatFs::open_with(image, MountOptions::new().with_read_only())
        .with_context(|| format!("failed to open FAT volume {}", image_path.display()))?;
    let vol = Volume::new(volume);

    for entry in vol
        .read_dir("/")
        .context("failed to open the root directory")?
    {
        let entry = entry.context("failed to read a FAT directory entry")?;
        let name = entry.name().to_str().context("entry name is not UTF-8")?;
        let meta = entry.metadata();
        let kind = if meta.file_type().is_dir() {
            "dir "
        } else {
            "file"
        };
        println!("{kind} {:>10} {name}", meta.len());
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
