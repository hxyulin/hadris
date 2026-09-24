//! Detects the filesystem of a block image (FAT12/16/32, exFAT or NTFS),
//! opens it with `hadris-block`, and prints its tree through one function
//! that works on any `hadris-fs` driver.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_block::detect::BlockFormat;
use hadris_block::sync::OpenVolume;
use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, NodeId};
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
    let root = volume.root();
    print_tree(&mut volume, root, 1)
}

/// Prints the directory `dir` and everything below it. `F` is any
/// `hadris-fs` filesystem: `OpenVolume` here, or a `FatFs`, `IsoView` or
/// `UdfFs`.
fn print_tree<F: FileSystem>(fs: &mut F, dir: NodeId, depth: usize) -> Result<()> {
    let mut cursor = DirCursor::START;
    while let Some(entry) = fs.readdir(dir, cursor)? {
        cursor = entry.next_cursor();
        let name = String::from_utf8_lossy(entry.name().as_bytes()).into_owned();
        if entry.file_type().is_dir() {
            println!("{:indent$}{name}/", "", indent = depth * 2);
            let child = fs.lookup(dir, entry.name())?;
            let printed = print_tree(fs, child, depth + 1);
            fs.forget(child, 1);
            printed?;
        } else {
            let len = entry.metadata().len();
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
