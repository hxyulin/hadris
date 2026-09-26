---
slug: /
title: Hadris
hide_title: true
---

# The Rust storage stack

Hadris is a collection of pure Rust libraries for block devices, partition
tables, FAT and exFAT filesystems, ISO 9660, UDF, CPIO archives, and disk
images, plus a read-only NTFS reader in preview.

It works across desktop applications, bootloaders, kernels, firmware, and
embedded systems, with explicit `std`, `alloc`, allocation-free, synchronous,
and asynchronous feature tiers.

![Hadris architecture: applications use the umbrella crate over block, optical, and archive formats backed by shared I/O, paths, and storage](/img/architecture.svg)

[Get started](./getting-started.md), [choose a crate](./crates.md), or jump
directly to the [use-case guides](./guides/index.md).

:::note Stability

These pages describe the 3.0 API, whose first release candidate is
`3.0.0-rc.1`; `2.4.0` is the current stable release. The
[migration guide](https://github.com/hxyulin/hadris/blob/next/docs/hadris-3.0.0-migration.md)
covers the upgrade from 2.4. In 3.0, exFAT is stable in `hadris-fat`, and the
`hadris-ntfs` reader stays a preview outside the stability promise. See
[Stability and compatibility](./stability.md).

:::
