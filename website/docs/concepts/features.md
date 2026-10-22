---
title: Features and capabilities
---

# Features and capabilities

Hadris separates three decisions that many crates combine:

1. **Platform support:** allocation-free, `alloc`, or `std`
2. **I/O mode:** `sync`, `async`, or both
3. **Capability:** `write` (creating images), and in the umbrella crate one
   feature per format

Every format crate always compiles reading; none has a `read` feature. See
[getting started](../getting-started.md) for the dependency recipe and the
[async guide](../guides/async-io.md) for custom devices. Choose each dimension
explicitly when disabling default features. Enabling `std` provides heap
allocation, but it does not implicitly select `sync` or `async`. A feature
only adds items: none changes what an existing item does, except
`unstable-apfs`, which makes `detect` and `open` handle APFS for the whole
build. Only `unstable-*` features add APIs outside the stability promise.

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

## The `write` feature

`write` means "can create images" in every crate that has it, and it is on
by default. In the umbrella it forwards to each enabled format crate:

| Crate | `write` adds |
|---|---|
| `hadris-fat` | `format` for FAT and exFAT, and with `alloc` the tree writer `write` |
| `hadris-iso` | `IsoOptions` and the other writer options, `plan`, `write` and `Session`; implies `alloc` |
| `hadris-udf` | `UdfOptions`, `plan` and `write`; implies `alloc`. The bridge writer (`plan_bridge`, `write_bridge`) is the separate `bridge` feature, which the umbrella's `write` enables |
| `hadris-cpio` | The streaming `Writer`, `plan` and `write`; implies `alloc` |

`alloc` on its own only means the target has a heap: it enables bounded
caches and owned results without the feature-gated image writers. Using
`default-features = false` without `write` excludes those image writers;
ordinary FAT and exFAT file mutation remains available. Runtime
policies such as read-only mounts, caches, read-ahead, code pages and case
folding are values passed at runtime, never features. The
[principles and features](https://github.com/hxyulin/hadris/blob/main/docs/v3-api-design.md#features)
document has the full per-crate feature table.

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
`hadris::fat::async_`. Block-format
drivers use one async type for local and Send devices. Implement
`hadris::storage::async_::BlockDevice` once; `SendBlockDevice` is derived when
both the device and its operation state are Send. A Send device alone does not
guarantee Send operation futures.

Use `hadris::fs::local::{FileSystem, Volume}` for local callers and
`hadris::fs::async_::{FileSystem, Volume}` for generic callers requiring Send
futures. The `async` feature enables both filesystem tiers. Storage has no
`local` namespace; byte-stream traits in `hadris-io` have separate `local`
and Send `async_` contracts, and CPIO retains its Send stream API.
Device errors remain `Send + Sync + 'static` in every mode.

The sync and async APIs retain mode-specific differences: the tree
functions of `hadris-fs::host` (`file`, `read_tree`, `write_tree`) use
blocking `std::fs` and are sync-only, while its mount and build defaults and
`hadris-storage::host::FileDevice` need only `std`. A sync `Volume` needs `std`; async
volumes need `alloc` and pointer-sized atomics. Local volumes do not expose
lazy `read_tree`, whose content sources still require Send and Sync.

## Format capability matrix

| Crate | Formats or role | Read | Write/create | Sync | Async | Minimum for reading | Stability |
|---|---|---:|---:|---:|---:|---|---|
| `hadris-fat` | FAT12/16/32 | Yes | Yes | Yes | Yes | `alloc` (checking is allocation-free) | Stable |
| `hadris-fat` `exfat` | exFAT, including TexFAT volumes with two FATs | Yes | Yes | Yes | Yes | `alloc` (checking is allocation-free) | Stable |
| `hadris-part` | MBR (with logical partitions), GPT, hybrid MBR | Yes | Yes | Yes | Yes | Allocation-free (`scan`, `open`) | Stable |
| `hadris-iso` | ISO 9660, Joliet, Rock Ridge, El Torito | Yes | Yes | Yes | Yes | Allocation-free (writing and sessions need `write`) | Stable |
| `hadris-udf` | UDF 1.02 to 2.01, type 1 partitions; ISO 9660 and UDF bridge images | Yes | Yes | Yes | Yes | Allocation-free (writing needs `write`, bridge images `bridge`) | Stable |
| `hadris-cpio` | CPIO newc, CRC and odc; old binary read | Yes | Yes | Yes | Yes | Allocation-free (writing needs `write`) | Stable |
| `hadris-ntfs` | NTFS | Yes | No | Yes | Yes | Allocation-free | Preview |
| `hadris-apfs` | APFS containers and volumes | Yes | No | Yes | Yes | `alloc` | Preview |
| `hadris` `detect` | Detection of every format above, opening FAT, exFAT, ISO 9660 and UDF as `AnyFs`, and single-volume APFS with `unstable-apfs` | Yes | N/A | Yes | Yes | Allocation-free detection; `open` needs `alloc` | Stable |

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
always available, and `write` adds only `format` and the tree writer.

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
features = ["alloc", "sync", "write", "cpio"]
```

`hadris-cpio` has no `read` feature: the reader is always compiled, and
`write` adds the writer.

## Feature selection rules

- Select exactly the formats and capabilities the application uses.
- Select at least one I/O mode for APIs that access storage.
- Add `alloc` only when the chosen API returns or stores owned data.
- Add `write` when the application creates images or formats volumes.
- Use the umbrella with defaults disabled for one dependency and one format; use leaf crates for direct ownership of their versions.
- Treat `hadris-ntfs`, `hadris-apfs`, `unstable-ntfs` and `unstable-apfs`
  as previews outside semver. The crates carry 3.x versions but may break
  in a minor release; depend on them with `"~3.0"`, and pin `hadris` when
  enabling an `unstable-*` feature. See [stability](../stability.md).
- `tracing` adds spans without selecting `std`, so `no_std` builds can trace
  when they have a global allocator, which the `tracing` crate needs.

The workspace CI checks representative allocation-free, `alloc`, `std`, sync,
async, and combined-mode tiers for every stable format crate.
