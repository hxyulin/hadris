---
title: no_std and embedded use
---

# Use Hadris without the standard library

Disable default features, then select the platform, I/O mode, and capabilities
that the target needs:

```toml
[dependencies.hadris]
version = "3.0.0-rc.1"
git = "https://github.com/hxyulin/hadris"
branch = "main"
default-features = false
features = ["alloc", "sync", "fat"]
```

Stable format crates have no `read` feature: reading is always compiled. `FatFs` and
`ExFatFs` read and write with `alloc`, which holds their node table, and the
`write` feature adds `format` and, with `alloc`, the tree writer `write`.
FAT and exFAT `format` and `check` run on an unmounted device without an
allocator, and the ISO 9660, UDF and NTFS readers and the CPIO reader need
no allocator either.

Add `alloc` for the FAT and exFAT drivers, for the image writers (ISO 9660,
UDF, the ISO/UDF bridge, CPIO, FAT and exFAT), which take a
`hadris::fs::Tree`, and for owned names, `copy_tree`, `read_tree` and the
async `Volume`.
`std` implies `alloc` but does not select `sync` or `async`.

All storage I/O flows through `hadris-storage` block devices (every
filesystem driver) or `hadris-io` streams (CPIO), so callers adapt firmware,
kernel, memory, or device-specific readers rather than depending on
`std::io`.

`hadris::fs::sync::Volume` requires `std`; allocation-only sync applications
use the bare driver. The async `Volume` requires `alloc` instead. For FAT
without an allocator, use `hadris::fat::embedded`, not `FatFs`.

## Choose the narrowest tier

| Need | Features |
|---|---|
| Allocation-free synchronous reader | `sync` |
| Allocation-free asynchronous reader | `async` |
| FAT read and write or exFAT read without an allocator (the embedded API) | `sync` or `async` |
| FAT or exFAT shared drivers (`FatFs`, `ExFatFs`) | Add `alloc` |
| FAT or exFAT formatting | Add `write`; no allocator required |
| Image writers, owned names or buffers | Add `alloc` |
| Several I/O modes | Enable each; the APIs live in separate namespaces |

See the complete [feature and capability matrix](../concepts/features.md).

For integration examples, see [Adapt a custom device](./custom-io.md) and
[Use asynchronous I/O](./async-io.md). Firmware on a microcontroller should
read [Use FAT and exFAT on a microcontroller](./embedded.md), which has the
flash and stack budget per target.
