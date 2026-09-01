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
| FAT12/16/32 filesystem access | [`hadris-fat`](https://docs.rs/hadris-fat) | Complete FAT API, including formatting and mutation |
| Experimental read-only NTFS access | [`hadris-ntfs`](https://docs.rs/hadris-ntfs) | NTFS remains a separate experimental leaf crate |
| MBR or GPT partition tables | [`hadris-part`](https://docs.rs/hadris-part) | Concrete partition parsing and writing |
| Block-format detection and partition views | [`hadris-block`](https://docs.rs/hadris-block) | Combines storage, partitions, and FAT without erasing concrete types |
| ISO 9660 images | [`hadris-iso`](https://docs.rs/hadris-iso) | ISO, Joliet, Rock Ridge, and El Torito APIs |
| UDF images | [`hadris-udf`](https://docs.rs/hadris-udf) | UDF descriptors, reading, and image creation |
| ISO/UDF detection and opening | [`hadris-optical`](https://docs.rs/hadris-optical) | Detects bridge images and applies an explicit open policy |
| Hybrid ISO/UDF authoring | [`hadris-cd`](https://docs.rs/hadris-cd) | Builds images sharing file data between both filesystems |
| CPIO newc archives or initramfs | [`hadris-cpio`](https://docs.rs/hadris-cpio) | Complete CPIO reader and writer |
| Several categories through one dependency | [`hadris`](https://docs.rs/hadris) | Re-exports selected category facades |

## Leaf crates

Leaf crates own a concrete format. They expose the richest API, produce the
smallest dependency graph, and are normally the right choice when the input
format is known in advance.

Examples include `hadris-fat`, `hadris-part`, `hadris-iso`, `hadris-udf`, and
`hadris-cpio`.

```toml
[dependencies]
hadris-fat = "2.3.0"
```

## Category facades

Category facades combine related layers and add detection or opening policy:

- `hadris-block` combines storage traits, partitions, FAT, and block detection.
- `hadris-optical` combines ISO, UDF, bridge detection, and hybrid authoring.
- `hadris-archive` provides a common feature surface for sequential archives.

Facades preserve the underlying leaf types. They do not force unrelated
formats behind one generic filesystem interface.

## The umbrella crate

Use `hadris` when an application spans multiple categories and benefits from a
single dependency declaration.

```toml
[dependencies]
hadris = {
  version = "2.3.0",
  default-features = false,
  features = ["std", "sync", "read", "block", "optical"]
}
```

Using the umbrella does not enable every format automatically. Select category,
platform, I/O, and capability features explicitly.

## Foundation crates

Most applications consume these indirectly, but they are useful integration
points for kernels, firmware, and other storage libraries:

| Crate | Role |
|---|---|
| `hadris-io` | Sync and async byte-stream traits and adapters |
| `hadris-storage` | Logical-block geometry, device traits, and bounded views |
| `hadris-path` | Allocation-free lexical paths for virtual filesystems |
| `hadris-fixed` | Fixed-capacity byte, UTF-8, and UTF-16 values |
| `hadris-common` | Shared endian and disk-format primitives |
| `hadris-macros` | Internal dual sync/async code-generation support |

## Experimental APIs

The `unstable-exfat` feature and `hadris-ntfs` crate are outside the stable V2
API promise. They are appropriate for evaluation and compatibility testing,
but callers should expect API and behavior changes.

NTFS is intentionally not opened by `hadris-block` or re-exported by the
`hadris` umbrella. exFAT remains an opt-in feature of `hadris-fat`.

## Next steps

- [Select platform, I/O, and capability features](./concepts/features.md)
- [Understand the storage and I/O layers](./concepts/storage-model.md)
- [Follow a task-oriented guide](./guides/index.md)
