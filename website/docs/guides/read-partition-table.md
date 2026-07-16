---
title: Read a partition table
---

# Inspect an MBR or GPT image

```toml
[dependencies]
hadris-part = "2.0.0-rc.1"
```

```rust
use hadris_part::{
    DiskPartitionScheme, DiskPartitionSchemeReadExt, PartitionInfoTrait,
};
use std::fs::File;

fn main() -> hadris_part::Result<()> {
    let mut disk = File::open("disk.img")?;
    let table = DiskPartitionScheme::read_from(&mut disk, 512)?;

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
