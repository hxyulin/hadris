---
title: Open FAT inside a partition
---

# Open FAT inside an MBR or GPT partition

A partition table and the filesystem inside it are separate layers. Read the
table, choose a partition, then restrict all filesystem I/O to that partition's
block range.

```toml
[dependencies]
anyhow = "1"
hadris-block = "2.4.0"
hadris-fs = "2.4.0"
hadris-io = "2.4.0"
```

```rust,no_run
use anyhow::{Context, Result};
use hadris_block::{
    part::{PartitionTable, PartitionTableReadExt},
    storage::{BlockIndex, sync::Slice},
    sync::OpenVolume,
};
use hadris_fs::sync::DriverExt;
use hadris_io::StdIo;
use std::fs::File;

fn main() -> Result<()> {
    const BLOCK_SIZE: u32 = 512;

    let mut stream = StdIo::new(File::open("disk.img")?);
    let table = PartitionTable::read_from(&mut stream, BLOCK_SIZE)?;
    let partition = table
        .partitions()
        .into_iter()
        .next()
        .context("the disk has no partitions")?;

    let mut disk = stream.into_inner();
    let slice = Slice::new(&mut disk, BlockIndex(partition.start_lba), partition.size_sectors)
        .map_err(|_| anyhow::anyhow!("the partition does not fit on the disk"))?;
    let opened = OpenVolume::open(slice)?;
    let mut fat = opened
        .into_fat()
        .ok()
        .context("the selected partition is not FAT")?;

    for entry in fat.read_dir("/")? {
        println!("{}", entry?.name_str().unwrap_or("?"));
    }

    Ok(())
}
```

`std::fs::File` is a block device with 512-byte blocks, so the partition's
start and length in logical blocks become a `Slice` directly. With a
`hadris_part::MbrPartition` or `GptPartitionEntry` in hand,
`hadris_block::partition::sync::mbr_partition(&mut disk, &entry)` and
`gpt_partition` build the same slice, and `partition::r#async` does so for
async devices. The table itself is still read by `hadris-part` from a stream.

Do not seek to the partition offset and then pass the unrestricted disk handle
to a filesystem parser. Filesystem offsets are relative to its start, and an
unbounded handle can allow corrupt metadata to address neighboring partitions.

Use a disk device whose block size is the logical block size the table was
written with. The common value is 512 bytes, but GPT and storage devices are
not universally limited to it.
