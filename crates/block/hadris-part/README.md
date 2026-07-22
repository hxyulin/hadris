# Hadris Partition

Partition table support for MBR, GPT, and Hybrid MBR.

## Overview

This crate provides read and write support for common partition table formats used in disk images and storage devices.

## Features

- **MBR** - Legacy BIOS partition tables (4 primary partitions)
- **GPT** - Modern UEFI partition tables (128+ partitions with GUIDs)
- **Hybrid MBR** - Combined MBR+GPT for dual BIOS/UEFI boot compatibility
- **No-std Compatible** - Use in bootloaders and embedded systems

## Feature Flags

| Feature | Description | Dependencies | Default |
|---------|-------------|--------------|---------|
| `std` | Standard library support | `alloc` | Yes |
| `read` | Reading partition tables via `*ReadExt` traits | - | Yes |
| `sync` | Synchronous I/O traits | - | Yes |
| `async` | Asynchronous I/O traits | - | - |
| `alloc` | Heap allocation for `Vec`-based APIs (`GptDisk`, `DiskPartitionScheme`) | - | via `std` |
| `write` | Writing partition tables | `alloc`, `read` | - |
| `crc` | CRC32 verification/calculation for GPT headers | `crc` crate | - |
| `rand` | Random GUID generation | `rand` crate | - |

> **Note:** GPT header/entry CRC checks run only when the `crc` feature is enabled.
> Without it, CRC fields are ignored on read.

`std` selects platform integration but does not select an I/O mode. The default
feature set enables `sync` explicitly; custom configurations should enable
`sync`, `async`, or both.

## Usage

### Detecting and reading a partition scheme

Requires features `read` and `alloc` (included when using `std` + `read`):

```rust,no_run
use std::fs::File;
use hadris_part::{
    DiskPartitionScheme, DiskPartitionSchemeReadExt, PartitionInfoTrait,
};

# fn main() -> hadris_part::Result<()> {
let mut disk = File::open("disk.img")?;
let scheme = DiskPartitionScheme::read_from(&mut disk, 512)?;

for part in scheme.partitions() {
    println!(
        "Partition {}: {} sectors at LBA {}",
        part.index,
        part.size_sectors,
        part.start_lba
    );
}
# Ok(())
# }
```

### Reading an MBR directly

```rust,no_run
use std::fs::File;
use hadris_part::{MasterBootRecord, MasterBootRecordReadExt, PartitionInfoTrait};

# fn main() -> hadris_part::Result<()> {
let mut disk = File::open("disk.img")?;
let mbr = MasterBootRecord::read_from(&mut disk)?;

for partition in mbr.get_partition_table().iter() {
    if partition.sector_count.to_ne() == 0 {
        continue;
    }
    println!(
        "Partition: {} sectors at LBA {}",
        partition.size_sectors(),
        partition.start_lba()
    );
}
# Ok(())
# }
```

### Reading a GPT disk

```rust,no_run
use std::fs::File;
use hadris_part::{GptDisk, GptDiskReadExt};

# fn main() -> hadris_part::Result<()> {
let mut disk = File::open("disk.img")?;
let gpt = GptDisk::read_from(&mut disk, 512)?;

for (idx, entry) in gpt.partitions() {
    if entry.is_unused() {
        continue;
    }
    println!(
        "Partition {}: type {:?} first_lba={}",
        idx,
        entry.type_guid,
        entry.first_lba.to_ne()
    );
}
# Ok(())
# }
```

### For Bootloaders (minimal footprint)

```toml
[dependencies]
hadris-part = { version = "2.0.0-rc.3", default-features = false, features = ["read", "sync"] }
```

### For Desktop Applications

```toml
[dependencies]
hadris-part = { version = "2.0.0-rc.3", features = ["write"] }  # read is already default
# Optional GPT CRC verification:
# hadris-part = { version = "2.0.0-rc.3", features = ["write", "crc"] }
```

## Partition Types

### MBR Partition Types

Common partition type IDs:

- `0x00` - Empty
- `0x0B` - FAT32 (CHS)
- `0x0C` - FAT32 (LBA)
- `0x0E` - FAT16 (LBA)
- `0x07` - NTFS/exFAT
- `0x83` - Linux
- `0xEE` - GPT Protective MBR

### GPT Partition GUIDs

Common partition type GUIDs:

- EFI System Partition: `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`
- Microsoft Basic Data: `EBD0A0A2-B9E5-4433-87C0-68B6B72699C7`
- Linux Filesystem: `0FC63DAF-8483-4772-8E79-3D69D8477DE4`

Constants are available on [`Guid`](https://docs.rs/hadris-part/latest/hadris_part/struct.Guid.html) (e.g. `Guid::EFI_SYSTEM`).

## License

Licensed under the [MIT license](../../LICENSE-MIT).
