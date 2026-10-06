---
title: Features and capabilities
---

# Features and capabilities

Hadris separates three decisions that many crates combine:

1. **Platform support:** allocation-free, `alloc`, or `std`
2. **I/O mode:** `sync`, `async`, or both
3. **Capability:** `write` (FAT formatting), and in the umbrella crate one
   feature per format

Reading is always compiled; there is no `read` feature. Choose each dimension
explicitly when disabling default features. Enabling `std` provides heap
allocation, but it does not implicitly select `sync` or `async`. A feature
only adds items: none changes what an existing item does, and only
`unstable-*` features add APIs outside the stability promise.

## Platform features

| Configuration | Available facilities | Typical targets |
|---|---|---|
| No platform feature | Stack and caller-provided buffers only | Bootloaders, early kernels, small firmware |
| `alloc` | `Vec`, `String`, owned paths and trees | Kernels and firmware with a global allocator |
| `std` | Hosted files, clocks, OS errors, and `alloc` | CLI tools, desktop applications, build systems |

Not every operation can be allocation-free. The image writers (ISO 9660, UDF,
the ISO/UDF bridge, CPIO, and FAT and exFAT `write`) take a `hadris_fs::Tree`
and need `alloc`; `std` adds host files as tree content. The FAT and exFAT
drivers, `FatFs` and `ExFatFs`, need `alloc` for their node table. `format`,
which returns the new volume's geometry, and `check` run on an unmounted
device without an allocator.

## I/O modes

The `sync` and `async` features select parallel API namespaces generated
from one source. They may be enabled together.

```toml
[dependencies]
hadris-fat = {
  version = "3.0.0-rc.1",
  default-features = false,
  features = ["alloc", "sync", "async", "write"]
}
```

Name I/O types through their mode module, `hadris_fat::sync` or
`hadris_fat::r#async`; no crate re-exports a mode at its root. Async
futures are `Send` when the device is, so generic code can spawn them on
multi-threaded executors. `hadris-io` and `hadris-storage` also have a
`local` namespace, whose futures need not be `Send`, for single-threaded
executors.

Every crate has the same public items in each mode. The exceptions are the
`hadris-fs` `host` module (`read_tree`, `write_tree`, `file`), which is
sync-only because the host side is blocking `std::fs`.

## Format capability matrix

| Crate | Formats or role | Read | Write/create | Sync | Async | Minimum for reading | Stability |
|---|---|---:|---:|---:|---:|---|---|
| `hadris-fat` | FAT12/16/32 | Yes | Yes | Yes | Yes | `alloc` (checking is allocation-free) | Stable |
| `hadris-fat` `exfat` | exFAT, including TexFAT volumes with two FATs | Yes | Yes | Yes | Yes | `alloc` (checking is allocation-free) | Stable |
| `hadris-part` | MBR (with logical partitions), GPT, hybrid MBR | Yes | Yes | Yes | Yes | Allocation-free (`scan`, `open`) | Stable |
| `hadris-iso` | ISO 9660, Joliet, Rock Ridge, El Torito | Yes | Yes | Yes | Yes | Allocation-free (writing and sessions need `alloc`) | Stable |
| `hadris-udf` | UDF 1.02 to 2.01, type 1 partitions; ISO 9660 and UDF bridge images | Yes | Yes | Yes | Yes | Allocation-free (writing needs `alloc`) | Stable |
| `hadris-cpio` | CPIO newc, CRC and odc; old binary read | Yes | Yes | Yes | Yes | Allocation-free (writing needs `alloc`) | Stable |
| `hadris-ntfs` | NTFS | Yes | No | Yes | Yes | Allocation-free | Preview |
| `hadris` `detect` | Detection of every format above, opening FAT, exFAT, ISO 9660 and UDF as `AnyFs` | Yes | N/A | Yes | Yes | Allocation-free detection; `open` needs `alloc` | Stable |

"Allocation-free" means the core parser can operate without a global
allocator. Higher-level conveniences such as owned filenames, collected
directory trees, or image construction may still require `alloc`.

## Common configurations

### Bootloader reading FAT

```toml
hadris-fat = {
  version = "3.0.0-rc.1",
  default-features = false,
  features = ["alloc", "sync"]
}
```

`hadris-fat` has no `read` feature: with `alloc`, reading and writing are
always available, and `write` adds only the formatter.

### Kernel with an allocator and async I/O

```toml
hadris-iso = {
  version = "3.0.0-rc.1",
  default-features = false,
  features = ["alloc", "async"]
}
```

### Hosted FAT editor

```toml
hadris-fat = "3.0.0-rc.1"      # std, sync and write
hadris-fs = "3.0.0-rc.1"       # Volume, MountOptions and host helpers
hadris-storage = "3.0.0-rc.1"  # Cache<D> for block caching
```

The checker (`check`) is always compiled; block caching comes
from wrapping the device in `hadris_storage::sync::Cache`.

### Allocation-only CPIO writer

```toml
hadris-cpio = {
  version = "3.0.0-rc.1",
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
- Treat `hadris-ntfs` and `unstable-ntfs` as a preview whose native API may
  change in minor releases.

The workspace CI checks representative allocation-free, `alloc`, `std`, sync,
async, and combined-mode tiers for every stable format crate.
