---
title: no_std and embedded use
---

# Use Hadris without the standard library

Disable default features, then select the platform, I/O mode, and capabilities
that the target needs:

```toml
[dependencies]
hadris-fat = {
  version = "2.4.0",
  default-features = false,
  features = ["alloc", "sync"]
}
```

No crate has a `read` feature: reading is always compiled. `FatFs` and
`ExFatFs` read and write with `alloc`, which holds their node table, and the
`write` feature adds `format`. FAT and exFAT `check` runs on an unmounted
device without an allocator, and the ISO 9660, UDF and NTFS readers and the
CPIO reader need no allocator either.

Add `alloc` for the FAT and exFAT drivers, for the image writers (ISO 9660,
UDF, hybrid CD, CPIO), which take a `hadris_fs::tree::Tree`, and for owned
names, `copy_tree` and the async `Volume`.
`std` implies `alloc` but does not select `sync` or `async`.

All storage I/O flows through `hadris-storage` block devices (every
filesystem driver) or `hadris-io` streams (CPIO), so callers adapt firmware,
kernel, memory, or device-specific readers rather than depending on
`std::io`.

## Choose the narrowest tier

| Need | Features |
|---|---|
| Allocation-free synchronous reader | `sync` |
| Allocation-free asynchronous reader | `async` |
| FAT or exFAT drivers | Add `alloc` |
| FAT or exFAT formatting | Add `alloc` and `write` |
| Image writers, owned names or buffers | Add `alloc` |
| Several I/O modes | Enable each; the APIs live in separate namespaces |

See the complete [feature and capability matrix](../concepts/features.md).

For integration examples, see [Adapt a custom device](./custom-io.md) and
[Use asynchronous I/O](./async-io.md).
