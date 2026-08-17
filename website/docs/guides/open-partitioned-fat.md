---
title: Open FAT inside a partition
---

# Open FAT inside an MBR or GPT partition

A partition table and the filesystem inside it are separate layers. Read the
table, choose a partition, then restrict all filesystem I/O to that partition's
byte range.

```toml
[dependencies]
anyhow = "1"
hadris-block = "2.0.0"
```

```rust,no_run
use anyhow::{Context, Result};
use hadris_block::{
    part::{PartitionTable, PartitionTableReadExt},
    storage::PartitionView,
    sync::OpenVolume,
};
use std::fs::File;

fn main() -> Result<()> {
    const BLOCK_SIZE: u32 = 512;

    let mut disk = File::open("disk.img")?;
    let table = PartitionTable::read_from(&mut disk, BLOCK_SIZE)?;
    let partition = table
        .partitions()
        .into_iter()
        .next()
        .context("the disk has no partitions")?;

    let byte_offset = partition
        .start_lba
        .checked_mul(u64::from(BLOCK_SIZE))
        .context("partition offset overflow")?;
    let byte_len = partition
        .size_sectors
        .checked_mul(u64::from(BLOCK_SIZE))
        .context("partition length overflow")?;

    let mut view = PartitionView::new(&mut disk, byte_offset, byte_len)?;
    let opened = OpenVolume::open(&mut view, BLOCK_SIZE)?;
    let fat = opened.as_fat().context("the selected partition is not FAT")?;

    let root = fat.root_dir();
    let mut entries = root.entries();
    while let Some(entry) = entries.next_entry() {
        let entry = entry?;
        let file = entry.as_entry().context("unsupported directory record")?;
        println!("{}", file.name());
    }

    Ok(())
}
```

Do not seek to the partition offset and then pass the unrestricted disk handle
to a filesystem parser. Filesystem offsets are relative to its start, and an
unbounded handle can allow corrupt metadata to address neighboring partitions.

Use the logical block size reported by the device. The common value is 512
bytes, but GPT and storage devices are not universally limited to it.
