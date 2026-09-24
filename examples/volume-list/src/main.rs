//! Detects the filesystem of a block image (FAT12/16/32, exFAT or NTFS),
//! opens it with `hadris-block`, and prints its tree through one function
//! that works on any `hadris-fs` driver.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_block::detect::BlockFormat;
use hadris_block::sync::OpenVolume;
use hadris_fs::FileType;
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_storage::host::FileDevice;

fn main() -> Result<()> {
    let image_path = image_path()?;
    let mut image = FileDevice::open(&image_path)
        .with_context(|| format!("failed to open {}", image_path.display()))?;

    match hadris_block::detect::sync::detect(&mut image)? {
        Some(BlockFormat::PartitionTable(kind)) => {
            bail!("{kind:?} partitioned disk: open one partition with hadris_part::sync::open")
        }
        None => bail!("no supported filesystem detected"),
        Some(_) => {}
    }
    let mut volume = OpenVolume::open(image).map_err(|err| err.into_error())?;
    println!("{:?}", volume.format());
    print_tree(&mut volume, "/", 1)
}

/// Prints the directory at `path` and everything below it. `D` is any
/// `hadris-fs` driver: `OpenVolume` here, or a `FatFs`, `IsoView` or `UdfFs`.
fn print_tree<D: FsDriver>(fs: &mut D, path: &str, depth: usize) -> Result<()> {
    let mut children = Vec::new();
    for entry in fs.read_dir(path)? {
        let entry = entry?;
        let name = String::from_utf8_lossy(entry.name_bytes()).into_owned();
        children.push((name, entry.file_type()));
    }
    for (name, file_type) in children {
        let child = format!("{}/{name}", path.trim_end_matches('/'));
        if file_type == FileType::Dir {
            println!("{:indent$}{name}/", "", indent = depth * 2);
            print_tree(fs, &child, depth + 1)?;
        } else {
            let len = fs.metadata(&child)?.len();
            println!("{:indent$}{name} ({len} bytes)", "", indent = depth * 2);
        }
    }
    Ok(())
}

fn image_path() -> Result<PathBuf> {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(path) = args.next() else {
        bail!("usage: {} <disk-image>", PathBuf::from(program).display());
    };
    if args.next().is_some() {
        bail!("expected exactly one disk image path");
    }
    Ok(path.into())
}
