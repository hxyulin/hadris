---
title: Getting started
---

# Getting started

Choose the narrowest crate that covers your application:

```toml
[dependencies]
# A single filesystem:
hadris-fat = "2.0.0-rc.3"

# Or several storage categories:
hadris = { version = "2.0.0-rc.3", features = ["block", "optical"] }
```

Hadris separates platform support, I/O mode, and capabilities. For a
freestanding read-only FAT consumer:

```toml
[dependencies]
hadris-fat = {
  version = "2.0.0-rc.3",
  default-features = false,
  features = ["read", "sync"]
}
```

For hosted applications, default features provide the ergonomic synchronous
configuration. Use explicit `sync` or `async` namespaces in new code when an
application enables both modes.

## Next steps

- [Choose a crate](./crates.md)
- [Read a FAT image](./guides/read-fat-image.md)
- [Inspect a partition table](./guides/read-partition-table.md)
- [Read or create an ISO](./guides/read-and-create-iso.md)
- [Build a CPIO initramfs](./guides/build-initramfs.md)
- [Configure a `no_std` target](./guides/no-std.md)
