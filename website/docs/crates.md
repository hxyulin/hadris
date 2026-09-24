---
title: Choosing a crate
---

# Choosing a crate

Start with the narrowest crate that owns the format or layer you need. Add a
facade only when the application must detect formats or work across several
storage categories.

![Hadris architecture: applications use the umbrella crate over block, optical, and archive formats backed by shared I/O, paths, and storage](/img/architecture.svg)

## Quick decision table

| Need | Start with | Why |
|---|---|---|
| FAT12/16/32 or exFAT filesystem access | [`hadris-fat`](https://docs.rs/hadris-fat) | `FatFs` and `ExFatFs`, including formatting, checking and mutation |
| Read-only NTFS access (preview) | [`hadris-ntfs`](https://docs.rs/hadris-ntfs) | Allocation-free reader; its native API is a preview |
| FAT or exFAT on-disk structures without a driver | [`hadris-fat-raw`](https://docs.rs/hadris-fat-raw) | Layouts and I/O-free codecs that `hadris-fat` is built on |
| MBR or GPT partition tables | [`hadris-part`](https://docs.rs/hadris-part) | Concrete partition parsing and writing |
| Block-format detection and opening | [`hadris-block`](https://docs.rs/hadris-block) | Detects FAT, exFAT, NTFS and partition tables and opens the filesystems through one driver |
| ISO 9660 images | [`hadris-iso`](https://docs.rs/hadris-iso) | ISO, Joliet, Rock Ridge, and El Torito APIs |
| UDF images | [`hadris-udf`](https://docs.rs/hadris-udf) | UDF descriptors, reading, and image creation |
| ISO/UDF detection and opening | [`hadris-optical`](https://docs.rs/hadris-optical) | Detects bridge images and opens one filesystem by an explicit policy |
| Hybrid ISO/UDF authoring | [`hadris-cd`](https://docs.rs/hadris-cd) | Builds images sharing file data between both filesystems |
| CPIO newc archives or initramfs | [`hadris-cpio`](https://docs.rs/hadris-cpio) | Streaming CPIO reader (newc, CRC, odc, binary) and writer |
| Several categories through one dependency | [`hadris`](https://docs.rs/hadris) | Re-exports every crate at a flat path, one feature per format |

## Leaf crates

Leaf crates own a concrete format. They expose the richest API, produce the
smallest dependency graph, and are normally the right choice when the input
format is known in advance.

Examples include `hadris-fat`, `hadris-part`, `hadris-iso`, `hadris-udf`, and
`hadris-cpio`.

```toml
[dependencies]
hadris-fat = "2.4.0"
```

## Category facades

Category facades detect a format and open it:

- `hadris-block` detects FAT, NTFS, exFAT and partition tables and opens FAT,
  exFAT and NTFS as one `OpenVolume`.
- `hadris-optical` detects ISO 9660, UDF and bridge images and opens one of
  them as an `OpenOpticalImage`.

Both openers implement the `hadris-fs` `FileSystem` trait by delegating to the
format's driver, and keep that driver reachable for its native API.

## The umbrella crate

Use `hadris` when an application spans multiple categories and benefits from a
single dependency declaration.

```toml
[dependencies]
hadris = {
  version = "2.4.0",
  default-features = false,
  features = ["std", "sync", "block", "optical"]
}
```

The umbrella always re-exports `hadris::io`, `hadris::storage` and
`hadris::fs`, and each format at a flat path (`hadris::fat`, `hadris::iso`)
behind a feature of the same name. Select format, platform and I/O features
explicitly.

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

`hadris-block` opens NTFS through the `FileSystem` trait in every build; its
`unstable-ntfs` feature, like the umbrella's, adds the NTFS re-export and
access to the NTFS driver's native API. exFAT is stable in 3.0: `ExFatFs`
lives in `hadris_fat::exfat` in every build, and the 2.x `unstable-exfat`
feature is gone.

## Next steps

- [Select platform, I/O, and capability features](./concepts/features.md)
- [Understand the storage and I/O layers](./concepts/storage-model.md)
- [Follow a task-oriented guide](./guides/index.md)
