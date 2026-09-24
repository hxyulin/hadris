# Hadris FAT

A modern Rust FAT12, FAT16, FAT32 and exFAT filesystem library with read,
write, format and check support. Hadris FAT handles VFAT long filenames and targets desktop disk
image tools as well as `no_std` bootloaders, kernels, firmware, embedded
systems, SD cards, and USB drives.

## Features

- **FAT12/16/32 Support** - Full read and write support for all FAT variants
- **Volume Formatting** - Create new FAT12/16/32 volumes with automatic type selection
- **Long Filenames (VFAT/LFN)** - Always read and written
- **No-std Compatible** - Use in bootloaders and custom kernels
- **No allocator needed** - Read, write, format and check without `alloc`
- **Sync, async and `Send` async** - One driver generated for each mode
- **Checker** - A read-only `fsck` that reports each problem it finds
- **exFAT** - `ExFatFs` reads, writes, formats and checks exFAT, including TexFAT volumes with two FATs

## Quick Start

### The `FatFs` Driver

`FatFs` is the node-based driver, available as
`sync::FatFs`, `r#async::FatFs` and `async_send::FatFs`. It mounts any
`hadris-storage` block device, needs no allocator (its only buffer is one
device block of at most 4096 bytes), and implements the `hadris-fs`
`FsDriver` trait, so `Volume`, the path helpers and the `File`/`Dir` handles
of `hadris-fs` work on it. Long names are always read and written: a name
that fits 8.3 in one case per part is stored as a short entry alone, and
other names get long-name entries and a short name with a `~N` tail.

```rust,no_run
use hadris_fat::sync::FatFs;
use hadris_fs::sync::{FileSystem, PathExt, Volume};
use hadris_fs::Name;
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let image = std::fs::read("disk.img")?;
let mut fs = FatFs::open(MemDevice::new(image, BlockSize::new(512).unwrap()))?;

// Raw tier: node ids, no locks. `lookup` pins, `forget` unpins.
let root = fs.root();
let efi = fs.lookup(root, Name::new("efi")?)?;
fs.forget(efi);

// Shared tier: paths and handles.
let vol = Volume::new(fs);
for entry in vol.read_dir("/EFI")? {
    println!("{}", entry?.name_str().unwrap_or("?"));
}
vol.create_dir_all("/logs")?;
vol.write_file("/logs/boot.txt", b"booted")?;
vol.sync()?;
# Ok(())
# }
```

`FatFs<D, T, C, P>` takes three type parameters after the device, each
with a zero-sized or allocation-free default, chosen through `MountOptions`
and `FatFs::open_with`:

- `T`, the node table of open nodes, `FixedTable<64>` by default. A full
  table makes `lookup` fail with `ErrorKind::LimitExceeded`; use
  `HeapTable::new()` or a larger `FixedTable<N>` for more.
- `C`, the `hadris_fs::Clock` for new and modified entries. `NoClock`, the
  default, writes 1980-01-01 so images are reproducible; `SystemClock`
  (`std`) writes the current UTC time.
- `P`, the `CodePage` of short names. `Ascii`, the default, reads a byte
  `b` above `0x7F` as the private-use character `U+F700 + b`, so every
  short name lists as its own name and is found by it; `Cp437` maps them.

```rust,no_run
use hadris_fat::sync::FatFs;
use hadris_fat::{Cp437, MountOptions};
use hadris_fs::{HeapTable, SystemClock};
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let image = std::fs::read("disk.img")?;
let options = MountOptions::new()
    .with_table(HeapTable::new())
    .with_clock(SystemClock)
    .with_code_page(Cp437);
let fs = FatFs::open_with(MemDevice::new(image, BlockSize::new(512).unwrap()), options)?;
# Ok(())
# }
```

`MountOptions::with_read_only()` mounts without ever calling
`write_blocks`.

A failed `open`, `open_with` or `format` returns a `hadris_fs::MountError`, which
gives the device back through `into_device` or `into_parts`. `?` converts
it into `hadris_fs::Error`, `PathError` or `std::io::Error`, dropping the
device.

Writes go to the device at once, except the size and modification time of
a pinned file, which stay in the node table so every handle sees one size
until `publish_node`, `sync_node` or `sync` writes them; closing a `File`
handle calls `publish_node`, which does not flush the device, and
`File::sync_all` calls `sync_node`, which does. `sync` also writes the FAT32 FSInfo free count and flushes the
device. `remove` of an open node (an open `File`, or one marked with
`open_node`) fails with `ErrorKind::Busy`; a node that is only pinned is
removed and its id answers `ErrorKind::NotFound` until its last `forget`. A
device that refuses a write makes the volume read-only with nothing changed.
Writes are ordered so that an interrupted operation, or a dropped `async`
future, leaves a volume that `fsck` repairs: at worst lost clusters, a
chain longer than its file, or a renamed node under both names.

### Formatting with `FatFs`

`format` (the `write` feature, every mode, no allocator) lays out a volume
that fills the block device, using its block count and size, and returns it
mounted. Format a partition by passing a `hadris_storage` `Partition`.

```rust,no_run
use hadris_fat::sync::format;
use hadris_fat::{FatKind, FormatOptions, VolumeLabel};
use hadris_fs::SystemClock;
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let dev = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
let options = FormatOptions::new()
    .with_kind(FatKind::Fat32)
    .with_label(VolumeLabel::new("BOOT")?)
    .with_clock(SystemClock);
let fs = format(dev, options)?;
# let _ = fs;
# Ok(())
# }
```

Without `with_kind`, volumes below 16 MiB are FAT12, below 512 MiB FAT16,
and larger ones FAT32. The cluster size starts from Microsoft's defaults
for the size and doubles or halves until the cluster count suits the
variant; `with_cluster_size` fixes it. `with_sector_size`,
`with_volume_id`, `with_oem_name`, `with_reserved_sectors`,
`with_hidden_sectors`, `with_fat_count`, `with_root_entries` and
`with_media` set the other boot sector fields. The clock stamps the label
entry and derives the volume id, so the default `NoClock` produces the same
bytes on every run. A device too small for the variant gives
`ErrorKind::NoSpace`, one too large gives `ErrorKind::LimitExceeded`, and a
bad option gives `ErrorKind::InvalidInput` before anything is written.

### Checking with `FatFs`

`check` (every mode, no allocator) reads the whole volume without changing
it and reports what `fsck` would: boot sector and FSInfo problems, FAT
copies that differ, chains that are broken, cyclic, cross-linked, lost or
the wrong length for their file, bad names and dot entries, labels out of
place, and long-name runs that are orphaned or fail their checksum.
`check_with` passes each `Finding` to a callback and takes the bitmap it
marks clusters in; the tree is walked once per bitmap's worth of clusters,
so a smaller bitmap costs time, never accuracy. `Finding` and its variants
with fields are `#[non_exhaustive]`, so match them with `..`. Repair is not
implemented.

```rust,no_run
use hadris_fat::sync::{FatFs, check_with};
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let image = std::fs::read("disk.img")?;
let mut fs = FatFs::open(MemDevice::new(image, BlockSize::new(512).unwrap()))?;
let mut bitmap = [0u8; 4096];
let report = check_with(&mut fs, &mut bitmap, |finding| println!("{finding:?}"))?;
println!("{} findings, {} lost clusters", report.findings(), report.lost_clusters());
# Ok(())
# }
```

The device is read as it is, so call `sync` first on a volume you have
written to. A volume left by an interrupted `FatFs` operation shows only
what the crash-safety rules allow: lost clusters, chains longer than their
file, a renamed node under both names, orphaned long-name fragments, FAT
copies that lag the active one and a stale FSInfo free count.
`FatFs::label` reads the volume label from the root directory, and
`FatFs::cluster_chain` passes the clusters of a file or directory to a
callback, for tools that show layout or fragmentation.

### Sharing a Volume Between Threads

`hadris_fs::sync::Volume` puts a `FatFs` behind a lock, so its path helpers
work on `&self` and an `Arc` shares it between threads. The runnable
`shared_volume` example mounts an image with a `SystemClock` and writes from a
worker thread:

```console
cargo run -p hadris-fat --example shared_volume -- disk.img
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `write` | `format` in each mode; `FatFs` and `ExFatFs` write without it | None |
| `alloc` | `HeapTable` and the other heap-backed `hadris-fs` conveniences | `alloc` crate |
| `sync` | Synchronous API in `sync` | `hadris-io/sync` |
| `async` | Asynchronous API in `r#async` | `hadris-io/async` |
| `async-send` | Asynchronous API with `Send` futures, in `async_send` | `async` |
| `std` | `hadris_storage::host::FileDevice` for image files and `SystemClock` | `std`, `alloc` |
| `defmt` | `defmt::Format` for `FatKind` and `Finding` | `defmt` |

Default features: `write`, `std`, `sync`

`std` selects platform integration but does not select an I/O mode. Custom
configurations should enable `sync`, `async`, or both explicitly. No feature
changes what an item does.

### exFAT

`hadris_fat::exfat::sync::ExFatFs` and its `r#async` and `async_send` twins
are a sibling of `FatFs` that needs no allocator and implements `FsDriver`,
with `format` (the `write` feature), `check` and `check_with` in each mode.
exFAT is stable and needs no feature flag. Its options, label, findings and
on-disk layouts are in `hadris_fat::exfat` (`exfat::FormatOptions`,
`exfat::MountOptions`, `exfat::raw`), since their names match FAT's. It reads contiguous and
chained allocations, fragmented bitmaps and up-case tables, and entry sets
that cross clusters; it writes FAT chains, grows directories, and keeps
`VolumeDirty` and `PercentInUse`. On TexFAT volumes it follows `ActiveFat`
and keeps both FATs and bitmaps equal. TexFAT transactions and repair are
not supported. The conformance suite in `tests/` qualifies it against
exfatprogs, macOS `newfs_exfat`/`fsck_exfat` and the macOS kernel driver.

### For Bootloaders and Embedded Systems

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["sync"] }
```

Add `write` for `format`, and `alloc` for `HeapTable`.

### For Desktop Applications (full features)

```toml
[dependencies]
hadris-fat = "2.4.0"  # Uses default features
```

## FAT Variant Support

| Variant | Max Volume Size | Max File Size | Cluster Size | Status |
|---------|----------------|---------------|--------------|--------|
| FAT12 | 32 MB | 32 MB | 512B - 8KB | Supported |
| FAT16 | 2 GB | 2 GB | 2KB - 32KB | Supported |
| FAT32 | 2 TB | 4 GB | 4KB - 32KB | Supported |
| exFAT | 128 PB | 128 PB | 512B - 32MB | Supported |

## Long Filename Support

`FatFs` always reads and writes VFAT long filenames:

- Filenames up to 255 UTF-16 code units
- Unicode character support (including supplementary-plane characters)
- Automatic short-name generation for 8.3 compatibility
- Directory-entry runs may span FAT cluster-chain boundaries

## No-std Compatibility

- `FatFs`, `format` and `check` need neither `std` nor `alloc` in any mode
- The only buffer is one device block of at most 4096 bytes
- All I/O goes through `hadris-storage` block devices
- Suitable for bootloaders, embedded systems, and custom kernels

## Specification Compliance

Implements the following specifications:

- Microsoft FAT specification
- VFAT (Long Filename) extension
- exFAT specification (TexFAT transactions not supported)

## Documentation

- [Read a FAT image](https://hxyulin.github.io/hadris/guides/read-fat-image)
- [Modify FAT safely](https://hxyulin.github.io/hadris/guides/modify-fat)
- [Create FAT filesystems](https://hxyulin.github.io/hadris/creation/fat)
- [API reference](https://docs.rs/hadris-fat)

## License

This project is licensed under the [MIT license](../../../LICENSE-MIT).
