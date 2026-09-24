---
title: Use asynchronous I/O
---

# Use asynchronous I/O

Hadris async APIs are runtime-neutral. They depend on async I/O traits, not on
Tokio, async-std, or an executor. The application supplies a compatible reader
and drives the future with its chosen runtime.

A reader implements `hadris_io::r#async::{Read, Seek}` directly, or wraps an
`embedded-io-async` device in `hadris_io::FromEmbedded`. `hadris_io::Cursor`
implements the async traits for in-memory images. `StdIo` covers only the sync
traits. `FatFs` reads a `hadris_storage::r#async::BlockDevice` instead:
`MemDevice` implements it for bytes in memory, and a custom driver implements
it for real hardware.

```toml
[dependencies]
hadris-fat = { version = "2.4.0", default-features = false, features = ["async"] }
hadris-fs = { version = "2.4.0", default-features = false, features = ["async"] }
hadris-storage = { version = "2.4.0", default-features = false, features = ["async"] }
```

```rust
use hadris_fat::r#async::FatFs;
use hadris_fs::{DirCursor, FsResult, NameBuf};
use hadris_storage::{BlockSize, MemDevice, OutOfRange};

async fn list_root(image: &[u8]) -> FsResult<(), OutOfRange> {
    let dev = MemDevice::new(image, BlockSize::new(512).unwrap());
    let mut volume = FatFs::open(dev).await?;
    let root = volume.root();
    let mut cursor = DirCursor::start();
    let mut name = NameBuf::new();

    while let Some(_entry) = volume.read_dir_entry(root, &mut cursor, &mut name).await? {
        println!("{}", name.as_name().and_then(|n| n.to_str().ok()).unwrap_or("?"));
    }

    Ok(())
}
```

This needs no allocator. With `alloc`, `hadris_fs::r#async::DriverExt` adds the
path helpers (`read_dir`, `read_to_vec`, `write_file` and so on) as async
methods. Tokio and other multi-threaded executors that need `Send` futures from
generic code use the `async-send` feature and `hadris_fat::async_send::FatFs`.

When several modes are enabled, use explicit namespaces:

```rust,ignore
use hadris_fat::sync::FatFs as SyncFatFs;
use hadris_fat::r#async::FatFs as AsyncFatFs;
```

## Limitations

- The `hadris-fs` host helpers (`extract_to_host`, `import_from_host`) are
  sync-only.
- The underlying reader must support seeking; network streams generally need a
  buffering or range-request adapter.

Enable `alloc` when directory names or file contents must be returned as owned
values. FAT reading, writing, formatting and checking need no allocator; ISO,
CPIO, and partition parsing have narrower allocation-free async tiers; UDF filesystem traversal and NTFS reading require `alloc`.
