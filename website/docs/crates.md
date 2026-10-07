---
title: Choosing a crate
---

# Choosing a crate

Start with `hadris` for one dependency that exposes the native format APIs,
block devices and shared filesystem types. Disable defaults and select only
the needed formats. Choose individual crates when independent versioning or
an explicit dependency on one layer is useful. The
[getting-started recipe](./getting-started.md) targets RC2 and explains how to use it before and after publication.

![Hadris architecture: applications use the umbrella crate over block, optical, and archive formats backed by shared I/O, block devices, and filesystem APIs](/img/architecture-v3.svg)

## Quick decision table

| Need | Start with | Why |
|---|---|---|
| FAT12/16/32 or exFAT filesystem access | [`hadris-fat`](https://github.com/hxyulin/hadris/tree/main/crates/block/hadris-fat) | `FatFs` and `ExFatFs`, including formatting, checking and mutation |
| Read-only NTFS access (preview) | [`hadris-ntfs`](https://github.com/hxyulin/hadris/tree/main/crates/block/hadris-ntfs) | Allocation-free reader; its native API is a preview |
| FAT or exFAT on-disk structures without a driver | [`hadris-fat-raw`](https://github.com/hxyulin/hadris/tree/main/crates/block/hadris-fat-raw) | Layouts and I/O-free codecs that `hadris-fat` is built on |
| CPIO on-disk structures without a driver | [`hadris-cpio-raw`](https://github.com/hxyulin/hadris/tree/main/crates/archive/hadris-cpio-raw) | Allocation-free layouts and I/O-free codecs; first publication pending |
| UDF on-disk structures without a driver | [`hadris-udf-raw`](https://github.com/hxyulin/hadris/tree/main/crates/optical/hadris-udf-raw) | Allocation-free layouts and I/O-free codecs; first publication pending |
| ISO on-disk structures without a driver | [`hadris-iso-raw`](https://github.com/hxyulin/hadris/tree/main/crates/optical/hadris-iso-raw) | Allocation-free layouts and I/O-free codecs; first publication pending |
| MBR or GPT partition tables | [`hadris-part`](https://github.com/hxyulin/hadris/tree/main/crates/block/hadris-part) | Concrete partition parsing and writing |
| ISO 9660 images | [`hadris-iso`](https://github.com/hxyulin/hadris/tree/main/crates/optical/hadris-iso) | ISO, Joliet, Rock Ridge, and El Torito APIs |
| UDF images and hybrid ISO/UDF authoring | [`hadris-udf`](https://github.com/hxyulin/hadris/tree/main/crates/optical/hadris-udf) | UDF descriptors, reading, image creation, and bridge images sharing file data with ISO 9660 |
| CPIO newc archives or initramfs | [`hadris-cpio`](https://github.com/hxyulin/hadris/tree/main/crates/archive/hadris-cpio) | Streaming CPIO reader (newc, CRC, odc, binary) and writer |
| Detecting and opening unknown images | [`hadris`](https://github.com/hxyulin/hadris/tree/main/crates/core/hadris) | `detect` lists every format a device holds; `open` mounts the first filesystem as an `AnyFs` |
| Several categories through one dependency | [`hadris`](https://github.com/hxyulin/hadris/tree/main/crates/core/hadris) | Re-exports every crate at a flat path, one feature per format |

## Leaf crates

Leaf crates own a concrete format and can be versioned independently. A
complete application may also depend directly on `hadris-fs`, `hadris-storage`
or `hadris-io`. The umbrella re-exports the same native API and can select a
single format; choosing it does not hide capabilities or require detection.

Examples include `hadris-fat`, `hadris-part`, `hadris-iso`, `hadris-udf`, and
`hadris-cpio`.

```toml
[dependencies]
hadris-fat = { version = "3.0.0-rc.2" }
```

## The umbrella crate

Use `hadris` when an application spans multiple categories, must detect and
open unknown images, or benefits from a single dependency declaration.

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["std", "sync", "detect", "part"]
```

The umbrella always re-exports `hadris::io`, `hadris::storage` and
`hadris::fs`, and each format at a flat path (`hadris::fat`, `hadris::iso`)
behind a feature of the same name. The `detect` feature adds
`hadris::{sync, async_}::{detect, open, AnyFs}` and `hadris::host::open`
with the FAT, ISO 9660, UDF, CPIO and APFS reader crates: `detect` lists every format a
device holds, including partition tables, archives and NTFS, and `open`
mounts the first filesystem as an `AnyFs`, which implements the `hadris-fs`
`FileSystem` trait and reaches each driver's native API by `match`. Select
format, platform and I/O features explicitly.

## Foundation crates

Most applications consume these indirectly, but they are useful integration
points for kernels, firmware, and other storage libraries:

| Crate | Role |
|---|---|
| `hadris-io` | Sync and async byte-stream traits and adapters |
| `hadris-storage` | Block devices, geometry, slices, and a block cache |
| `hadris-fs` | Shared vocabulary, the `FileSystem` trait, `MountOptions`, `Volume` and its handles, `copy_tree`, and the writer input tree |
| `hadris-common` | Internal endian integers for on-disk layouts; not for direct use |
| `hadris-macros` | Internal dual sync/async code-generation support |

## Preview APIs

The `hadris-ntfs` crate is outside the stable API promise. It is appropriate
for evaluation and compatibility testing, but callers should expect changes to
its native API.

The umbrella's `detect` lists NTFS volumes in every build, and `open` refuses
them with `NotRecognized` until NTFS is stable; its `unstable-ntfs` feature
adds the `hadris::ntfs` re-export for the driver's native API. exFAT is stable in 3.0: `ExFatFs`
lives in `hadris_fat::exfat` in every build, and the 2.x `unstable-exfat`
feature is gone.

## Next steps

- [Select platform, I/O, and capability features](./concepts/features.md)
- [Understand the storage and I/O layers](./concepts/storage-model.md)
- [Follow a task-oriented guide](./guides/index.md)

## APFS preview

`hadris-apfs` provides an experimental, read-only `ApfsFs` driver in sync and
async modes over V3 storage devices. It implements `FileSystem`, so generic
`Volume` file access, walks and extraction work on APFS. Mounting needs `alloc`.
The native `Container` API exposes container and volume inspection.

The umbrella's `detect` feature recognizes and mounts single-volume APFS
containers. `unstable-apfs` exposes native inspection and selection; the unified
CLI provides `hadris apfs` commands. A default mount requires one volume; choose explicitly by index,
object ID, UUID or name when a container holds several. APFS remains read-only;
compressed files and snapshot views are unsupported. Optional software
FileVault unlocking uses `apfs-encryption`; Apple-silicon hardware FileVault
remains unsupported.
