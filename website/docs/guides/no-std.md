---
title: no_std and embedded use
---

# Use Hadris without the standard library

Disable default features, then select the platform, I/O mode, and capabilities
that the target needs:

```toml
[dependencies]
hadris-fat = {
  version = "2.0.0-rc.1",
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
