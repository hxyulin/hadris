---
title: no_std and embedded use
---

# Use Hadris without the standard library

Disable default features, then select the platform, I/O mode, and capabilities
that the target needs:

```toml
[dependencies]
hadris-fat = {
  version = "2.1.0",
  default-features = false,
  features = ["read", "sync"]
}
```

Add `alloc` for APIs backed by `Vec`, `String`, or owned trees. Add `write`
only when mutation or image creation is required. `std` implies allocation but
does not implicitly select `sync` or `async`.

All storage I/O flows through `hadris-io`, allowing callers to adapt firmware,
kernel, memory, or device-specific readers rather than depending on
`std::io`.

## Choose the narrowest tier

| Need | Features |
|---|---|
| Allocation-free synchronous reader | `read,sync` |
| Allocation-free asynchronous reader | `read,async` |
| Owned names or buffers | Add `alloc` |
| Filesystem mutation | Add `write` and its required platform tier |
| Both I/O modes | Enable `sync,async` and use explicit namespaces |

The exact minimum differs by format. NTFS reading requires `alloc`; ISO and UDF
image creation require `std`. See the complete
[feature and capability matrix](../concepts/features.md).

For integration examples, see [Adapt a custom device](./custom-io.md) and
[Use asynchronous I/O](./async-io.md).
