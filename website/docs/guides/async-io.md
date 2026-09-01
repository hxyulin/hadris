---
title: Use asynchronous I/O
---

# Use asynchronous I/O

Hadris async APIs are runtime-neutral. They depend on async I/O traits, not on
Tokio, async-std, or an executor. The application supplies a compatible reader
and drives the future with its chosen runtime.

```toml
[dependencies]
hadris-fat = {
  version = "2.3.0",
  default-features = false,
  features = ["alloc", "async", "read", "lfn"]
}
hadris-io = { version = "2.3.0", default-features = false, features = ["async"] }
```

```rust
use hadris_fat::r#async::FatVolume;
use hadris_io::Cursor;

async fn list_root(image: &[u8]) -> hadris_fat::Result<()> {
    let volume = FatVolume::open(Cursor::new(image)).await?;
    let root = volume.root_dir();
    let mut entries = root.entries();

    while let Some(entry) = entries.next_entry().await {
        let entry = entry?;
        if let Some(file) = entry.as_entry() {
            println!("{}", file.name());
        }
    }

    Ok(())
}
```

When both modes are enabled, use explicit namespaces:

```rust,ignore
use hadris_fat::sync::FatVolume as SyncFatVolume;
use hadris_fat::r#async::FatVolume as AsyncFatVolume;
```

## Limitations

- FAT caching and analysis tools are sync-only.
- The exFAT preview is sync-only.
- The hybrid `hadris-cd` writer is sync-only.
- The underlying reader must support seeking; network streams generally need a
  buffering or range-request adapter.

Enable `alloc` when directory names or file contents must be returned as owned
values. FAT, ISO, CPIO, and partition parsing have narrower allocation-free
async tiers; UDF filesystem traversal and NTFS reading require `alloc`.
