---
title: Features and capabilities
---

# Features and capabilities

Hadris separates three decisions that many crates combine:

1. **Platform support:** allocation-free, `alloc`, or `std`
2. **I/O mode:** `sync`, `async`, or both
3. **Capability:** `read`, `write`, detection, caching, or tooling

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
  version = "2.2.0",
  default-features = false,
  features = ["alloc", "read", "sync", "async", "lfn"]
}
```

Use `hadris_fat::sync` and `hadris_fat::async` explicitly when both modes are
enabled. The crate-root re-exports remain available when `sync` is enabled for
backward compatibility.

Some components are intentionally sync-only: FAT caching and analysis tools,
the exFAT preview, and the hybrid ISO/UDF writer.

## Format capability matrix

| Crate | Formats or role | Read | Write/create | Sync | Async | Minimum for reading | Stability |
|---|---|---:|---:|---:|---:|---|---|
| `hadris-fat` | FAT12/16/32 | Yes | Yes | Yes | Yes | Allocation-free | Stable |
| `hadris-fat` `unstable-exfat` | exFAT preview | Partial | Partial | Yes | No | `alloc` | Experimental |
| `hadris-part` | MBR and GPT | Yes | Yes | Yes | Yes | Allocation-free | Stable |
| `hadris-iso` | ISO 9660, Joliet, Rock Ridge | Yes | Yes | Yes | Yes | Allocation-free | Stable |
| `hadris-udf` | UDF 1.02 | Yes | Yes | Yes | Yes | `alloc` for filesystem traversal | Stable |
| `hadris-cpio` | CPIO newc and CRC | Yes | Yes | Yes | Yes | Allocation-free | Stable |
| `hadris-ntfs` | NTFS | Yes | No | Yes | Yes | `alloc` | Experimental |
| `hadris-cd` | Hybrid ISO/UDF images | N/A | Yes | Yes | No | `std` | Stable |

“Allocation-free” means the core parser can operate without a global
allocator. Higher-level conveniences such as owned filenames, collected
directory trees, or image construction may still require `alloc`.

## Common configurations

### Bootloader reading FAT

```toml
hadris-fat = {
  version = "2.2.0",
  default-features = false,
  features = ["read", "sync"]
}
```

### Kernel with an allocator and async I/O

```toml
hadris-iso = {
  version = "2.2.0",
  default-features = false,
  features = ["alloc", "read", "async", "joliet"]
}
```

### Hosted FAT editor

```toml
hadris-fat = {
  version = "2.2.0",
  features = ["cache", "tool"]
}
```

### Allocation-only CPIO writer

```toml
hadris-cpio = {
  version = "2.2.0",
  default-features = false,
  features = ["alloc", "read", "write", "sync"]
}
```

## Feature selection rules

- Select exactly the formats and capabilities the application uses.
- Select at least one I/O mode for APIs that access storage.
- Add `alloc` only when the chosen API returns or stores owned data.
- Prefer leaf crates when only one format is needed.
- Treat `unstable-exfat` and `hadris-ntfs` as separately versioned experiments.

The workspace CI checks representative allocation-free, `alloc`, `std`, sync,
async, and combined-mode tiers for every stable format crate.
