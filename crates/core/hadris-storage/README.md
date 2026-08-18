# hadris-storage

Format-neutral block geometry, device capabilities, and bounded storage views
for Hadris. The crate does not assume 512-byte sectors and does not define
filesystem concepts such as FAT clusters or ISO logical sectors.

Use it to adapt logical-block hardware, validate block ranges, or restrict a
larger disk stream to one partition before opening a filesystem.

## Core types

| Type | Purpose |
|---|---|
| `BlockGeometry` | Logical block size, block count, and physical alignment hint |
| `BlockRange` | Checked contiguous range of logical blocks |
| `BlockDevice` | Whole-block read capability |
| `BlockDeviceMut` | Whole-block write and flush capability |
| `SeekBlockDevice` | Adapts a seekable byte stream to block operations |
| `PartitionView` | Bounds byte-stream reads, writes, and seeks to one region |

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

## Bounded partition views

```rust
use hadris_storage::PartitionView;

let mut disk = std::io::Cursor::new(vec![0_u8; 1024 * 1024]);
let partition = PartitionView::new(&mut disk, 64 * 1024, 256 * 1024)?;
assert_eq!(partition.len(), 256 * 1024);
# Ok::<(), hadris_storage::Error<hadris_io::ErrorKind>>(())
```

`PartitionView` translates partition-relative positions to the backing stream
and rejects out-of-range seeks. Use `hadris-block` when partition-table parsing
and filesystem detection are also required.

## Features

| Feature | Default | Purpose |
|---|---:|---|
| `std` | Yes | Hosted byte-stream support and `alloc` |
| `alloc` | Via `std` | Allocation support forwarded to `hadris-io` |
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
