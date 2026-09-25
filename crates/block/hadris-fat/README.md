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
- **No allocator needed** - The embedded API reads and writes, and `format` and `check` run, without `alloc`
- **Sync and async** - One driver generated for each mode; async futures are `Send`
- **Checker** - A read-only `fsck` that reports each problem it finds
- **exFAT** - `ExFatFs` reads, writes, formats and checks exFAT, including TexFAT volumes with two FATs

## Quick Start

### The `FatFs` Driver

`FatFs` is the node-based driver, available as
`sync::FatFs` and `r#async::FatFs`. It mounts any
`hadris-storage` block device, needs `alloc` for its node table (its only
I/O buffer is one device block of at most 4096 bytes), and implements the `hadris-fs`
`FileSystem` trait, so `Volume` and its `File` and `ReadDir` handles work
on it. Long names are always read and written: a name
that fits 8.3 in one case per part is stored as a short entry alone, and
other names get long-name entries and a short name with a `~N` tail.

```rust,no_run
use hadris_fat::sync::FatFs;
use hadris_fs::sync::{FileSystem, Volume};
use hadris_fs::{MountOptions, Name, OpenOptions};
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let image = std::fs::read("disk.img")?;
let dev = MemDevice::new(image, BlockSize::new(512).unwrap());
let mut fs = FatFs::mount(dev, MountOptions::new())?;

// Node ids, no locks. `lookup` pins, `forget` unpins.
let root = fs.root();
let efi = fs.lookup(root, Name::new("efi"))?;
fs.forget(efi, 1);

// Paths and handles, shared behind a lock.
let vol = Volume::new(fs);
for entry in vol.read_dir("/EFI")? {
    println!("{:?}", entry?.name());
}
vol.create_dir_all("/logs")?;
let mut log = vol.open("/logs/boot.txt", OpenOptions::new().write().create())?;
log.write(b"booted")?;
log.close()?;
vol.lock().sync()?;
# Ok(())
# }
```

`FatFs<D>` takes its settings at mount time from `hadris_fs::MountOptions`:

- `read_only()` mounts without ever calling `write_blocks`.
- `with_clock` sets the `hadris_fs::Clock` for new and modified entries.
  `NoClock`, the default, writes 1980-01-01 so images are reproducible;
  `SystemClock` (`std`) writes the current time.
- `with_utc_offset` names the zone of FAT's zoneless timestamps; without
  it they are read and written as UTC.
- `with_code_page` sets the `hadris_fs::CodePage` of short names. `Cp437`
  is the default; `Ascii` reads a byte `b` above `0x7F` as the private-use
  character `U+F700 + b`, so every short name lists as its own name and is
  found by it.
- `with_node_limit` caps the pinned and open nodes; past it `lookup`,
  `create` and `mkdir` fail with `ErrorKind::LimitExceeded`. The table is
  unbounded otherwise.

```rust,no_run
use hadris_fat::sync::FatFs;
use hadris_fs::{Ascii, MountOptions, SystemClock};
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let image = std::fs::read("disk.img")?;
let options = MountOptions::new()
    .with_clock(&SystemClock)
    .with_utc_offset(60)?
    .with_code_page(&Ascii);
let fs = FatFs::mount(MemDevice::new(image, BlockSize::new(512).unwrap()), options)?;
let dev = fs.unmount()?;
# let _ = dev;
# Ok(())
# }
```

`unmount` syncs and gives the device back; `into_inner` gives it back
without syncing.

A failed `mount` or `unmount` returns a `hadris_fs::MountError`, which
gives the device back through `into_device` or `into_parts`. `?` converts
it into `hadris_fs::Error`, `PathError` or `std::io::Error`, dropping the
device.

Writes go to the device at once, except the size and modification time of
a pinned file, which stay in the node table so every handle sees one size
until `close`, `fsync` or `sync` writes them; closing a `File` handle
calls `close`, which does not flush the device, and `File::sync_all` calls
`fsync`, which does. `sync` also writes the FAT32 FSInfo free count and
flushes the device. `unlink` of an open file (an open `File`, or one
opened with `open`) fails with `ErrorKind::Busy`; a node that is only pinned is
removed and its id answers `ErrorKind::NotFound` until its last `forget`. A
device that refuses a write makes the volume read-only with nothing changed.
Writes are ordered so that an interrupted operation, or a dropped `async`
future, leaves a volume that `fsck` repairs: at worst lost clusters, a
chain longer than its file, or a renamed node under both names.

### Formatting with `FatFs`

`format(&mut dev, &options)` (the `write` feature, every mode, no
allocator) lays out a volume and returns its `Geometry`; mount it with
`FatFs::mount` and the `MountOptions` of your choice. The volume fills the
device unless `with_size` asks for another size, and a growable device such
as `Vec<u8>` grows to it. Format a partition by passing a `hadris_storage`
`Partition`; its start becomes the boot sector's hidden sectors unless
`with_partition_offset` says otherwise.

```rust,no_run
use hadris_fat::sync::{FatFs, format};
use hadris_fat::{FatKind, FatOptions, VolumeLabel};
use hadris_fs::MountOptions;
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut dev = MemDevice::new(vec![0u8; 64 << 20], BlockSize::new(512).unwrap());
let options = FatOptions::new()
    .with_kind(FatKind::Fat32)
    .with_label(VolumeLabel::new("BOOT")?);
let geometry = format(&mut dev, &options)?;
let fs = FatFs::mount(dev, MountOptions::new())?;
# let _ = (geometry, fs);
# Ok(())
# }
```

Without `with_kind`, volumes below 16 MiB are FAT12, below 512 MiB FAT16,
and larger ones FAT32. The cluster size starts from Microsoft's defaults
for the size and doubles or halves until the cluster count suits the
variant; `with_cluster_size` fixes it. `with_sector_size`, `with_serial`,
`with_oem_name`, `with_reserved_sectors`, `with_fat_count`,
`with_root_entries` and `with_media` set the other boot sector fields, and
`with_alignment` starts the data region on a multiple of its size. The time
(`with_time`, `NoClock::TIME` by default) stamps the label entry, and the
serial derives from `with_seed` or the time, so the same options produce the
same bytes on every run. A device too small for the size or variant gives
`ErrorKind::NoSpace`, one too large gives `ErrorKind::LimitExceeded`, and a
bad option gives `ErrorKind::InvalidInput` before anything is written.

With `alloc`, `write(dev, &tree, &options)` formats and copies a
`hadris_fs::Tree` into the volume with `copy_tree`, giving nodes without
times the options' time, and returns a `hadris_fs::Report` with the volume
size and what FAT could not store.

### Checking

`check(&mut dev, scratch, on_finding)` (every mode, no allocator) reads an
unmounted volume without changing it and reports what `fsck` would: boot
sector and FSInfo problems, FAT copies that differ, a dirty volume, chains
that are broken, cyclic, cross-linked, lost or the wrong length for their
file, bad names and dot entries, labels out of place, and long-name runs
that are orphaned or fail their checksum. Each `hadris_fs::Finding` has a
message, a `hadris_fat::Detail` code (the one mount errors use), a
severity, a location and the path of its entry. A damaged FAT32 boot
sector is a finding, and the check goes on from the backup.

The first 1 KiB of `scratch` holds the path of each finding and the rest
a bitmap of one bit per cluster; the tree is walked once per bitmap's worth
of clusters, so a smaller buffer costs time, never accuracy. It must be at
least 1536 bytes. Repair is not implemented.

```rust,no_run
use hadris_fat::sync::check;
use hadris_storage::{BlockSize, MemDevice};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let image = std::fs::read("disk.img")?;
let mut dev = MemDevice::new(image, BlockSize::new(512).unwrap());
let mut scratch = [0u8; 4096];
let report = check(&mut dev, &mut scratch, |finding| println!("{finding}"))?;
println!("{} findings in {} passes", report.findings(), report.passes());
# Ok(())
# }
```

Unmount a volume you have written to, with `sync` and `into_inner`, before
checking its device. A volume left by an interrupted `FatFs` operation shows only
what the crash-safety rules allow: lost clusters, chains longer than their
file, a renamed node under both names, orphaned long-name fragments, FAT
copies that lag the active one and a stale FSInfo free count.
`FatFs` and `ExFatFs` also have format extras as inherent methods: `info()`
returns the boot sector's `Geometry` (FAT variant, cluster size, serial),
`was_dirty()` whether the volume was cleanly unmounted, `extents(node,
from, &mut out)` maps a file or directory to device ranges, FIEMAP style,
for tools that show layout or fragmentation, `records(node, &mut out)`
locates its directory entries, and `read_raw(offset, buf)` reads the
device through the driver. `set_label(Some(label))` and
`set_volume_serial(serial)` change the label and serial in place.

### Sharing a Volume Between Threads

`hadris_fs::sync::Volume` puts a `FatFs` behind a lock, so its path methods
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
| `alloc` | `FatFs`, `ExFatFs` and the tree writers; without it the embedded API, `format`, `check` and the raw layer | `alloc` crate |
| `sync` | Synchronous API in `sync` | `hadris-io/sync` |
| `async` | Asynchronous API with `Send` futures in `r#async` | `hadris-io/async` |
| `std` | `hadris_storage::host::FileDevice` for image files and `SystemClock` | `std`, `alloc` |
| `defmt` | `defmt::Format` for `FatKind` | `defmt` |

Default features: `write`, `std`, `sync`

`std` selects platform integration but does not select an I/O mode. Custom
configurations should enable `sync`, `async`, or both explicitly. No feature
changes what an item does.

### exFAT

`hadris_fat::exfat::sync::ExFatFs` and its `r#async` twin
are a sibling of `FatFs` that needs `alloc` and implements `FileSystem`,
with `format` (the `write` feature) and `check` in each mode.
exFAT is stable and needs no feature flag. It mounts with the same `hadris_fs::MountOptions`. Its format options, label
and detail codes are in `hadris_fat::exfat` (`exfat::ExFatOptions`,
`exfat::Geometry`, `exfat::Detail`), since their names match FAT's. It reads contiguous and
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

Without `alloc` this gives the embedded API, `check` and the raw layer;
add `write` for `format`, and `alloc` for `FatFs` and `ExFatFs`.

`hadris_fat::embedded::sync::Fat<D, const FILES: usize = 4>` and its
`embedded::r#async` twin are a handle-based driver for firmware. They are
built on the raw layer and need no allocator: one 512-byte block buffer,
the geometry, the options and `FILES` file slots, under 1 KiB with four
slots. Directories are `Copy` handles, names are passed one component per
call, a `File` is a slot consumed by `close`, and `list` lends each entry
to a callback. Names fold ASCII case unless
`Options::new().with_fold(hadris_fat_raw::fold_unicode)` asks for
Unicode, so the Unicode case tables stay out of flash. The async variant
takes a `hadris_storage::local::BlockDevice`, whose futures need not be
`Send`. The device's blocks must be 512 bytes.

```rust,ignore
use hadris_fat::embedded::sync::Fat;
use hadris_fs::OpenOptions;

let mut fat: Fat<_> = Fat::mount(sd_card)?;
let logs = fat.create_dir_all(fat.root(), "data/logs")?;
let log = fat.open(logs, "boot.txt", OpenOptions::new().write().create().append())?;
fat.write(&log, b"booted\n")?;
fat.close(log)?;
let sd_card = fat.unmount()?;
```

Writes follow the same crash ordering as `FatFs`. A power cut, or a
dropped async future, leaves at worst lost clusters, and the next writing
call or `sync` on the same `Fat` frees them.

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

## The Raw Layer

The on-disk layouts and the I/O-free codecs the drivers use live in the
[`hadris-fat-raw`](../hadris-fat-raw) crate: boot
sector parsing into a `Geometry`, FAT entry encoding, directory slots,
long and short names, timestamps, the format layout planner and the exFAT
checksums and up-case decoder. Its `io` module holds the device
primitives `FatFs` is built on: FAT entry reads and writes on every copy,
chain walks, batched allocation and freeing, directory slots and `mkfs`,
generated for each mode; `exfat::io` holds the exFAT bitmap, up-case and
entry set primitives `ExFatFs` is built on. It is for tools and firmware that the drivers do
not fit. It has its own version, so `hadris-fat` does not re-export it:
only `FatKind`, `Detail`, `exfat::Detail` and the `check` functions, which
this crate's own API uses, are available here. Depend on `hadris-fat-raw`
directly for the rest.

## No-std Compatibility

- The embedded API, `format` and `check` need neither `std` nor `alloc` in
  any mode; `FatFs` and `ExFatFs` need `alloc`
- `FatFs` keeps one device block of at most 4096 bytes, the embedded API
  one of 512 bytes
- CI builds the no-allocator tiers for `thumbv6m-none-eabi`,
  `thumbv7em-none-eabihf` and `riscv32imc-unknown-none-elf`
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
