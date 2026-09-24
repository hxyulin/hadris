---
title: Features and capabilities
---

# Features and capabilities

Hadris separates three decisions that many crates combine:

1. **Platform support:** allocation-free, `alloc`, or `std`
2. **I/O mode:** `sync`, `async`, or both
3. **Capability:** `read`, `write`, detection, or formatting

Choose each dimension explicitly when disabling default features. Enabling
`std` provides heap allocation, but it does not implicitly select `sync` or
`async`.

## Platform features

| Configuration | Available facilities | Typical targets |
|---|---|---|
| No platform feature | Stack and caller-provided buffers only | Bootloaders, early kernels, small firmware |
| `alloc` | `Vec`, `String`, owned paths and trees | Kernels and firmware with a global allocator |
| `std` | Hosted files, clocks, OS errors, and `alloc` | CLI tools, desktop applications, build systems |

Not every operation can be allocation-free. Creating filesystem images and
holding arbitrary directory trees generally requires `alloc`; ISO and UDF
authoring currently require `std`.

## I/O modes

The `sync` and `async` features select parallel API namespaces backed by
`hadris-io` traits. They may be enabled together.

```toml
[dependencies]
hadris-fat = {
  version = "2.4.0",
  default-features = false,
  features = ["sync", "async", "write"]
}
```

Use `hadris_fat::sync` and `hadris_fat::r#async` explicitly when both modes are
enabled; `hadris-fat` has no crate-root re-exports of either mode. Its
`async-send` feature adds a third namespace, `async_send`, whose futures are
`Send` for multi-threaded executors.

Some components are intentionally sync-only: the `hadris-fs` host helpers,
the exFAT preview, and the hybrid ISO/UDF writer. `hadris-iso` has the same
three namespaces as `hadris-fat`, with its writer and sessions in each.

## Format capability matrix

| Crate | Formats or role | Read | Write/create | Sync | Async | Minimum for reading | Stability |
|---|---|---:|---:|---:|---:|---|---|
| `hadris-fat` | FAT12/16/32 | Yes | Yes | Yes | Yes | Allocation-free (writing, formatting and checking too) | Stable |
| `hadris-fat` `unstable-exfat` | exFAT preview | Partial | Partial | Yes | No | `alloc` | Experimental |
| `hadris-part` | MBR (with logical partitions), GPT, hybrid MBR | Yes | Yes | Yes | Yes | Allocation-free (`scan`, `open`) | Stable |
| `hadris-iso` | ISO 9660, Joliet, Rock Ridge, El Torito | Yes | Yes | Yes | Yes | Allocation-free (writing and sessions need `alloc`) | Stable |
| `hadris-udf` | UDF 1.02 to 2.01, type 1 partitions | Yes | Yes | Yes | Yes | Allocation-free (writing needs `alloc`) | Stable |
| `hadris-cpio` | CPIO newc, CRC and odc; old binary read | Yes | Yes | Yes | Yes | Allocation-free (writing needs `alloc`) | Stable |
| `hadris-ntfs` | NTFS | Yes | No | Yes | Yes | Allocation-free | Experimental |
| `hadris-cd` | Hybrid ISO/UDF images | N/A | Yes | Yes | Yes | `alloc` | Stable |

“Allocation-free” means the core parser can operate without a global
allocator. Higher-level conveniences such as owned filenames, collected
directory trees, or image construction may still require `alloc`.

## Common configurations

### Bootloader reading FAT

```toml
hadris-fat = {
  version = "2.4.0",
  default-features = false,
  features = ["sync"]
}
```

`hadris-fat` has no `read` feature: reading and writing are always available,
and `write` adds only the formatter.

### Kernel with an allocator and async I/O

```toml
hadris-iso = {
  version = "2.4.0",
  default-features = false,
  features = ["alloc", "async"]
}
```

### Hosted FAT editor

```toml
hadris-fat = "2.4.0"      # std, sync and write
hadris-fs = "2.4.0"       # path and host helpers
hadris-storage = "2.4.0"  # Cache<D> for block caching
```

The checker (`check`, `check_with`) is always compiled; block caching comes
from wrapping the device in `hadris_storage::sync::Cache`.

### Allocation-only CPIO writer

```toml
hadris-cpio = {
  version = "2.4.0",
  default-features = false,
  features = ["alloc", "sync"]
}
```

`hadris-cpio` has no `read` or `write` feature: the reader is always
compiled, and `alloc` adds the writer.

## Feature selection rules

- Select exactly the formats and capabilities the application uses.
- Select at least one I/O mode for APIs that access storage.
- Add `alloc` only when the chosen API returns or stores owned data.
- Prefer leaf crates when only one format is needed.
- Treat `unstable-exfat`, `unstable-streaming`, and `hadris-ntfs` as separately
  versioned experiments.

The workspace CI checks representative allocation-free, `alloc`, `std`, sync,
async, and combined-mode tiers for every stable format crate.
