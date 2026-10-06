//! Detects the filesystem of an image (FAT12/16/32, exFAT, ISO 9660 or
//! UDF), opens it with `hadris::host::open`, and prints its tree through
//! one function that works on any `hadris-fs` driver.

use std::path::PathBuf;

use anyhow::{Result, bail};
use hadris::ImageFormat;
use hadris::fs::sync::FileSystem;
use hadris::fs::{DirCursor, NodeId};
use hadris::host::FileDevice;

fn main() -> Result<()> {
    let image_path = image_path()?;
    let mut image = FileDevice::open(&image_path)?;
    let found = hadris::sync::detect(&mut image)?;
    match found.first().map(|candidate| candidate.format()) {
        Some(ImageFormat::Mbr | ImageFormat::Gpt) => {
            bail!("partitioned disk: open one partition with hadris::part::sync::open")
        }
        None => bail!("no supported filesystem detected"),
        Some(format) => println!("{format:?}"),
    }
    let mut fs = hadris::host::open(&image_path)?;
    let root = fs.root();
    print_tree(&mut fs, root, 1)
}

/// Prints the directory `dir` and everything below it. `F` is any
/// `hadris-fs` filesystem: `AnyFs` here, or a `FatFs`, `IsoFs` or `UdfFs`.
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
