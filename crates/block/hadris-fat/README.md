# Hadris FAT

A comprehensive Rust implementation of the FAT filesystem family with support for FAT12, FAT16, FAT32, long filenames (VFAT), and no-std environments.

## Features

- **FAT12/16/32 Support** - Full read and write support for all FAT variants
- **Volume Formatting** - Create new FAT12/16/32 volumes with automatic type selection
- **Long Filenames (VFAT/LFN)** - Support for filenames beyond 8.3 format
- **No-std Compatible** - Use in bootloaders and custom kernels
- **FAT Caching** - Optional sector caching for improved performance
- **Analysis Tools** - Filesystem verification and diagnostic utilities
- **ExFAT** - Experimental support for exFAT (work in progress)

## Quick Start

### Reading a FAT Filesystem

```rust,no_run
use std::fs::File;
use hadris_fat::{FatFs, FatFsReadExt};

# fn main() -> hadris_fat::Result<()> {
let file = File::open("disk.img")?;
let fs = FatFs::open(file)?;

let root = fs.root_dir();
let mut iter = root.entries();
while let Some(Ok(entry)) = iter.next_entry() {
    println!("{}", entry.name());
}
# Ok(())
# }
```

### Writing to a FAT Filesystem

```rust,no_run
use std::fs::OpenOptions;
use hadris_fat::{FatFs, FatFsWriteExt};

# fn main() -> hadris_fat::Result<()> {
let file = OpenOptions::new().read(true).write(true).open("disk.img")?;
let fs = FatFs::open(file)?;

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
use hadris_fat::format::{FatVolumeFormatter, FormatOptions, FatTypeSelection};
use std::io::Cursor;

# fn main() -> hadris_fat::Result<()> {
// Create a 64 MB in-memory volume
let mut buffer = vec![0u8; 64 * 1024 * 1024];
let cursor = Cursor::new(&mut buffer[..]);

let options = FormatOptions::new(64 * 1024 * 1024)
    .with_label("MYDISK");

let fs = FatVolumeFormatter::format(cursor, options)?;
println!("Created {} volume", fs.fat_type());

// Or force a specific FAT type
let options = FormatOptions::new(64 * 1024 * 1024)
    .with_fat_type(FatTypeSelection::Fat32)
    .with_label("FAT32VOL");
# let _ = options;
# Ok(())
# }
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `read` | Read operations | None |
| `write` | Write operations | `alloc`, `read` |
| `lfn` | Long filename (VFAT) support | None |
| `cache` | FAT sector caching for performance | `alloc`, `sync` |
| `tool` | Analysis and verification utilities | `alloc`, `read`, `sync` |
| `exfat` | Sync-only exFAT filesystem support (WIP) | `alloc`, `sync` |
| `alloc` | Heap allocation without full std | `alloc` crate |
| `sync` | Synchronous API | `hadris-io/sync` |
| `async` | Asynchronous API | `hadris-io/async` |
| `std` | Full standard library support | `std`, `alloc` |

Default features: `read`, `write`, `lfn`, `std`, `sync`

`std` selects platform integration but does not select an I/O mode. Custom
configurations should enable `sync`, `async`, or both explicitly. The `cache`,
`tool`, and `exfat` capabilities remain sync-only and therefore imply `sync`.

## Volume Formatting

The `format` module (requires `write`) provides volume formatting:

```rust,no_run
use hadris_fat::format::{FatVolumeFormatter, FormatOptions, SectorSize};

# fn main() -> hadris_fat::Result<()> {
# let volume_size = 64 * 1024 * 1024usize;
# let data = std::io::Cursor::new(vec![0u8; volume_size]);
let options = FormatOptions::new(volume_size)
    .with_label("VOLUME")
    .with_sector_size(SectorSize::S512)
    .with_fat_copies(2);

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
hadris-fat = { version = "1.2.1", default-features = false, features = ["read", "sync"] }
```

### For Embedded Systems with Heap

```toml
[dependencies]
hadris-fat = { version = "1.2.1", default-features = false, features = ["read", "write", "alloc", "lfn", "sync"] }
```

### For Desktop Applications (full features)

```toml
[dependencies]
hadris-fat = "1.2.1"  # Uses default features
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
- **Limitation:** LFN directory-entry runs that would span a cluster boundary are rejected with `FatError::DirEntryRunTooLong`

## FAT Caching

The `cache` feature enables LRU FAT sector caching (sync API only; silently bypassed under async):

- Reduces redundant disk reads
- Configurable capacity via `FatFs::builder(data).with_fat_cache(n).open()`
- Dirty entries flush to all FAT copies on eviction

## Analysis Tools

The `tool` feature adds extension traits on `FatFs`:

```rust,no_run
use hadris_fat::{FatFs, FatAnalysisExt, FatVerifyExt};

# fn main() -> hadris_fat::Result<()> {
# let fs = FatFs::open(std::fs::File::open("disk.img")?)?;
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

## License

This project is licensed under the [MIT license](../../LICENSE-MIT).
