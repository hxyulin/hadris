---
title: Read a partition table
---

# Inspect an MBR or GPT image

```toml
[dependencies]
hadris-part = "2.4.0"
hadris-storage = "2.4.0"
```

```rust
use hadris_part::sync::read;
use hadris_part::{PartitionKind, PartitionTable};
use hadris_storage::host::FileDevice;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut disk = FileDevice::open("disk.img")?;
    let table = read(&mut disk)?;

    if let PartitionTable::Gpt(gpt) = table.table() {
        if let Some(copy) = gpt.damaged_copy() {
            println!("the {copy:?} GPT is damaged; writing the table repairs it");
        }
    }
    for partition in table.partitions() {
        let kind = match partition.kind() {
            PartitionKind::Mbr(kind) => kind.to_string(),
            PartitionKind::Gpt(guid) => guid.to_string(),
            _ => String::from("unknown"),
        };
        let name = partition.name().map(|n| n.to_string()).unwrap_or_default();
        println!(
            "#{}: block {} ({} blocks, {} bytes) {kind} {name}",
            partition.index(),
            partition.start(),
            partition.len(),
            partition.size_bytes(),
        );
    }

    Ok(())
}
```

The block size comes from the device: `host::FileDevice` has 512-byte blocks,
and a `hadris_storage::sync::StreamDevice` over any stream takes the block
size you give it, such as 4096 for a 4Kn disk image. GPT CRCs are always
checked; when the primary copy is damaged the table is read from the backup,
and `damaged_copy` says so. MBR logical partitions in an extended partition
are listed from index 4, as Linux numbers them.

Without an allocator, `hadris_part::sync::scan` lists the same partitions
through a callback. To open a filesystem inside one of them, see
[Open FAT inside a partition](./open-partitioned-fat.md).
