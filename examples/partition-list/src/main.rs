use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_part::{PartitionTable, PartitionTableReadExt};

fn main() -> Result<()> {
    let (image_path, logical_block_size) = arguments()?;
    let mut image = File::open(&image_path)
        .with_context(|| format!("failed to open {}", image_path.display()))?;
    let table = PartitionTable::read_from(&mut image, logical_block_size)
        .with_context(|| format!("failed to read partitions from {}", image_path.display()))?;

    for partition in table.partitions() {
        println!(
            "#{:<3} start LBA {:>12}  sectors {:>12}",
            partition.index, partition.start_lba, partition.size_sectors,
        );
    }

    Ok(())
}

fn arguments() -> Result<(PathBuf, u32)> {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(path) = args.next() else {
        bail!(
            "usage: {} <disk-image> [logical-block-size]",
            PathBuf::from(program).display()
        );
    };
    let logical_block_size = match args.next() {
        Some(value) => value
            .to_string_lossy()
            .parse()
            .context("logical block size must be an integer")?,
        None => 512,
    };
    if args.next().is_some() {
        bail!("too many arguments");
    }
    Ok((path.into(), logical_block_size))
}
