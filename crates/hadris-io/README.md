# Hadris I/O

No-std I/O abstraction layer for the Hadris filesystem crates.

## Overview

This crate provides `Read`, `Write`, and `Seek` traits that work in no-std environments, enabling filesystem implementations to run on bare-metal systems, bootloaders, and embedded devices.

## Features

- **No-std Compatible** - Works without the standard library
- **Familiar API** - Trait signatures mirror `std::io` where possible
- **Zero-copy** - Uses bytemuck for efficient struct reading/writing
- **Sector-based I/O** - `SectorCursor` wrapper for sector-aligned operations

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Standard library support | Yes |

## Usage

### With std (default)

```toml
[dependencies]
hadris-io = "0.2"
```

### No-std

```toml
[dependencies]
hadris-io = { version = "0.2", default-features = false }
```

## Core Traits

```rust
pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), IoError>;
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize, IoError>;
    fn write_all(&mut self, buf: &[u8]) -> Result<(), IoError>;
    fn flush(&mut self) -> Result<(), IoError>;
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError>;
}
```

## SectorCursor

The `SectorCursor` wrapper provides sector-aligned I/O operations:

```rust
use hadris_io::SectorCursor;

let cursor = SectorCursor::new(data, 512, 4096); // 512-byte sectors, 4KB clusters
cursor.seek_sector(Sector(100))?;
cursor.seek_cluster(Cluster(2))?;
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
