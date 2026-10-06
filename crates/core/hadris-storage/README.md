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
underneath. Errors must be `core::error::Error + Send + Sync + 'static` in
every mode. `local` permits non-`Send` devices and futures, not non-`Send` errors.

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
| `BlockDevice` | Whole-block reads, optional writes and flush, in `sync`, `r#async` (`Send` futures) and `local` (futures need not be `Send`). `&mut D` and `Box<D>` implement it too |
| `Vec<u8>` | With `alloc`, an in-memory image with 512-byte blocks that grows when written past its end. Device error `Infallible` |
| `MemDevice` | A fixed-size block device over `&[u8]` (read-only), `&mut [u8]`, `[u8; N]`, `Vec<u8>` or `Box<[u8]>`, with any block size. Device error `Infallible`; requests past the end fail with kind `InvalidInput` |
| `Partition` | A byte window of another device, such as an MBR or GPT partition. Its offset and length are multiples of the device block size. Requests past its end never reach the device, and `disk_offset` reports its start |
| `host::FileDevice` | With `std` and `sync`, an image file (512-byte blocks by default) or disk device (OS-reported logical blocks). `open(path)` is read-only; `new(file)` takes a file the caller opened and is writable when the file is. An image file grows when written past its end |
| `StreamDevice` | A block device over any `Read + Seek` stream, with any block size. Wrap read-only streams in `ReadOnly`; the sealed `StreamWrite` trait carries the choice |
| `Cache` | Write-back LRU cache of whole blocks (`alloc`). Its first write goes straight through, so a read-only device says so at once. Requests of at least `capacity` blocks bypass it |
| `ReadAhead` | Optional, write-through read buffering (`alloc`). Two windows share a configurable block budget; adjacent access enables larger reads, while scattered misses fetch only requested blocks |
| `ByteView` | Byte-granular reads and writes over a device, also usable as a stream |
| `BlockIndex`, `BlockCount`, `BlockSize` | Value types with private fields and `const fn` constructors and accessors |
| `BlockGeometry`, `BlockRange` | Checked block geometry and ranges |

## Opening an image

`host::FileDevice` opens a host image file with 512-byte blocks by default,
and its errors are the `std::io::Error` itself. Use
`FileDevice::open_with_block_size(path, BlockSize::new(4096).unwrap())` for a
GPT image copied from a 4Kn disk, or `FileDevice::with_block_size(file, size)`
for a file you opened. Disk devices such as `/dev/sdb`,
`/dev/disk4`, `/dev/md0` or `\\.\PhysicalDrive1` work too:
`host::file_len` measures them with the platform's disk size request
(seeking to the end on Linux), since their metadata reports 0, and
physical devices use their OS-reported logical block size on Linux, macOS,
FreeBSD and Windows. `FileDevice` refuses unknown capacity or logical block
size; use an explicit block-size constructor when the size is known but the
OS query is unavailable:

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
| `async` | No | Asynchronous device traits and adapters with `Send` futures in `r#async`, and without the `Send` bound in `local` |

`std` and the I/O mode are independent. Disable default features and select
`sync`, `async`, or both explicitly for custom configurations.

## Documentation

- [Storage and I/O model](https://hxyulin.github.io/hadris/concepts/storage-model)
- [Adapt a custom device](https://hxyulin.github.io/hadris/guides/custom-io)
- [API reference](https://docs.rs/hadris-storage/3.0.0-rc.1)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).

## Bounded read-ahead

Wrap the device before mounting a filesystem to combine nearby small reads:

```rust,no_run
use hadris_storage::host::FileDevice;
use hadris_storage::sync::ReadAhead;

let device = ReadAhead::new(FileDevice::open("disk.img")?, 128);
# Ok::<(), std::io::Error>(())
```

For 512-byte blocks, 128 blocks bound the two buffers together to 64 KiB.
The buffers allocate lazily. Zero disables buffering, and direct devices remain
unchanged. The budget is clamped to the initial device size and addressable
allocation size. The adapter keeps two windows so metadata and data can alternate.
A miss adjacent to a retained window reads ahead; other misses fetch exactly the
requested blocks. Requests larger than the selected window bypass buffering.

All writes go through immediately and invalidate both windows before starting.
Reads never speculate outside the device or partition. If an expanded read
fails, the adapter retries the original request. `get_mut()` and `clear()`
invalidate retained data; callers must clear after changes through external
handles. Like any read cache, it cannot detect external modifications itself.
The same adapter is available in `sync`, `r#async`, and `local`.

## Hardware adapters

`BlockDevice` accepts caller buffers at any memory address and any whole-block
length. An adapter uses suitable bounce buffers for hardware alignment or DMA
memory restrictions and splits requests to fit transfer limits. The logical
block size does not specify buffer-address alignment. `flush` makes earlier
writes durable. Async adapters must finish or stop hardware access to borrowed
buffers before returning or when their future is dropped.

The [aligned-device example](examples/aligned_device.rs) adapts unaligned,
multi-block requests to a controller requiring 64-byte alignment and one block
per transfer. Its adapter uses only `core` and a fixed 512-byte buffer:

```sh
cargo run -p hadris-storage --no-default-features --features sync --example aligned_device
```

Raw NOR/NAND flash needs a layer providing block overwrite semantics, including
erase handling and any required translation. It cannot be treated as an ordinary
rewritable disk solely by implementing whole-block reads.

## Unified asynchronous drivers

The `async` feature enables both contracts in one canonical namespace:
`async_::BlockDevice` allows non-Send futures, and `async_::SendBlockDevice`
guarantees Send futures. Implement the common contract directly for local
I/O, or the stronger contract for Send I/O. Send implementations automatically
satisfy the common contract. ISO uses the same reader for either contract.

Existing Send adapters are also available through `async_::{Cache, ReadAhead,
StreamDevice, ByteView}`. These retain their Send requirements.
Existing `r#async` and `local` adapter paths remain compatible. Build a legacy
local adapter chain first, then wrap it in `async_::Local`; `into_inner` returns
the original chain. A new local device implementing the common contract can
mount ISO directly, without that wrapper.

See the [ISO guide](../../../docs/unified-async-iso.md) for usage and limits.
