---
title: Adapt a custom device
---

# Adapt a custom device or firmware reader

Format crates consume `hadris-io` traits rather than requiring `std::io`.
There are two ways to connect a device: implement the Hadris traits directly,
or wrap a device that already implements `embedded-io` in `FromEmbedded`.
Hosted `std::io` types are wrapped in `StdIo` instead.

```toml
[dependencies]
hadris-io = { version = "2.4.0", default-features = false, features = ["sync"] }
```

`hadris-fat` reads a block device rather than a stream; see
[Implement a block device for FAT](#implement-a-block-device-for-fat) below.

## Implement the Hadris traits

`hadris_io::Read`, `Write`, and `Seek` have no associated error type. Each
method returns `hadris_io::Result<T>`. Implement only `read`, `write` and
`flush`, and `seek`; the other methods have defaults.

```rust,no_run
use hadris_io::{Error, ErrorKind, Read, Result, Seek, SeekFrom};

struct FirmwareDisk {
    position: u64,
    len: u64,
}

impl Read for FirmwareDisk {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        // Read from the firmware or device protocol into `buf`, then advance
        // `self.position`. Report failures as a `hadris_io::Error`.
        let _ = buf;
        Err(Error::new(ErrorKind::Unsupported, "firmware read not implemented"))
    }
}

impl Seek for FirmwareDisk {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let target = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(offset) => self.len.checked_add_signed(offset),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
        };
        match target {
            Some(position) if position <= self.len => {
                self.position = position;
                Ok(position)
            }
            _ => Err(Error::new(ErrorKind::InvalidInput, "seek outside the device")),
        }
    }
}

let disk = FirmwareDisk { position: 0, len: 64 * 1024 * 1024 };
// Pass `disk` to a format crate that reads streams.
```

`Error::new` takes a portable `ErrorKind` and a static message, so it works
without an allocator. With `alloc`, `Error::with_source(kind, err)` keeps a
device-specific error as the source, and `downcast_source` recovers it.

`&mut FirmwareDisk` implements the same traits, so a caller can pass
`&mut disk` to a format crate and keep ownership of the device.

## Wrap an `embedded-io` device

A device that already implements the `embedded-io` traits is wrapped in
`FromEmbedded`. Its error becomes the source of the `hadris_io::Error`, and its
`embedded_io::ErrorKind` maps to the matching `hadris_io::ErrorKind`.

```toml
[dependencies]
embedded-io = "0.7"
```

```rust,no_run
use embedded_io::{ErrorKind, ErrorType, Read, Seek, SeekFrom};
use hadris_io::FromEmbedded;

struct FirmwareDisk {
    // Firmware protocol handle and current position.
}

impl ErrorType for FirmwareDisk {
    type Error = ErrorKind;
}

impl Read for FirmwareDisk {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        // Read from the firmware or device protocol into `buf`.
        todo!()
    }
}

impl Seek for FirmwareDisk {
    fn seek(&mut self, position: SeekFrom) -> Result<u64, Self::Error> {
        // Validate the requested position and update the device cursor.
        todo!()
    }
}

let disk = FromEmbedded::new(FirmwareDisk { /* ... */ });
// Pass `disk` to a format crate that reads streams.
```

The device error type can be any `embedded_io::Error` that is
`Send + Sync + 'static`; `ErrorKind` is used here for brevity. With the `async`
feature, `FromEmbedded` wraps `embedded-io-async` devices for the async traits
in the same way.

## Implement a block device for FAT

`hadris_fat::sync::FatFs` mounts any `hadris_storage::sync::BlockDevice`.
A device reports its block size and count and reads whole blocks; a read-only
device leaves `write_blocks` to its default, which answers
`WriteError::ReadOnly`. Its error type implements `core::error::Error`, so
`FatFs` can carry it inside `hadris_fs::Error`. No allocator is needed.

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["sync"] }
hadris-io = { version = "2.4.0", default-features = false, features = ["sync"] }
hadris-storage = { version = "2.4.0", default-features = false, features = ["sync"] }
```

```rust,no_run
use core::fmt;

use hadris_io::ErrorType;
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize};

#[derive(Debug)]
struct FirmwareError;

impl fmt::Display for FirmwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("firmware block read failed")
    }
}

impl core::error::Error for FirmwareError {}

struct FirmwareDisk {
    blocks: u64,
}

impl ErrorType for FirmwareDisk {
    type Error = FirmwareError;
}

impl BlockDevice for FirmwareDisk {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }

    fn block_count(&self) -> u64 {
        self.blocks
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), FirmwareError> {
        // Read `buf.len() / 512` blocks starting at `first` from the device.
        let _ = (first, buf);
        Err(FirmwareError)
    }
}

let disk = FirmwareDisk { blocks: 131_072 };
let volume = hadris_fat::sync::FatFs::open(disk);
```

A byte stream implementing the `hadris-io` traits becomes a block device
through `hadris_storage::sync::StreamDevice`, and `std::fs::File` and
`hadris_storage::MemDevice` are block devices already.

## Device requirements

The device must provide the access pattern required by the format. Mounted
filesystems generally need `Read + Seek`; mutation adds `Write`. Return short
reads only when the device genuinely has fewer bytes available, and reject
seeks outside the device rather than wrapping arithmetic.

For logical-block-native hardware, implement the traits in `hadris-storage`
and use its seekable block-device adapter. Keep the physical block size and the
filesystem's logical sector size distinct.

For memory-backed parsing without `std`, use `hadris_io::Cursor` over a caller
provided byte slice.
