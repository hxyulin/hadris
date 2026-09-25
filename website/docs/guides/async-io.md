---
title: Use asynchronous I/O
---

# Use asynchronous I/O

Hadris async APIs are runtime-neutral. They depend on async I/O traits, not on
Tokio, async-std, or an executor. The application supplies a compatible reader
and drives the future with its chosen runtime.

Every filesystem driver reads a `hadris_storage::r#async::BlockDevice`:
`MemDevice` implements it for bytes in memory, and a custom driver implements
it for real hardware. The CPIO reader and writer take
`hadris_io::r#async::{Read, Write}` streams instead; wrap an
`embedded-io-async` device in `hadris_io::FromEmbedded` (the `embedded-io`
feature), or use `hadris_io::Cursor` for bytes in memory. `StdIo` covers only
the sync traits.

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["alloc", "async"] }
hadris-fs = { version = "2.4.0", default-features = false, features = ["async"] }
hadris-storage = { version = "2.4.0", default-features = false, features = ["async"] }
```

```rust
use hadris_fat::r#async::FatFs;
use hadris_fs::r#async::FileSystem;
use hadris_fs::{DirCursor, FsResult, MountOptions};
use hadris_storage::{BlockSize, MemDevice};

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

`hadris_fs::r#async::Volume` adds the path methods (`read_dir`, `metadata`,
`open`, `create_dir` and so on) as async methods, with `File` and `ReadDir`
handles. The futures are `Send` when the device is, so Tokio and other
multi-threaded executors can spawn them from generic code.

When several modes are enabled, use explicit namespaces:

```rust,ignore
use hadris_fat::sync::FatFs as SyncFatFs;
use hadris_fat::r#async::FatFs as AsyncFatFs;
```

## Limitations

- The `hadris-fs` `host` module (`read_tree`, `write_tree`, `file`) is
  sync-only. Host files in a tree are read only by the sync writers; the
  async writers refuse them with `Unsupported` before writing anything.
- Filesystem drivers need random access through a block device; network
  streams generally need a buffering or range-request adapter.

Enable `alloc` for the FAT and exFAT drivers, the async `Volume`, owned
values and the image writers. FAT and exFAT checking, ISO 9660, UDF and NTFS
reading, CPIO reading and partition scanning need no allocator in any mode.
