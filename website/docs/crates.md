---
title: Choosing a crate
---

# Choosing a crate

| Need | Start with |
|---|---|
| FAT12/16/32 filesystem access | `hadris-fat` |
| MBR or GPT partition tables | `hadris-part` |
| Block-format detection and partition views | `hadris-block` |
| ISO 9660 images | `hadris-iso` |
| UDF images | `hadris-udf` |
| ISO/UDF detection and opening | `hadris-optical` |
| Hybrid ISO/UDF authoring | `hadris-cd` |
| CPIO newc archives or initramfs | `hadris-cpio` |
| Several categories through one dependency | `hadris` |

Leaf crates expose the complete format-specific API. Category facades add
detection and opening without hiding the concrete types. The `hadris` umbrella
re-exports those facades for applications spanning several storage categories.

Shared building blocks are also published independently:
`hadris-io`, `hadris-storage`, `hadris-path`, `hadris-fixed`, and
`hadris-common`.
