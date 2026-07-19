# Hadris I/O

No-std I/O abstraction layer for the Hadris filesystem crates.

## Overview

This crate provides `Read`, `Write`, and `Seek` traits that work in no-std environments, enabling filesystem implementations to run on bare-metal systems, bootloaders, and embedded devices.

When the `std` feature is enabled, traits re-export from `std::io`. In `no_std` mode, a minimal custom trait surface is provided. The no-std [`Error`](https://docs.rs/hadris-io) type is intentionally smaller than `std::io::Error` (no OS error codes; message storage requires `alloc`).

## Features

- **No-std Compatible** - Works without the standard library
- **Familiar API** - Trait signatures mirror `std::io` where practical
- **Zero-copy helpers** - [`ReadExt`](https://docs.rs/hadris-io) structured reads via bytemuck
- **In-memory [`Cursor`](https://docs.rs/hadris-io)** - Byte-slice reader/seeker for parsing

> Sector-aligned wrappers such as `SectorCursor` live in **`hadris-fat`**, not in this crate.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Standard library support (implies `sync`, `alloc`) | Yes |
| `sync` | Synchronous I/O traits | Yes |
| `async` | Asynchronous I/O traits | No |
| `alloc` | Heap allocation (dynamic error messages in no-std) | via `std` |

## Usage

### With std (default)

```toml
[dependencies]
hadris-io = "2.0.0-rc.2"
```

### No-std

```toml
[dependencies]
hadris-io = { version = "2.0.0-rc.2", default-features = false, features = ["sync"] }
```

## Quick Start

```rust
use hadris_io::{Cursor, SeekFrom, Read, Seek};

let data = [0x48, 0x44, 0x52, 0x53]; // "HDRS"
let mut cursor = Cursor::new(&data);

let mut buf = [0u8; 2];
cursor.read_exact(&mut buf).unwrap();
assert_eq!(&buf, b"HD");

cursor.seek(SeekFrom::Start(0)).unwrap();
cursor.read_exact(&mut buf).unwrap();
assert_eq!(&buf, b"HD");
```

## Core Traits

With `std`, these are `std::io::{Read, Write, Seek}`. Without `std`, this crate defines compatible traits returning [`hadris_io::Result`](https://docs.rs/hadris-io):

```rust,ignore
pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()>;
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn write_all(&mut self, buf: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
}
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
