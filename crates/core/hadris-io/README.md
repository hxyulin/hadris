# Hadris I/O

No-std I/O abstraction layer for the Hadris filesystem crates.

## Overview

This crate provides the `Read`, `Write`, and `Seek` traits that every Hadris
format crate consumes. They work in no-std environments, so the same
filesystem code runs on desktops, bootloaders, kernels, and embedded devices.

The traits are Hadris's own. They have no associated error type: every method
returns `hadris_io::Result<T>` with the single, non-generic
[`Error`](https://docs.rs/hadris-io) type. Devices from other I/O ecosystems
are connected through explicit adapters rather than blanket implementations.

## Features

- **No-std Compatible** - Works without the standard library or an allocator
- **Sync and async** - Both trait sets share one definition and never drift
- **Explicit adapters** - `StdIo`, `ToStd`, and `FromEmbedded` at the edges
- **Zero-copy helpers** - [`ReadExt`](https://docs.rs/hadris-io) structured reads via bytemuck
- **In-memory [`Cursor`](https://docs.rs/hadris-io)** - Byte-slice reader/seeker for parsing
- **Positional sources** - `ByteSource` for writer inputs that are read more than once

> Sector-aligned wrappers such as `SectorCursor` live in **`hadris-fat`**, not in this crate.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | `StdIo`, `ToStd`, and `std::io::Error` conversions; implies `alloc` | Yes |
| `sync` | Synchronous I/O traits | Yes |
| `async` | Asynchronous I/O traits in `hadris_io::r#async` | No |
| `alloc` | Keeps the device error as the `Error` source; `Box<T>` implements the traits | via `std` |

`std` and the I/O mode are independent. Defaults enable both `std` and `sync`,
while custom configurations may select `sync`, `async`, or both. Enabling a
feature only adds items; no trait or type changes shape.

## Usage

### With std (default)

```toml
[dependencies]
hadris-io = "2.4.0"
```

### No-std

```toml
[dependencies]
hadris-io = { version = "2.4.0", default-features = false, features = ["sync"] }
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

Implementors write only `read`, `write` and `flush`, and `seek`. Everything
else has a default:

```rust,ignore
pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> { ... }
}

pub trait Write {
    fn write(&mut self, buf: &[u8]) -> Result<usize>;
    fn flush(&mut self) -> Result<()>;
    fn write_all(&mut self, buf: &[u8]) -> Result<()> { ... }
}

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
    fn stream_position(&mut self) -> Result<u64> { ... }
    fn seek_relative(&mut self, offset: i64) -> Result<()> { ... }
    fn rewind(&mut self) -> Result<()> { ... }
}
```

The async traits in `hadris_io::r#async` have the same shape with `async fn`.

`&mut T` implements each trait when `T` does, and with `alloc` so does
`Box<T>`. Generic code holding `R: Read` can therefore pass `&mut R` to any API
that takes a reader and keep using it afterwards.

A custom device implements the traits directly:

```rust
use hadris_io::{Error, ErrorKind, Read, Result};

struct Zeroes {
    online: bool,
}

impl Read for Zeroes {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if !self.online {
            return Err(Error::new(ErrorKind::NotConnected, "device offline"));
        }
        buf.fill(0);
        Ok(buf.len())
    }
}

let mut device = Zeroes { online: true };
let mut buf = [0xff_u8; 4];
device.read_exact(&mut buf).unwrap();
assert_eq!(buf, [0; 4]);
```

## Adapters

| Adapter | Feature | Purpose |
|---------|---------|---------|
| `StdIo<T>` | `std` | Use a `std::io` reader, writer, or seeker as a Hadris device |
| `ToStd<T>` | `std` + `sync` | Expose a Hadris reader, writer, or seeker as `std::io` |
| `FromEmbedded<T>` | always | Use an `embedded-io` or `embedded-io-async` device |

```rust
use hadris_io::{Cursor, Read, StdIo, ToStd};

let mut file = StdIo::new(std::io::Cursor::new(b"abc".to_vec()));
let mut buf = [0u8; 3];
file.read_exact(&mut buf).unwrap();
assert_eq!(&buf, b"abc");

let mut reader = ToStd::new(Cursor::new(b"xyz"));
let mut text = String::new();
std::io::Read::read_to_string(&mut reader, &mut text).unwrap();
assert_eq!(text, "xyz");
```

`FromEmbedded::new(device)` works the same way for any type implementing the
`embedded-io` traits, and for `embedded-io-async` types when `async` is
enabled.

## Errors

`Error` holds a portable `ErrorKind`, an optional static message, and with
`alloc` the original device error as its source. Its public shape is the same
with and without `alloc`; without it, only the kind and message survive.

| Constructor | Use |
|-------------|-----|
| `Error::from_kind(kind)` | A bare kind, also available as `From<ErrorKind>` |
| `Error::new(kind, msg)` | A kind with static context |
| `Error::other(msg)` | `ErrorKind::Other` with static context |
| `Error::from_io(err)` | Convert an `embedded-io` error, keeping it as the source |
| `Error::with_source(kind, err)` | Wrap any `core::error::Error` as the source |

`downcast_source::<E>()` recovers a typed device error. With `std`,
`std::io::Error` converts to and from `Error` in both directions.

```rust
use hadris_io::{Error, ErrorKind};

let error = Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"));
assert_eq!(error.kind(), ErrorKind::NotFound);
assert!(error.downcast_source::<std::io::Error>().is_some());
```

## Byte Sources

`ByteSource` is a positional source of bytes with a known length. Writers use
it for file contents so they can read the same bytes more than once without a
seek contract. `&[u8]`, `Vec<u8>`, and `&mut S` implement it, and
`SeekSource<T>` adapts any `Read + Seek`:

```rust
use hadris_io::{ByteSource, Cursor, SeekSource};

let data = [1u8, 2, 3, 4, 5, 6];
let mut source = SeekSource::new(Cursor::new(&data)).unwrap();
assert_eq!(source.len(), 6);

let mut buf = [0u8; 2];
source.read_exact_at(4, &mut buf).unwrap();
assert_eq!(buf, [5, 6]);
```

## Documentation

- [Storage and I/O model](https://hxyulin.github.io/hadris/concepts/storage-model)
- [Adapt a custom device](https://hxyulin.github.io/hadris/guides/custom-io)
- [Use asynchronous I/O](https://hxyulin.github.io/hadris/guides/async-io)
- [API reference](https://docs.rs/hadris-io)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
