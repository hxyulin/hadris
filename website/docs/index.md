---
slug: /
title: Hadris
hide_title: true
---

# The Rust storage stack

Hadris is a collection of pure Rust libraries for block devices, partition
tables, FAT filesystems, ISO 9660, UDF, CPIO archives, and disk images, plus an
experimental read-only NTFS reader.

It works across desktop applications, bootloaders, kernels, firmware, and
embedded systems, with explicit `std`, `alloc`, allocation-free, synchronous,
and asynchronous feature tiers.

[Get started](./getting-started.md) or jump directly to the
[use-case guides](./guides/index.md).

:::note Stability

`2.0.0` is the stable V2 public API, released under Semantic Versioning. The
`unstable-exfat` feature and experimental `hadris-ntfs` crate remain outside
that stability promise.

:::
