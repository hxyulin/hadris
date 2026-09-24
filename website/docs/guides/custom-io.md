---
title: Adapt a custom device
---

# Adapt a custom device or firmware reader

Every Hadris filesystem driver reads a `hadris-storage` block device, and the
CPIO reader and writer read `hadris-io` streams. Neither needs `std::io`.
There are three ways to connect a device: implement `BlockDevice` for it,
implement the `hadris-io` stream traits and wrap the stream in a
`StreamDevice`, or wrap a device that already implements `embedded-io` in
`FromEmbedded`. Hosted `std::io` types are wrapped in `StdIo` instead.

For a filesystem, [implement a block device](#implement-a-block-device-for-fat)
directly when the hardware addresses whole blocks.

## Implement the stream traits

```toml
[dependencies]
hadris-io = { version = "2.4.0", default-features = false, features = ["sync"] }
```

`hadris_io::sync::Read`, `Write` and `Seek` report the implementor's own error
through the `ErrorType` supertrait, which can be any
`core::error::Error + Send + Sync + 'static`. Implement only `read`, `write`
and `flush`, and `seek`; the other methods have defaults.

```rust,no_run
use core::fmt;

use hadris_io::sync::{Read, Seek};
use hadris_io::{ErrorType, SeekFrom};

#[derive(Debug)]
enum DiskError {
    Io,
    BadSeek,
}

impl fmt::Display for DiskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DiskError::Io => "firmware read failed",
            DiskError::BadSeek => "seek outside the device",
        })
    }
}

impl core::error::Error for DiskError {}

struct FirmwareDisk {
    position: u64,
    len: u64,
}

impl ErrorType for FirmwareDisk {
    type Error = DiskError;
}

impl Read for FirmwareDisk {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, DiskError> {
        // Read from the firmware or device protocol into `buf`, then advance
        // `self.position`.
        let _ = buf;
        Err(DiskError::Io)
    }
}

impl Seek for FirmwareDisk {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, DiskError> {
        match pos.resolve(self.position, self.len) {
            Some(position) if position <= self.len => {
                self.position = position;
                Ok(position)
            }
            _ => Err(DiskError::BadSeek),
        }
    }
}

let disk = FirmwareDisk { position: 0, len: 64 * 1024 * 1024 };
// Pass `disk` to a stream consumer, or to `StreamDevice::new` for a filesystem.
```

The device's error reaches the caller unchanged, inside
`hadris_io::ExactError` or the format crate's error, with no allocation.
`SeekFrom` is `#[non_exhaustive]`, so resolve it with `SeekFrom::resolve`
rather than matching it.

`&mut FirmwareDisk` implements the same traits, so a caller can pass
`&mut disk` to a format crate and keep ownership of the device.

## Wrap an `embedded-io` device

A device that already implements the `embedded-io` traits is wrapped in
`FromEmbedded`, which needs the `embedded-io` feature. Its error passes
through unchanged.

```toml
[dependencies]
embedded-io = "0.7"
hadris-io = { version = "2.4.0", default-features = false, features = ["sync", "embedded-io"] }
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
// Pass `disk` to a stream consumer, or to `StreamDevice::new` for a filesystem.
```

The device error type can be any `embedded_io::Error` that is
`Send + Sync + 'static`; `ErrorKind` is used here for brevity. With the `async`
feature, `FromEmbedded` wraps `embedded-io-async` devices for the async traits
in the same way.

## Implement a block device for FAT

`hadris_fat::sync::FatFs` mounts any `hadris_storage::sync::BlockDevice`.
A device reports its block size and count and reads whole blocks; a read-only
device leaves `write_blocks` to its default, which answers kind
`ReadOnly`. Every method returns `hadris_io::Error` over the device's own
error type, which implements `core::error::Error`; `Error::device` wraps a
device failure with a static message, and `FatFs` passes it on unchanged.
No allocator is needed.

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["sync"] }
hadris-io = { version = "2.4.0", default-features = false, features = ["sync"] }
hadris-storage = { version = "2.4.0", default-features = false, features = ["sync"] }
```

```rust,no_run
use core::fmt;

use hadris_io::{Error, ErrorType, Location};
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

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<FirmwareError>> {
        // Read `buf.len() / 512` blocks starting at `first` from the device.
        let _ = buf;
        Err(Error::device(FirmwareError, "reading a block failed")
            .with_location(Location::Block(first.get())))
    }
}

let disk = FirmwareDisk { blocks: 131_072 };
let volume = hadris_fat::sync::FatFs::mount(disk, hadris_fs::MountOptions::new());
```

A byte stream implementing the `hadris-io` traits becomes a block device
through `hadris_storage::sync::StreamDevice`, and
`hadris_storage::host::FileDevice`, `Vec<u8>` and
`hadris_storage::MemDevice` are block devices already.

## Device requirements

Report the device's real block size and count, read and write whole blocks
only, and fail requests past the end rather than wrapping. A device that
cannot write leaves `writable` and `write_blocks` to their defaults: it is
not writable, so drivers mount it read-only, and a write returns kind
`ReadOnly`. A device that can write returns true from `writable` and
implements `write_blocks`; it may still refuse a write with `ReadOnly`, for
example when its media become write-protected, and drivers then stop
writing. A request past the end fails with
kind `InvalidInput`.
`flush` must make earlier writes durable, because `sync` and `fsync`
rely on it.

Keep the device's block size and the filesystem's logical sector size
distinct: the drivers read whole device blocks and take their own sector
size from the on-disk metadata.

For memory-backed parsing without `std`, use `hadris_storage::MemDevice` over
a caller-provided byte slice, or `hadris_io::Cursor` for a stream.
