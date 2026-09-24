# hadris-storage

Block devices for Hadris. Filesystems read and write whole logical blocks
through `BlockDevice`, whose block size is explicit and non-zero. The crate
does not assume 512-byte sectors and does not define filesystem concepts such
as FAT clusters or ISO logical sectors.

Every device reports its own error through `hadris_io::ErrorType`. Writes
return `WriteError<E>`, whose `ReadOnly` variant is how a device refuses a
write. There is no `writable()` query: a static flag is wrong for an SD card
whose lock switch moves while mounted and for a `std::fs::File` that cannot
tell how it was opened, and a probe write wears flash. A read-only device
implements `block_size`, `block_count` and `read_blocks`, and nothing else.

## Core types

| Type | Purpose |
|---|---|
| `BlockDevice` | Whole-block reads, optional writes and flush. `&mut D`, `Box<D>` and, with `std`, `std::fs::File` implement it too |
| `WriteError<E>` | `ReadOnly`, or the device's own error |
| `StreamDevice` | A block device over any `Read + Seek` stream, with any block size. Wrap read-only streams in `ReadOnly`; the sealed `StreamWrite` trait carries the choice |
| `MemDevice` | A block device over `&[u8]` (read-only), `&mut [u8]`, `[u8; N]`, `Vec<u8>` or `Box<[u8]>`. Error `OutOfRange` |
| `Slice` | A contiguous block range of another device, such as a partition. Requests past its end never reach the device |
| `Cache` | Write-back LRU cache of whole blocks (`alloc`). Its first write goes straight through, so a read-only device says so at once. Requests of at least `capacity` blocks bypass it |
| `ByteView` | Byte-granular reads and writes over a device, also usable as a stream |
| `StorageError<E>` | Error of the adapters that can refuse a request themselves (`StreamDevice`, `Slice`, `ByteView`) |
| `BlockIndex`, `BlockCount`, `BlockSize` | Value types with private fields and `const fn` constructors and accessors |
| `BlockGeometry`, `BlockRange` | Checked block geometry and ranges |

## Opening an image

A host file is a device with 512-byte blocks, and its errors are the
`std::io::Error` itself. Disk devices such as `/dev/sdb`, `/dev/disk4`,
`/dev/md0` or `\\.\PhysicalDrive1` work too: `file_len` measures them with
the platform's disk size request (seeking to the end on Linux), since their
metadata reports 0, and fails rather than report 0 bytes when it cannot:

```rust,no_run
use hadris_storage::BlockIndex;
use hadris_storage::sync::{BlockDevice, Slice};

let disk = std::fs::File::open("disk.img")?;
let mut partition = Slice::new(disk, BlockIndex::new(2048), 65536).expect("partition fits");

let mut sector = [0_u8; 512];
partition.read_blocks(BlockIndex::new(0), &mut sector)?;
# Ok::<(), std::io::Error>(())
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
| `std` | Yes | Hosted byte-stream support and `alloc` |
| `alloc` | Via `std` | `Cache`, `Box` impls, and block sizes above 4096 bytes in `ByteView` |
| `sync` | Yes | Synchronous device traits and adapters |
| `async` | No | Asynchronous device traits and adapters |
| `async-send` | No | Asynchronous devices with `Send` futures in `async_send`; implies `async` |

`std` and the I/O mode are independent. Disable default features and select
`sync`, `async`, or both explicitly for custom configurations.

## Documentation

- [Storage and I/O model](https://hxyulin.github.io/hadris/concepts/storage-model)
- [Adapt a custom device](https://hxyulin.github.io/hadris/guides/custom-io)
- [API reference](https://docs.rs/hadris-storage)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
