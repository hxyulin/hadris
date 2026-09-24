# Hadris Partition

MBR, GPT and hybrid MBR partition tables on `hadris-storage` block devices.

## Overview

`hadris-part` reads, edits and writes partition tables and opens partitions
as block devices for a filesystem driver. It works on any
`hadris_storage` `BlockDevice` and takes the block size from the device, so
512-byte and 4 KiB disks need no special handling.

- **MBR** with extended and logical partitions (EBR chains)
- **GPT** with CRCs always checked and written, and a fallback to the backup
  copy when the primary is damaged
- **Hybrid MBR** that mirrors up to three GPT partitions for BIOS boot
- **UTF-16 partition names**, edits with bounds and overlap checks, and a
  `DiskLayout` builder for whole-disk images
- **`no_std`**, with `alloc` optional, and sync, async and `Send` async APIs
  from one source

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Implies `alloc`; `Guid::random` and `std::io::Error` conversions | Yes |
| `alloc` | `Disk`, `Mbr`, `Gpt`, `Hybrid`, `DiskLayout`, and `read`, `write` and `create` | via `std` |
| `sync` | Blocking API in `hadris_part::sync` | Yes |
| `async` | Asynchronous API in `hadris_part::r#async` | - |
| `async-send` | Asynchronous API with `Send` futures in `hadris_part::async_send` | - |

No feature changes what an item does. CRCs are always computed and checked,
and GUIDs are never generated behind your back: constructors take them, and
`Guid::random` (with `std`) is there when you want one.

## Usage

### Reading a disk and opening a partition

```rust,no_run
use hadris_part::sync::{open, read};
use hadris_part::PartitionKind;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut disk = hadris_storage::host::FileDevice::open("disk.img")?;
let table = read(&mut disk)?;
for p in table.partitions() {
    let name = p.name().map(|n| n.to_string()).unwrap_or_default();
    println!("#{} {} blocks at {} {:?} {name}", p.index(), p.len(), p.start(), p.kind());
}
let esp = table
    .partitions()
    .find(|p| p.kind() == PartitionKind::Gpt(hadris_part::gpt::types::EFI_SYSTEM))
    .expect("an EFI system partition");
let esp_device = open(&mut disk, &esp)?; // a hadris_storage Partition
# let _ = esp_device;
# Ok(())
# }
```

### Creating a disk image

```rust,no_run
use hadris_part::gpt::types;
use hadris_part::{Alignment, DiskLayout, Guid, PartitionSpec, Size};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let file = std::fs::File::options().read(true).write(true).open("disk.img")?;
let mut disk = hadris_storage::host::FileDevice::new(file)?;
let layout = DiskLayout::gpt(Guid::random())
    .with_alignment(Alignment::MiB1)
    .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(100)).with_name("EFI"))
    .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining));
let table = hadris_part::sync::create(&mut disk, &layout)?;
let esp = hadris_part::sync::open(&mut disk, &table.partition(0).unwrap())?;
// hadris_fat::sync::format(esp, hadris_fat::FormatOptions::new())?;
# let _ = esp;
# Ok(())
# }
```

### Editing a table

`Mbr` and `Gpt` edits (`add`, `add_logical`, `remove`, `resize`, `set_*`)
check that the partition stays in the usable area and overlaps no other one,
and change nothing when they fail. `write` puts the table back, recomputing
both GPT copies.

```rust,no_run
use hadris_part::sync::{read, write};
use hadris_part::{GptEntry, Guid, PartitionTable};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let file = std::fs::File::options().read(true).write(true).open("disk.img")?;
let mut dev = hadris_storage::host::FileDevice::new(file)?;
let mut disk = read(&mut dev)?;
if let PartitionTable::Gpt(gpt) = disk.table_mut() {
    gpt.resize(1, 409_600)?;
    gpt.add(GptEntry::new(
        hadris_part::gpt::types::LINUX_SWAP,
        Guid::random(),
        2_000_000,
        1_048_576,
    ))?;
}
write(&mut dev, &disk)?;
# Ok(())
# }
```

### Without an allocator

`scan` in each mode lists partitions through a callback with the same
validation as `read`, backup GPT fallback included, and `open` needs no
allocator either. The on-disk layouts live in `hadris_part::raw`.

```toml
[dependencies]
hadris-part = { version = "2.4.0", default-features = false, features = ["sync"] }
```

## Partition Types

MBR type codes are `MbrType` values with named constants
(`MbrType::FAT32_LBA`, `MbrType::LINUX`, `MbrType::EXTENDED_LBA`,
`MbrType::GPT_PROTECTIVE`, ...); any byte is a valid `MbrType`. GPT type
GUIDs are constants in `hadris_part::gpt::types` (`EFI_SYSTEM`,
`BASIC_DATA`, `LINUX_FILESYSTEM`, ...), and `Guid` parses the usual text
form with `FromStr`, or in `const` context with `Guid::parse_const`.

## Documentation

- [Inspect a partition table](https://hxyulin.github.io/hadris/guides/read-partition-table)
- [Open FAT inside a partition](https://hxyulin.github.io/hadris/guides/open-partitioned-fat)
- [API reference](https://docs.rs/hadris-part)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
