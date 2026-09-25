---
title: Getting started
---

# Getting started

Choose the narrowest crate that covers your application:

```toml
[dependencies]
# A single filesystem:
hadris-fat = "2.4.0"

# Read-only NTFS (preview):
hadris-ntfs = "2.4.0"

# Or several storage categories:
hadris = { version = "2.4.0", features = ["block", "optical"] }
```

Hadris separates platform support, I/O mode, and capabilities. For a
freestanding FAT or exFAT consumer, which needs an allocator for the
driver's node table but not `std`:

```toml
[dependencies]
hadris-fat = {
  version = "2.4.0",
  default-features = false,
  features = ["alloc", "sync"]
}
```

For hosted applications, the default features provide the synchronous API
with `std`. Every I/O type is named through its mode module
(`hadris_fat::sync::FatFs`, `hadris_iso::r#async::IsoFs`), so the same code
reads the same way whichever modes are enabled.

Every filesystem driver implements the `hadris-fs` `FileSystem` trait, which
works on node ids. Wrap a driver in `hadris_fs::sync::Volume` for paths and
methods named after `std::fs` (`read_dir`, `open`, `metadata`,
`create_dir_all`) and to share it between handles and threads.

The NTFS reader is a preview and is outside the stability promise. Its crate
README documents the supported read-only scope and known gaps.

For the complete support table and feature recipes, see
[Features and capabilities](./concepts/features.md).

## Next steps

- [Choose a crate](./crates.md)
- [Understand the storage and I/O model](./concepts/storage-model.md)
- [Detect and open an unknown image](./guides/detect-open-images.md)
- [Read a FAT image](./guides/read-fat-image.md)
- [Inspect a partition table](./guides/read-partition-table.md)
- [Open FAT inside a partition](./guides/open-partitioned-fat.md)
- [Read an ISO](./guides/read-iso.md)
- [Read UDF](./guides/read-udf.md)
- [Create an ISO](./creation/iso.md)
- [Build a CPIO initramfs](./guides/build-initramfs.md)
- [Use asynchronous I/O](./guides/async-io.md)
- [Configure a `no_std` target](./guides/no-std.md)
