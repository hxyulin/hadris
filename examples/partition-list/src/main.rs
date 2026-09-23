use std::fs::File;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use hadris_io::StdIo;
use hadris_part::PartitionKind;
use hadris_storage::sync::StreamDevice;
use hadris_storage::{BlockSize, ReadOnly};

fn main() -> Result<()> {
    let (image_path, block_size) = arguments()?;
    let image = File::open(&image_path)
        .with_context(|| format!("failed to open {}", image_path.display()))?;
    let mut device = StreamDevice::new(ReadOnly::new(StdIo::new(image)), block_size)
        .with_context(|| format!("failed to size {}", image_path.display()))?;
    let disk = hadris_part::sync::read(&mut device)
        .with_context(|| format!("failed to read partitions from {}", image_path.display()))?;

    println!(
        "{:?} table, {} blocks of {} bytes",
        disk.table().kind(),
        disk.block_count(),
        block_size.get()
    );
    for partition in disk.partitions() {
        let kind = match partition.kind() {
            PartitionKind::Mbr(kind) => kind.to_string(),
            PartitionKind::Gpt(guid) => guid.to_string(),
            _ => String::from("?"),
        };
        let name = partition
            .name()
            .map(ToString::to_string)
            .unwrap_or_default();
        println!(
            "#{:<3} start {:>12}  blocks {:>12}  {kind:<36}  {name}",
            partition.index(),
            partition.start(),
            partition.len(),
        );
    }

    Ok(())
}

fn arguments() -> Result<(PathBuf, BlockSize)> {
    let mut args = std::env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(path) = args.next() else {
        bail!(
            "usage: {} <disk-image> [logical-block-size]",
            PathBuf::from(program).display()
        );
    };
    let block_size = match args.next() {
        Some(value) => value
            .to_string_lossy()
            .parse()
            .context("logical block size must be an integer")?,
        None => 512,
    };
    if args.next().is_some() {
        bail!("too many arguments");
    }
    let block_size = BlockSize::new(block_size).context("logical block size must not be zero")?;
    Ok((path.into(), block_size))
}
