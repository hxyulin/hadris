# Hadris FAT

A modern Rust FAT12, FAT16, and FAT32 filesystem library with read, write, and
format support. Hadris FAT handles VFAT long filenames and targets desktop disk
image tools as well as `no_std` bootloaders, kernels, firmware, embedded
systems, SD cards, and USB drives.

## Features

- **FAT12/16/32 Support** - Full read and write support for all FAT variants
- **Volume Formatting** - Create new FAT12/16/32 volumes with automatic type selection
- **Long Filenames (VFAT/LFN)** - Support for filenames beyond 8.3 format
- **No-std Compatible** - Use in bootloaders and custom kernels
- **FAT Caching** - Optional sector caching for improved performance
- **Analysis Tools** - Filesystem verification and diagnostic utilities
- **exFAT preview** - Opt-in unstable support for basic exFAT workflows

## Quick Start

### Reading a FAT Filesystem

```rust,no_run
use std::fs::File;
use hadris_fat::{FatVolume, FatVolumeReadExt};
use hadris_io::StdIo;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let file = File::open("disk.img")?;
let fs = FatVolume::open(StdIo::new(file))?;

let root = fs.root_dir();
let mut iter = root.entries();
while let Some(Ok(entry)) = iter.next_entry() {
    println!("{}", entry.name());
}

if let Some(entry) = root.find("README.TXT")? {
    let mut reader = fs.read_file(&entry)?;
    reader.seek(hadris_fat::SeekFrom::Start(128))?;
    let mut buffer = [0_u8; 64];
    let read = reader.read(&mut buffer)?;
    println!("{}", String::from_utf8_lossy(&buffer[..read]));
}
# Ok(())
# }
```

`FileReader::seek` accepts start-, current-, and end-relative positions. As
with `std::io::Seek`, positions past the end are valid and reads there return
zero bytes.

### The V3 `FatFs` Driver

`FatFs` is the node-based driver of the upcoming V3 API, available as
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
- `P`, the `CodePage` of short names. `Ascii`, the default, reads bytes
  above `0x7F` as U+FFFD; `Cp437` maps them.

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

`MountOptions::with_read_only(true)` mounts without ever calling
`write_blocks`.

Writes go to the device at once, except the size and modification time of
a pinned file, which stay in the node table so every handle sees one size
until `sync_node` or `sync` writes them; closing a `File` handle calls
`sync_node`. `sync` also writes the FAT32 FSInfo free count and flushes the
device. `remove` of a pinned node fails with `ErrorKind::Busy`, and a device
that refuses a write makes the volume read-only with nothing changed.
Writes are ordered so that an interrupted operation, or a dropped `async`
future, leaves a volume that `fsck` repairs: at worst lost clusters, a
chain longer than its file, or a renamed node under both names.

### Formatting with `FatFs`

`format` (the `write` feature, every mode, no allocator) lays out a volume
that fills the block device, using its block count and size, and returns it
mounted. Format a partition by passing a `hadris_storage` `Slice`.

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
so a smaller bitmap costs time, never accuracy. Repair is not implemented.

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
`FatFs::label` reads the volume label from the root directory.

### Writing to a FAT Filesystem

```rust,no_run
use std::fs::OpenOptions;
use hadris_fat::{FatVolume, FatVolumeWriteExt};
use hadris_io::StdIo;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let file = OpenOptions::new().read(true).write(true).open("disk.img")?;
let fs = FatVolume::open(StdIo::new(file))?;

let root = fs.root_dir();
let entry = fs.create_file(&root, "newfile.txt")?;
let mut writer = fs.write_file(&entry)?;
writer.write(b"Hello, FAT!")?;
writer.finish()?;
# Ok(())
# }
```

### Formatting a New FAT Volume

```rust,no_run
use hadris_fat::format::{FatFormatOptions, FatVolumeFormatter, FatTypeSelection};
use hadris_io::StdIo;
use std::io::Cursor;

# fn main() -> hadris_fat::Result<()> {
// Create a 64 MB in-memory volume
let mut buffer = vec![0u8; 64 * 1024 * 1024];
let cursor = StdIo::new(Cursor::new(&mut buffer[..]));

let options = FatFormatOptions::new(64 * 1024 * 1024)
    .volume_label("MYDISK");

let fs = FatVolumeFormatter::format(cursor, options)?;
println!("Created {} volume", fs.fat_type());

// Or force a specific FAT type
let options = FatFormatOptions::new(64 * 1024 * 1024)
    .fat_type(FatTypeSelection::Fat32)
    .volume_label("FAT32VOL");
# let _ = options;
# Ok(())
# }
```

### Sharing a Volume Between Threads

`FatVolume` is `Send` when its backing storage is `Send`. Put it behind a
mutex to share it safely between worker threads. Custom time providers and OEM
code-page converters must implement `Sync`.

```rust,no_run
use hadris_fat::FatVolume;
use hadris_io::StdIo;
use std::{fs::File, sync::{Arc, Mutex}, thread};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let volume = FatVolume::open(StdIo::new(File::options()
    .read(true)
    .write(true)
    .open("disk.img")?))?;
let volume = Arc::new(Mutex::new(volume));

let worker_volume = Arc::clone(&volume);
let fat_type = thread::spawn(move || worker_volume.lock().unwrap().fat_type())
    .join()
    .expect("volume worker panicked");

println!("mounted {fat_type}");
# Ok(())
# }
```

See the runnable `shared_volume` example for configuring a custom clock:

```console
cargo run -p hadris-fat --example shared_volume -- disk.img
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `read` | Read operations | None |
| `write` | `FatFs` formatting; with `alloc`, the V2 writer and formatter | `read` |
| `lfn` | Long filename (VFAT) support | None |
| `cache` | FAT sector caching for performance | `alloc`, `sync` |
| `tool` | Analysis and verification utilities | `alloc`, `read`, `sync` |
| `unstable-exfat` | Unstable, sync-only exFAT preview | `alloc`, `sync` |
| `alloc` | Heap allocation without full std | `alloc` crate |
| `sync` | Synchronous API | `hadris-io/sync` |
| `async` | Asynchronous API | `hadris-io/async` |
| `async-send` | Asynchronous API with `Send` futures, in `async_send` | `async` |
| `std` | Full standard library support | `std`, `alloc` |

Default features: `read`, `write`, `lfn`, `std`, `sync`

`std` selects platform integration but does not select an I/O mode. Custom
configurations should enable `sync`, `async`, or both explicitly. The `cache`,
`tool` and `unstable-exfat` capabilities remain sync-only and therefore imply
`sync`.

### exFAT preview status

The `unstable-exfat` feature is outside the Hadris V2 API stability promise.
It provides basic formatting, reading, traversal, and simple mutation on
conventional layouts, but is not recommended for irreplaceable data. The
preview does not support fragmented allocation bitmap or up-case metadata,
directory growth, general cross-cluster directory entry-set placement, async
operation, TexFAT, or repair workflows.

## Volume Formatting

The V2 `format` module (requires `write` and `alloc`) provides volume
formatting for `FatVolume`:

```rust,no_run
use hadris_fat::format::{FatFormatOptions, FatVolumeFormatter, SectorSize};

# fn main() -> hadris_fat::Result<()> {
# let volume_size: u64 = 64 * 1024 * 1024;
# let data = hadris_io::StdIo::new(std::io::Cursor::new(vec![0u8; volume_size as usize]));
let options = FatFormatOptions::new(volume_size)
    .volume_label("VOLUME")
    .sector_size(SectorSize::S512)
    .fat_copies(2);

let params = FatVolumeFormatter::calculate_params(&options)?;
println!("Will create {} with {} clusters", params.fat_type, params.cluster_count);

let fs = FatVolumeFormatter::format(data, options)?;
# let _ = fs;
# Ok(())
# }
```

Automatic FAT type selection follows Microsoft recommendations:

- < 16 MB: FAT12
- 16 MB - 512 MB: FAT16
- \> 512 MB: FAT32

### For Bootloaders (minimal footprint)

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["read", "sync"] }
```

### For Embedded Systems with Heap

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["read", "write", "alloc", "lfn", "sync"] }
```

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
| ExFAT | 128 PB | 128 PB | 4KB - 32MB | Experimental |

## Long Filename Support

When the `lfn` feature is enabled, the crate supports VFAT long filenames:

- Filenames up to 255 UTF-16 code units
- Unicode character support (including supplementary-plane characters)
- Automatic short-name generation for 8.3 compatibility
- Directory-entry runs may span FAT cluster-chain boundaries

## FAT Caching

The `cache` feature enables a write-back LRU cache for FAT-table sectors in the
synchronous API:

- Reduces redundant disk reads
- Configurable capacity via `FatVolume::builder(data).fat_cache(n).open()`
- Normal filesystem reads and writes use an installed cache transparently
- Dirty entries flush to all FAT copies on eviction or an explicit
  `FatVolume::flush()`

Enable the cache alongside the synchronous API and the capabilities your
application needs:

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["read", "write", "alloc", "lfn", "sync", "cache"] }
```

Install it while opening the volume:

```rust,no_run
use hadris_fat::FatVolume;
use hadris_io::StdIo;
use std::fs::OpenOptions;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let disk = OpenOptions::new()
    .read(true)
    .write(true)
    .open("disk.img")?;
let fs = FatVolume::builder(StdIo::new(disk))
    .fat_cache(16) // Capacity is measured in FAT sectors.
    .open()?;

// Existing FatVolume operations now route FAT-table access through the cache.
let root = fs.root_dir();
let mut entries = root.entries();
while let Some(entry) = entries.next_entry() {
    println!("{}", entry?.name());
}

// Required after cached writes to guarantee all dirty sectors reach every
// on-disk FAT copy before the volume is dropped.
fs.flush()?;
# Ok(())
# }
```

A capacity of zero disables caching. Read-only users do not need `flush`; a
writable cached volume should call it before teardown. The cache is sync-only:
async `FatVolume` operations continue to access the FAT directly.

## Analysis Tools

The `tool` feature adds extension traits on `FatVolume`:

```rust,no_run
use hadris_fat::{FatVolume, FatAnalysisExt, FatVerifyExt};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
# let fs = FatVolume::open(hadris_io::StdIo::new(std::fs::File::open("disk.img")?))?;
let stats = fs.statistics()?;
println!("Total clusters: {}", stats.total_clusters);
println!("Free clusters: {}", stats.free_clusters);
println!("Bad clusters: {}", stats.bad_clusters);

let report = fs.verify()?;
println!("Issues: {}", report.issues.len());
# Ok(())
# }
```

## No-std Compatibility

- Core reading requires `read` + `sync` (add `alloc` for high-level APIs that need heap)
- Write operations require `alloc`
- All I/O uses `hadris-io` traits instead of `std::io` directly
- Suitable for bootloaders, embedded systems, and custom kernels

## Specification Compliance

Implements the following specifications:

- Microsoft FAT specification
- VFAT (Long Filename) extension
- exFAT specification (partial, experimental)

## Documentation

- [Read a FAT image](https://hxyulin.github.io/hadris/guides/read-fat-image)
- [Modify FAT safely](https://hxyulin.github.io/hadris/guides/modify-fat)
- [Create FAT filesystems](https://hxyulin.github.io/hadris/creation/fat)
- [API reference](https://docs.rs/hadris-fat)

## License

This project is licensed under the [MIT license](../../../LICENSE-MIT).
