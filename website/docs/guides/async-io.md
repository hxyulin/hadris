---
title: Use asynchronous I/O
---

# Use asynchronous I/O

Hadris async APIs are runtime-neutral. They depend on async I/O traits, not on
Tokio, async-std, or an executor. The application supplies a compatible reader
and drives the future with its chosen runtime.

Every filesystem driver reads a `hadris::storage::async_::BlockDevice`:
`MemDevice` implements it for bytes in memory, and a custom driver implements
it for real hardware. The CPIO reader and writer take
`hadris::io::async_::{Read, Write}` streams instead; wrap an
`embedded-io-async` device in `hadris::io::FromEmbedded` (the `embedded-io`
feature on a direct `hadris-io` dependency), or use `hadris::io::Cursor` for bytes in memory. `StdIo` covers only
the sync traits. See [embedded I/O adapters](./custom-io.md#wrap-an-embedded-io-device)
for the additional dependency and feature recipe.

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["alloc", "async", "fat"]
```

```rust
use hadris::fat::async_::FatFs;
use hadris::fs::{DirCursor, FsResult, MountOptions};
use hadris::storage::{BlockSize, MemDevice};

async fn list_root(image: &[u8]) -> FsResult<(), core::convert::Infallible> {
    let dev = MemDevice::new(image, BlockSize::new(512).unwrap());
    let mut volume = FatFs::mount(dev, MountOptions::new()).await?;
    let root = volume.root();
    let mut cursor = DirCursor::START;

    while let Some(entry) = volume.readdir(root, cursor).await? {
        println!("{}", entry.name().to_str().unwrap_or("?"));
        cursor = entry.next_cursor();
    }

    Ok(())
}
```

`hadris::fs::async_::Volume` adds the path methods (`read_dir`, `metadata`,
`open`, `create_dir` and so on) as async methods, with `File` and `ReadDir`
handles. The stronger filesystem contract guarantees Send futures, so Tokio and other
multi-threaded executors can spawn them from generic code.

When several modes are enabled, use explicit namespaces:

```rust,ignore
use hadris::fat::sync::FatFs as SyncFatFs;
use hadris::fat::async_::FatFs as AsyncFatFs;
```

## Send and local executors

One filesystem type accepts both local and Send block devices. Implement
`hadris::storage::async_::BlockDevice` with operation-owned state and poll
hooks. `SendBlockDevice` follows automatically when both the device and its
state are Send. Generic code requiring Send operation futures uses that
stronger bound; a Send device value alone is insufficient.

Use `hadris::fs::local::FileSystem` and `local::Volume` for local devices, or
`hadris::fs::async_::FileSystem` and `async_::Volume` for the stronger Send
contract. The `async` feature enables both. Device errors remain
`Send + Sync + 'static` in every mode. `r#async` aliases `async_`.

CPIO still consumes the existing Send byte-stream contracts. Its local-stream
migration is separate from the block-device change. See the
[custom device guide](./custom-io.md) for poll-native stream adapters and
explicit blocking compatibility.

## Limitations

- The `hadris-fs` `host` module (`read_tree`, `write_tree`, `file`) is
  sync-only. Host files in a tree are read only by the sync writers; the
  async writers refuse them with `Unsupported` before writing anything.
- Filesystem drivers need random access through a block device; network
  streams generally need a buffering or range-request adapter.

Enable `alloc` for the FAT and exFAT drivers, the async `Volume`, owned
values and the image writers. FAT and exFAT checking, ISO 9660, UDF and NTFS
reading, CPIO reading and partition scanning need no allocator in any mode.
