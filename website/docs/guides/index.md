---
title: Use cases
---

# Use cases

These guides start with an application task and identify the narrowest crate
and feature set needed to complete it.

## Discover and open storage

- [Detect and open unknown images](./detect-open-images.md)
- [Inspect an MBR or GPT disk image](./read-partition-table.md)
- [Open FAT inside a partition](./open-partitioned-fat.md)

## Read and modify formats

- [Read files from a FAT image](./read-fat-image.md)
- [Modify a FAT filesystem safely](./modify-fat.md)
- [Read ISO 9660 images](./read-iso.md)
- [Read and extract UDF](./read-udf.md)
- [Read and create CPIO archives](./cpio-archives.md)
- [Build a CPIO initramfs](./build-initramfs.md)

## Integrate with a platform

- [Use Hadris without the standard library](./no-std.md)
- [Use asynchronous I/O](./async-io.md)
- [Adapt a custom device or firmware reader](./custom-io.md)

## Qualify output

- [Validate generated images](./validate-images.md)

Runnable application packages for FAT, partition, optical detection, and CPIO
workflows live in the repository's
[`examples/` directory](https://github.com/hxyulin/hadris/tree/main/examples).
Runnable ISO examples live in
[`crates/optical/hadris-iso/examples`](https://github.com/hxyulin/hadris/tree/main/crates/optical/hadris-iso/examples).
