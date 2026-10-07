---
slug: /
title: Hadris
hide_title: true
---

# The Rust storage stack

Hadris is a collection of pure Rust libraries for block devices, partition
tables, FAT and exFAT filesystems, ISO 9660, UDF, CPIO archives, and disk
images, plus read-only NTFS and APFS readers in preview.

It works across desktop applications, bootloaders, kernels, firmware, and
embedded systems, with explicit `std`, `alloc`, allocation-free, synchronous,
and asynchronous feature tiers.

![Hadris architecture: applications use the umbrella crate over block, optical, and archive formats backed by shared I/O, block devices, and filesystem APIs](/img/architecture-v3.svg)

[Get started](./getting-started.md), [choose a crate](./crates.md), or jump
directly to the [use-case guides](./guides/index.md).

:::note Stability

These pages describe the `3.0.0-rc.2` API. The
[installation recipe](./getting-started.md) selects that prerelease explicitly
for registry installation and explains source checkouts before publication.
V2 `2.5.0` is the published stable library version and remains in the version
selector. The [migration guide](./migration.md) covers the upgrade from 2.4/2.5. In 3.0, exFAT is stable in `hadris-fat`, and the
`hadris-ntfs` reader stays a preview outside the stability promise. See
[Stability and compatibility](./stability.md).

:::
