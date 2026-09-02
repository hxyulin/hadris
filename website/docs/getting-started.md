---
title: Getting started
---

# Getting started

Choose the narrowest crate that covers your application:

```toml
[dependencies]
# A single filesystem:
hadris-fat = "2.3.0"

# Experimental read-only NTFS:
hadris-ntfs = "2.3.0"

# Or several storage categories:
hadris = { version = "2.3.0", features = ["block", "optical"] }
```

Hadris separates platform support, I/O mode, and capabilities. For a
freestanding read-only FAT consumer:

```toml
[dependencies]
hadris-fat = {
  version = "2.3.0",
  default-features = false,
  features = ["read", "sync"]
}
```

For hosted applications, default features provide the ergonomic synchronous
configuration. Use explicit `sync` or `async` namespaces in new code when an
application enables both modes.

The NTFS reader is an experimental leaf crate and is outside the V2 stability
freeze. Its crate README documents the supported read-only scope and known
gaps.

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
