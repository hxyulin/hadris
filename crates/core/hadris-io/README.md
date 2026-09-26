# Hadris I/O

No-std I/O abstraction layer for the Hadris filesystem crates.

## Overview

This crate provides the `Read`, `Write` and `Seek` traits that Hadris format
crates consume. They work without the standard library or an allocator, so
the same filesystem code runs on desktops, bootloaders, kernels and embedded
devices.

Each trait reports the implementor's own error through the `ErrorType`
supertrait, as `embedded-io` does. The only requirement on the error is
`core::error::Error + Send + Sync + 'static`: a kernel uses its own enum,
`StdIo` reports `std::io::Error`, and `FromEmbedded` (with the `embedded-io`
feature) passes an `embedded-io` error through unchanged.

## Features

- **No-std compatible** - no standard library or allocator needed
- **Sync and async** - both trait sets come from one source and never drift
- **Typed errors** - the device's error reaches the caller unchanged, with no allocation
- **Explicit adapters** - `StdIo`, `ToStd` and `FromEmbedded` at the edges
- **In-memory `Cursor`** - byte-slice reader and seeker for parsing
- **Positional sources** - `ByteSource` for writer inputs that are read more than once

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | `StdIo`, `ToStd` and conversions to `std::io::Error`; implies `alloc` | Yes |
| `sync` | Synchronous traits in `hadris_io::sync` | Yes |
| `async` | Asynchronous traits with `Send` futures in `hadris_io::r#async`, and in `hadris_io::local` for executors whose futures are not `Send` | No |
| `alloc` | `Box<T>` and `Vec<u8>` implement the traits | via `std` |
| `embedded-io` | `FromEmbedded`, the `embedded-io` traits on `StdIo`, and `SeekFrom` conversions | No |

Enabling a feature only adds items; no trait or type changes shape. The
traits are always named through their mode module (`hadris_io::sync::Read`);
the crate root holds only mode-independent items.

## Usage

### With std (default)

```toml
[dependencies]
hadris-io = "3.0.0-rc.1"
```

### No-std

```toml
[dependencies]
hadris-io = { version = "3.0.0-rc.1", default-features = false, features = ["sync"] }
```

## Quick Start

```rust
use hadris_io::sync::{Read, Seek};
use hadris_io::{Cursor, SeekFrom};

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
pub trait ErrorType {
    type Error: core::error::Error + Send + Sync + 'static;
}

pub trait Read: ErrorType {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ExactError<Self::Error>> { ... }
}

pub trait Write: ErrorType {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;
    fn flush(&mut self) -> Result<(), Self::Error>;
    fn write_all(&mut self, buf: &[u8]) -> Result<(), ExactError<Self::Error>> { ... }
}

pub trait Seek: ErrorType {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error>;
    fn stream_position(&mut self) -> Result<u64, Self::Error> { ... }
    fn rewind(&mut self) -> Result<(), Self::Error> { ... }
}

pub enum ExactError<E> { UnexpectedEof, WriteZero, Io(E) }

#[non_exhaustive]
pub enum SeekFrom { Start(u64), End(i64), Current(i64) }
```

`SeekFrom` is Hadris's own, so no dependency is needed to seek. It converts
to and from `std::io::SeekFrom` with `std` and `embedded_io::SeekFrom` with
`embedded-io`, and an implementation of `Seek` resolves it with
`SeekFrom::resolve(current, len)` rather than matching it.

The async traits in `hadris_io::r#async` have the same shape with `async fn`.

`&mut T` implements each trait when `T` does, and with `alloc` so does
`Box<T>`. Generic code holding `R: Read` can therefore pass `&mut R` to any API
that takes a reader and keep using it afterwards.

A custom device implements the traits directly, with its own error:

```rust
use hadris_io::sync::Read;
use hadris_io::{ErrorType, ExactError};

#[derive(Debug, PartialEq)]
struct Offline;

impl core::fmt::Display for Offline {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("device offline")
    }
}

impl core::error::Error for Offline {}

struct Zeroes {
    online: bool,
}

impl ErrorType for Zeroes {
    type Error = Offline;
}

impl Read for Zeroes {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Offline> {
        if !self.online {
            return Err(Offline);
        }
        buf.fill(0);
        Ok(buf.len())
    }
}

let mut buf = [0xff_u8; 4];
Zeroes { online: true }.read_exact(&mut buf).unwrap();
assert_eq!(buf, [0; 4]);
assert_eq!(
    Zeroes { online: false }.read_exact(&mut buf),
    Err(ExactError::Io(Offline))
);
```

## Adapters

| Adapter | Feature | Error | Purpose |
|---------|---------|-------|---------|
| `StdIo<T>` | `std` | `std::io::Error` | Use a `std::io` reader, writer or seeker |
| `ToStd<T>` | `std` + `sync` | | Expose a Hadris reader, writer or seeker as `std::io` |
| `FromEmbedded<T>` | `embedded-io` | `T::Error` | Use an `embedded-io` or `embedded-io-async` device |

```rust
use hadris_io::sync::Read;
use hadris_io::{Cursor, StdIo, ToStd};

let mut file = StdIo::new(std::io::Cursor::new(b"abc".to_vec()));
let mut buf = [0u8; 3];
file.read_exact(&mut buf).unwrap();
assert_eq!(&buf, b"abc");

let mut reader = ToStd::new(Cursor::new(b"xyz"));
let mut text = String::new();
std::io::Read::read_to_string(&mut reader, &mut text).unwrap();
assert_eq!(text, "xyz");
```

## Errors

With `std`, every error converts to `std::io::Error` with `?`.
`into_std_error` returns an `std::io::Error` device error as itself, found by
a downcast that does not allocate, so `raw_os_error()` survives. Any other
device error becomes the source of an `Other` error and can be downcast back.

```rust
use hadris_io::sync::Read;
use hadris_io::{ExactError, StdIo};

fn read_header(file: &mut StdIo<std::fs::File>) -> std::io::Result<[u8; 4]> {
    let mut header = [0u8; 4];
    file.read_exact(&mut header)?;
    Ok(header)
}

let err: std::io::Error = ExactError::Io(std::io::Error::from_raw_os_error(5)).into();
assert_eq!(err.raw_os_error(), Some(5));
```

`Cursor` never fails to read and reports `InvalidSeek` for a seek to a
negative position.

### The Hadris error

`Error<E>` is the error of every block device and filesystem operation in
Hadris. It lives here so that block devices can return it, and `hadris-fs`
and `hadris` re-export it. Its context is `Copy` and needs no allocation:

- `kind()`: an `ErrorKind` such as `NotFound`, `ReadOnly`, `Corrupt` or
  `NotRecognized` (the bytes are not this format at all).
- `message()`: a static description.
- `location()`: an optional `Location` (byte, block, cluster, or a byte of
  a name).
- `detail()`: an optional `DetailCode`, a number within a static domain
  that a format crate's `Detail::of(&err)` reads back.
- `device_error()`: the device's own error `E` when the device failed
  (kind `Io`).

`Display` shows the message and location; the device error is the
`source()`, never repeated in the message. `ErrorKind::errno()` gives one
symbolic `Errno` per kind, with `Errno::linux()` for the number;
`Unsupported` is `EOPNOTSUPP`.

```rust
use hadris_io::{Error, ErrorKind, Location};

let err = Error::device(std::io::Error::from_raw_os_error(5), "reading a block failed")
    .with_location(Location::Block(12));
assert_eq!(err.kind(), ErrorKind::Io);
assert_eq!(err.to_string(), "reading a block failed at block 12");
assert_eq!(ErrorKind::ReadOnly.errno().linux(), 30);

let io: std::io::Error = err.into();
assert_eq!(io.raw_os_error(), Some(5));
```

## Byte Sources

`ByteSource` is a positional source of bytes with a known length. Writers use
it for file contents so they can read the same bytes more than once without a
seek contract. `&[u8]`, `Vec<u8>` and `&mut S` implement it, and
`SeekSource<T>` adapts any `Read + Seek`:

```rust
use hadris_io::sync::{ByteSource, SeekSource};
use hadris_io::Cursor;

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
