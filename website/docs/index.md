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

![Hadris architecture: applications use the umbrella crate over block, optical, and archive formats backed by shared I/O, paths, and storage](/img/architecture.svg)

[Get started](./getting-started.md), [choose a crate](./crates.md), or jump
directly to the [use-case guides](./guides/index.md).

:::note Stability

`2.1.0` is the current stable V2 release under Semantic Versioning. The
`unstable-exfat` feature and experimental `hadris-ntfs` crate remain outside
that stability promise.

:::
