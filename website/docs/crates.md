---
title: Choosing a crate
---

# Choosing a crate

Start with the narrowest crate that owns the format or layer you need. Add the
`hadris` umbrella when the application must detect formats or work across
several storage categories.

![Hadris architecture: applications use the umbrella crate over block, optical, and archive formats backed by shared I/O, paths, and storage](/img/architecture.svg)

## Quick decision table

| Need | Start with | Why |
|---|---|---|
| FAT12/16/32 or exFAT filesystem access | [`hadris-fat`](https://docs.rs/hadris-fat) | `FatFs` and `ExFatFs`, including formatting, checking and mutation |
| Read-only NTFS access (preview) | [`hadris-ntfs`](https://docs.rs/hadris-ntfs) | Allocation-free reader; its native API is a preview |
| FAT or exFAT on-disk structures without a driver | [`hadris-fat-raw`](https://docs.rs/hadris-fat-raw) | Layouts and I/O-free codecs that `hadris-fat` is built on |
| MBR or GPT partition tables | [`hadris-part`](https://docs.rs/hadris-part) | Concrete partition parsing and writing |
| ISO 9660 images | [`hadris-iso`](https://docs.rs/hadris-iso) | ISO, Joliet, Rock Ridge, and El Torito APIs |
| UDF images and hybrid ISO/UDF authoring | [`hadris-udf`](https://docs.rs/hadris-udf) | UDF descriptors, reading, image creation, and bridge images sharing file data with ISO 9660 |
| CPIO newc archives or initramfs | [`hadris-cpio`](https://docs.rs/hadris-cpio) | Streaming CPIO reader (newc, CRC, odc, binary) and writer |
| Detecting and opening unknown images | [`hadris`](https://docs.rs/hadris) | `detect` lists every format a device holds; `open` mounts the first filesystem as an `AnyFs` |
| Several categories through one dependency | [`hadris`](https://docs.rs/hadris) | Re-exports every crate at a flat path, one feature per format |

## Leaf crates

Leaf crates own a concrete format. They expose the richest API, produce the
smallest dependency graph, and are normally the right choice when the input
format is known in advance.

Examples include `hadris-fat`, `hadris-part`, `hadris-iso`, `hadris-udf`, and
`hadris-cpio`.

```toml
[dependencies]
hadris-fat = "3.0.0-rc.1"
```

## The umbrella crate

Use `hadris` when an application spans multiple categories, must detect and
open unknown images, or benefits from a single dependency declaration.

```toml
[dependencies]
hadris = {
  version = "3.0.0-rc.1",
  default-features = false,
  features = ["std", "sync", "detect", "part"]
}
```

The umbrella always re-exports `hadris::io`, `hadris::storage` and
`hadris::fs`, and each format at a flat path (`hadris::fat`, `hadris::iso`)
behind a feature of the same name. The `detect` feature adds
`hadris::{sync, r#async}::{detect, open, AnyFs}` and `hadris::host::open`
with the FAT, ISO 9660, UDF and cpio crates: `detect` lists every format a
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
