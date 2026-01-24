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
| `alloc` | Heap allocation for Vec-based APIs | - | - |
| `read` | Reading partition tables | - | - |
| `write` | Writing partition tables | `alloc`, `read` | - |

## Usage

### Reading Partition Tables

```rust
use hadris_part::{Mbr, Gpt};

// Read MBR
let mbr = Mbr::read(&mut disk)?;
for partition in mbr.partitions() {
    println!("Partition: {} sectors at LBA {}",
             partition.sector_count(),
             partition.start_lba());
}

// Read GPT
let gpt = Gpt::read(&mut disk)?;
for entry in gpt.entries() {
    println!("Partition: {} - {}",
             entry.name(),
             entry.partition_type_guid());
}
```

### For Bootloaders (minimal footprint)

```toml
[dependencies]
hadris-part = { version = "0.2", default-features = false, features = ["read"] }
```

### For Desktop Applications

```toml
[dependencies]
hadris-part = { version = "0.2", features = ["read", "write"] }
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

## License

Licensed under the [MIT license](../../LICENSE-MIT).
