---
title: Features and capabilities
---

# Features and capabilities

Hadris separates three decisions that many crates combine:

1. **Platform support:** allocation-free, `alloc`, or `std`
2. **I/O mode:** `sync`, `async`, or both
3. **Capability:** `write` (FAT formatting), and in the umbrella crate one
   feature per format

Stable format crates always compile reading; the APFS preview still has a
`read` feature. The APIs and Cargo examples below target RC2. Until RC2 is
published, use a workspace checkout as described in
[getting started](../getting-started.md). See the [async guide](../guides/async-io.md)
for custom device migration. Choose each dimension
explicitly when disabling default features. Enabling `std` provides heap
allocation, but it does not implicitly select `sync` or `async`. A feature
only adds items: none changes what an existing item does, and only
`unstable-*` features add APIs outside the stability promise.

## Platform features

| Configuration | Available facilities | Typical targets |
|---|---|---|
| No platform feature | Stack and caller-provided buffers only | Bootloaders, early kernels, small firmware |
| `alloc` | `Vec`, `String`, owned names and trees | Kernels and firmware with a global allocator |
| `std` | Hosted files, clocks, OS errors, and `alloc` | CLI tools, desktop applications, build systems |

Not every operation can be allocation-free. The image writers (ISO 9660, UDF,
the ISO/UDF bridge, CPIO, and FAT and exFAT `write`) take a `hadris::fs::Tree`
and need `alloc`; `std` adds host files as tree content. The FAT and exFAT
drivers, `FatFs` and `ExFatFs`, need `alloc` for their node table. `format`,
which returns the new volume's geometry, and `check` run on an unmounted
device without an allocator.

## I/O modes

The `sync` and `async` features select parallel API namespaces generated
from one source. They may be enabled together.

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["alloc", "sync", "async", "write", "fat"]
```

Name I/O types through their mode module, `hadris::fat::sync` or
`hadris::fat::async_`; `r#async` remains a compatibility alias. Block-format
drivers use one async type for local and Send devices. Implement
`hadris::storage::async_::BlockDevice` once; `SendBlockDevice` is derived when
both the device and its operation state are Send. A Send device alone does not
guarantee Send operation futures.

Use `hadris::fs::local::{FileSystem, Volume}` for local callers and
`hadris::fs::async_::{FileSystem, Volume}` for generic callers requiring Send
futures. The `async` feature enables both filesystem tiers. Storage's `local`
namespace aliases `async_`; byte-stream traits in `hadris-io` still have
separate local and Send contracts, and CPIO retains its Send stream API.
Device errors remain `Send + Sync + 'static` in every mode.

The sync and async APIs retain mode-specific differences: `hadris-fs::host`
uses blocking `std::fs` and is sync-only. A sync `Volume` needs `std`; async
volumes need `alloc` and pointer-sized atomics. Local volumes do not expose
lazy `read_tree`, whose content sources still require Send and Sync.

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
| `hadris-apfs` | APFS containers and volumes | Yes | No | Yes | Yes | `alloc` | Preview |
| `hadris` `detect` | Detection of every format above, opening FAT, exFAT, ISO 9660, UDF and single-volume APFS as `AnyFs` | Yes | N/A | Yes | Yes | Allocation-free detection; `open` needs `alloc` | Stable |

"Allocation-free" means the core parser can operate without a global
allocator. Higher-level conveniences such as owned filenames, collected
directory trees, or image construction may still require `alloc`.

## Common configurations

### Bootloader reading FAT

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["alloc", "sync", "fat"]
```

`hadris-fat` has no `read` feature: with `alloc`, reading and writing are
always available, and `write` adds only the formatter.

### Kernel with an allocator and async I/O

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["alloc", "async", "iso"]
```

### Hosted FAT editor

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["std", "sync", "write", "fat"]
```

The checker (`check`) is always compiled; block caching comes
from wrapping the device in `hadris::storage::sync::Cache`.

### Allocation-only CPIO writer

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["alloc", "sync", "cpio"]
```

`hadris-cpio` has no `read` or `write` feature: the reader is always
compiled, and `alloc` adds the writer.

## Feature selection rules

- Select exactly the formats and capabilities the application uses.
- Select at least one I/O mode for APIs that access storage.
- Add `alloc` only when the chosen API returns or stores owned data.
- Use the umbrella with defaults disabled for one dependency and one format; use leaf crates for direct ownership of their versions.
- Treat `hadris-ntfs` and `unstable-ntfs` as a preview whose native API may
  change in minor releases.

The workspace CI checks representative allocation-free, `alloc`, `std`, sync,
async, and combined-mode tiers for every stable format crate.
