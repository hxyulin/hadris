# hadris-storage

Block devices for Hadris. Filesystems read and write whole logical blocks
through `BlockDevice`, whose block size is explicit and non-zero. The crate
does not assume 512-byte sectors and does not define filesystem concepts such
as FAT clusters or ISO logical sectors.

Every device names its own error through `hadris_io::ErrorType`, and every
block operation returns `hadris_io::Error<E>` over it: `Error::device` when
the device failed, kind `ReadOnly` when it refuses a write, and a kind with
the block it concerns when an adapter refuses a request itself, such as one
past the end of a `Partition`. Adapters keep the error type of the device
underneath.

A read-only device implements `block_size`, `block_count` and
`read_blocks`, and nothing else. A device that accepts writes also
overrides `writable()`, which defaults to false, and `write_blocks`.
`writable()` answers whether the device accepts writes at all; a driver
mounts a device that says false read-only. A device that says true may
still refuse a later write with `ReadOnly`, as an SD card does when its
lock switch moves while mounted. `max_block_count()` is how far a device
grows when written past its end, and `disk_offset()` is the byte offset of
block 0 on the disk a device is a window of.

## Core types

| Type | Purpose |
|---|---|
| `BlockDevice` | Whole-block reads, optional writes and flush, in `sync`, `r#async`, `async_send` (`Send` futures) and `local` (futures need not be `Send`). `&mut D` and `Box<D>` implement it too |
| `Vec<u8>` | With `alloc`, an in-memory image with 512-byte blocks that grows when written past its end. Device error `Infallible` |
| `MemDevice` | A fixed-size block device over `&[u8]` (read-only), `&mut [u8]`, `[u8; N]`, `Vec<u8>` or `Box<[u8]>`, with any block size. Device error `Infallible`; requests past the end fail with kind `InvalidInput` |
| `Partition` | A byte window of another device, such as an MBR or GPT partition. Its offset and length are multiples of the device block size. Requests past its end never reach the device, and `disk_offset` reports its start |
| `host::FileDevice` | With `std` and `sync`, a host image file or disk device with 512-byte blocks. `open(path)` is read-only; `new(file)` takes a file the caller opened and is writable when the file is. An image file grows when written past its end |
| `StreamDevice` | A block device over any `Read + Seek` stream, with any block size. Wrap read-only streams in `ReadOnly`; the sealed `StreamWrite` trait carries the choice |
| `Cache` | Write-back LRU cache of whole blocks (`alloc`). Its first write goes straight through, so a read-only device says so at once. Requests of at least `capacity` blocks bypass it |
| `ByteView` | Byte-granular reads and writes over a device, also usable as a stream |
| `BlockIndex`, `BlockCount`, `BlockSize` | Value types with private fields and `const fn` constructors and accessors |
| `BlockGeometry`, `BlockRange` | Checked block geometry and ranges |

## Opening an image

`host::FileDevice` opens a host image file with 512-byte blocks, and its
errors are the `std::io::Error` itself. Disk devices such as `/dev/sdb`,
`/dev/disk4`, `/dev/md0` or `\\.\PhysicalDrive1` work too:
`host::file_len` measures them with the platform's disk size request
(seeking to the end on Linux), since their metadata reports 0, and
`FileDevice` refuses a device it cannot measure rather than report 0
bytes:

```rust,no_run
use hadris_storage::{BlockIndex, Partition};
use hadris_storage::host::FileDevice;
use hadris_storage::sync::BlockDevice;

let disk = FileDevice::open("disk.img")?;
let mut partition = Partition::new(disk, 2048 * 512, 65536 * 512);
assert_eq!(partition.disk_offset(), 2048 * 512);

let mut sector = [0_u8; 512];
partition.read_blocks(BlockIndex::new(0), &mut sector)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Any seekable stream works through `StreamDevice`, with any block size:

```rust
use hadris_io::StdIo;
use hadris_storage::{BlockIndex, BlockSize};
use hadris_storage::sync::{BlockDevice, StreamDevice};

let image = StdIo::new(std::io::Cursor::new(vec![0_u8; 1024 * 1024]));
let mut disk = StreamDevice::new(image, BlockSize::new(2048).unwrap())?;
disk.write_blocks(BlockIndex::new(16), &[1; 2048]).unwrap();
# Ok::<(), std::io::Error>(())
```

## Checked geometry

```rust
use hadris_storage::{BlockCount, BlockGeometry, BlockIndex, BlockRange, BlockSize};

let geometry = BlockGeometry::new(
    BlockSize::new(4096).unwrap(),
    BlockCount::new(1024),
);
let range = BlockRange::new(BlockIndex::new(8), BlockCount::new(16));
assert!(geometry.contains(range));
assert_eq!(geometry.byte_len(), Some(4 * 1024 * 1024));
```

## Features

| Feature | Default | Purpose |
|---|---:|---|
| `std` | Yes | `host::FileDevice`, `host::file_len` and `alloc` |
| `alloc` | Via `std` | `Cache`, the `Vec<u8>` device, `Box` impls, and block sizes above 4096 bytes in `ByteView` |
| `sync` | Yes | Synchronous device traits and adapters |
| `async` | No | Asynchronous device traits and adapters in `r#async` and `local` |
| `async-send` | No | Asynchronous devices with `Send` futures in `async_send`; implies `async` |

`std` and the I/O mode are independent. Disable default features and select
`sync`, `async`, or both explicitly for custom configurations.

## Documentation

- [Storage and I/O model](https://hxyulin.github.io/hadris/concepts/storage-model)
- [Adapt a custom device](https://hxyulin.github.io/hadris/guides/custom-io)
- [API reference](https://docs.rs/hadris-storage)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
