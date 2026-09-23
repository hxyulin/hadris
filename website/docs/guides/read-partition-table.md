---
title: Read a partition table
---

# Inspect an MBR or GPT image

```toml
[dependencies]
hadris-io = "2.4.0"
hadris-part = "2.4.0"
```

```rust
use hadris_io::StdIo;
use hadris_part::{
    PartitionInfoTrait, PartitionTable, PartitionTableReadExt,
};
use std::fs::File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut disk = StdIo::new(File::open("disk.img")?);
    let table = PartitionTable::read_from(&mut disk, 512)?;

    for partition in table.partitions() {
        println!(
            "#{}: LBA {} ({} sectors)",
            partition.index,
            partition.start_lba,
            partition.size_sectors,
        );
    }

    Ok(())
}
```

Use a real logical block size instead of assuming 512 bytes when the backing
device reports different geometry. Enable the `crc` feature when GPT CRC
validation is required.

The returned start and size values are expressed in logical blocks. To open a
filesystem safely, convert them with checked arithmetic and create a bounded
partition view. See [Open FAT inside a partition](./open-partitioned-fat.md).
