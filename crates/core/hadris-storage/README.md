# hadris-storage

Block devices for Hadris. Filesystems read and write whole logical blocks
through `BlockDevice`, whose block size is explicit and non-zero. The crate
does not assume 512-byte sectors and does not define filesystem concepts such
as FAT clusters or ISO logical sectors.

## Core types

| Type | Purpose |
|---|---|
| `BlockDevice` | Whole-block reads, optional writes and flush. `&mut D` and `Box<D>` implement it too |
| `StreamDevice` | A block device over any `Read + Seek` stream, with any block size. Wrap read-only streams in `ReadOnly` |
| `MemDevice` | A block device over `&[u8]`, `&mut [u8]`, `[u8; N]`, `Vec<u8>` or `Box<[u8]>` |
| `Slice` | A contiguous block range of another device, such as a partition |
| `Cache` | Write-back LRU cache of whole blocks (`alloc`) |
| `ByteView` | Byte-granular reads and writes over a device, also usable as a stream |
| `BlockGeometry`, `BlockRange` | Checked block geometry and ranges |
| `PartitionView` | Bounds a byte stream to one region. Replaced by `Slice` as formats move to `BlockDevice` |

## Opening an image

```rust
use hadris_io::StdIo;
use hadris_storage::BlockSize;
use hadris_storage::sync::{BlockDevice, Slice, StreamDevice};
use hadris_storage::BlockIndex;

let image = StdIo::new(std::io::Cursor::new(vec![0_u8; 1024 * 1024]));
let disk = StreamDevice::new(image, BlockSize::new(512).unwrap())?;
let mut partition = Slice::new(disk, BlockIndex(128), 512)?;

let mut sector = [0_u8; 512];
partition.read_blocks(BlockIndex(0), &mut sector)?;
# Ok::<(), hadris_io::Error>(())
```

## Checked geometry

```rust
use hadris_storage::{BlockCount, BlockGeometry, BlockIndex, BlockRange, BlockSize};

let geometry = BlockGeometry::new(
    BlockSize::new(4096).unwrap(),
    BlockCount(1024),
);
let range = BlockRange::new(BlockIndex(8), BlockCount(16));
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

`std` and the I/O mode are independent. Disable default features and select
`sync`, `async`, or both explicitly for custom configurations.

## Documentation

- [Storage and I/O model](https://hxyulin.github.io/hadris/concepts/storage-model)
- [Adapt a custom device](https://hxyulin.github.io/hadris/guides/custom-io)
- [API reference](https://docs.rs/hadris-storage)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
