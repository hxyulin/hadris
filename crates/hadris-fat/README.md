# Hadris FAT

A comprehensive Rust implementation of the FAT filesystem family with support for FAT12, FAT16, FAT32, long filenames (VFAT), and no-std environments.

## Features

- **FAT12/16/32 Support** - Full read and write support for all FAT variants
- **Long Filenames (VFAT/LFN)** - Support for filenames beyond 8.3 format
- **No-std Compatible** - Use in bootloaders and custom kernels
- **FAT Caching** - Optional sector caching for improved performance
- **Analysis Tools** - Filesystem verification and diagnostic utilities
- **ExFAT** - Experimental support for exFAT (work in progress)

## Quick Start

### Reading a FAT Filesystem

```rust
use std::fs::File;
use std::io::BufReader;
use hadris_fat::Fat;

let file = File::open("disk.img")?;
let reader = BufReader::new(file);
let fat = Fat::open(reader)?;

// Read root directory
let root = fat.root_dir();
for entry in root.iter() {
    let entry = entry?;
    println!("File: {}", entry.name());
}
```

### Writing to a FAT Filesystem

```rust
use hadris_fat::Fat;

let mut fat = Fat::open(file)?;

// Create a new file
let root = fat.root_dir_mut();
let mut file = root.create_file("newfile.txt")?;
file.write_all(b"Hello, FAT!")?;
```

### Creating a New FAT Volume

```rust
use hadris_fat::{Fat, FatType};
use std::io::Cursor;

let mut buffer = vec![0u8; 32 * 1024 * 1024]; // 32 MB
let cursor = Cursor::new(&mut buffer);

Fat::format(cursor, FatType::Fat32, "MYDISK")?;
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `read` | Read operations | None |
| `write` | Write operations | `alloc`, `read` |
| `lfn` | Long filename (VFAT) support | None |
| `cache` | FAT sector caching for performance | `alloc` |
| `tool` | Analysis and verification utilities | `alloc`, `read` |
| `exfat` | ExFAT filesystem support (WIP) | `alloc` |
| `alloc` | Heap allocation without full std | `alloc` crate |
| `std` | Full standard library support | `std`, `alloc` |

Default features: `read`, `write`, `lfn`, `std`

### For Bootloaders (minimal footprint)

```toml
[dependencies]
hadris-fat = { version = "0.2", default-features = false, features = ["read"] }
```

### For Embedded Systems with Heap

```toml
[dependencies]
hadris-fat = { version = "0.2", default-features = false, features = ["read", "write", "alloc", "lfn"] }
```

### For Desktop Applications (full features)

```toml
[dependencies]
hadris-fat = { version = "0.2" }  # Uses default features
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

- Filenames up to 255 characters
- Unicode character support
- Automatic fallback to 8.3 short names
- Maintains compatibility with non-LFN implementations

## FAT Caching

The `cache` feature enables intelligent FAT sector caching:

- Reduces redundant disk reads
- Configurable cache size
- Write-through or write-back strategies
- Significant performance improvement for fragmented files

## Analysis Tools

The `tool` feature provides filesystem analysis capabilities:

- Filesystem integrity verification
- Cluster chain validation
- Lost cluster detection
- Fragmentation analysis
- Disk space statistics

Example:

```rust
use hadris_fat::tool::FatAnalyzer;

let analyzer = FatAnalyzer::new(&fat);
let report = analyzer.analyze()?;
println!("Total clusters: {}", report.total_clusters);
println!("Free clusters: {}", report.free_clusters);
println!("Bad clusters: {}", report.bad_clusters);
```

## No-std Compatibility

The crate is designed for no-std environments:

- Core reading functionality requires only `read` feature (no allocations)
- Write operations require `alloc` feature for buffering
- All I/O uses `hadris-io` traits instead of `std::io`
- Suitable for bootloaders, embedded systems, and custom kernels

## Specification Compliance

Implements the following specifications:

- Microsoft FAT specification
- VFAT (Long Filename) extension
- exFAT specification (partial, experimental)

## License

This project is licensed under the [MIT license](LICENSE-MIT).
