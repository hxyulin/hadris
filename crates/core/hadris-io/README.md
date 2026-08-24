# Hadris I/O

No-std I/O abstraction layer for the Hadris filesystem crates.

## Overview

This crate provides `Read`, `Write`, and `Seek` traits that work in no-std environments, enabling filesystem implementations to run on bare-metal systems, bootloaders, and embedded devices.

The traits are always this crate's own definitions; enabling the `std` feature adds blanket implementations for `std::io` readers, writers, and seekers so they can be used directly. The [`Error`](https://docs.rs/hadris-io) type is intentionally smaller than `std::io::Error` (no OS error codes; context messages are static strings).

## Features

- **No-std Compatible** - Works without the standard library
- **Familiar API** - Trait signatures mirror `std::io` where practical
- **Zero-copy helpers** - [`ReadExt`](https://docs.rs/hadris-io) structured reads via bytemuck
- **In-memory [`Cursor`](https://docs.rs/hadris-io)** - Byte-slice reader/seeker for parsing

> Sector-aligned wrappers such as `SectorCursor` live in **`hadris-fat`**, not in this crate.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Standard library support; implies `alloc` | Yes |
| `sync` | Synchronous I/O traits | Yes |
| `async` | Asynchronous I/O traits | No |
| `alloc` | Currently a no-op; reserved for future use | via `std` |

`std` and the I/O mode are independent. Defaults enable both `std` and `sync`,
while custom configurations may select `sync`, `async`, or both.

## Usage

### With std (default)

```toml
[dependencies]
hadris-io = "2.2.0"
```

### No-std

```toml
[dependencies]
hadris-io = { version = "2.2.0", default-features = false, features = ["sync"] }
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

The traits below are always this crate's own definitions, returning [`hadris_io::Result`](https://docs.rs/hadris-io). With `std`, blanket implementations cover any `std::io::{Read, Write, Seek}` type:

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

## Documentation

- [Storage and I/O model](https://hxyulin.github.io/hadris/concepts/storage-model)
- [Adapt a custom device](https://hxyulin.github.io/hadris/guides/custom-io)
- [Use asynchronous I/O](https://hxyulin.github.io/hadris/guides/async-io)
- [API reference](https://docs.rs/hadris-io)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
