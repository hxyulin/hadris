# Hadris Common

Shared types and utilities used by Hadris filesystem crates.

## Overview

This is an internal support crate for the other Hadris crates and is not meant
for direct use. Its API can change in any release; depend on `hadris` or a
format crate instead.

It provides endian-aware types, extents and layout helpers.
Virtual path code uses `hadris_fs::path`.

## Features

- **Endian Types** - Little-endian and big-endian wrappers for integers
- **Extents** - On-disk layout helpers used by ISO and related crates
- **No-std Compatible** - Works without the standard library

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Standard library support; implies `alloc` | Yes |
| `alloc` | Heap allocation without full std | via `std` |
| `bytemuck` | `Pod` and `Zeroable` for the number and endian types; adds impls only | Yes |
| `sync` | Synchronous I/O feature forwarded to `hadris-io` (for dependents) | No |
| `async` | Asynchronous I/O feature forwarded to `hadris-io` | No |

> `sync` / `async` enable the matching `hadris-io` features for crates that depend on `hadris-common`. This crate does **not** re-export `hadris-io` traits at the root.

`std` and the I/O mode are independent. The default feature set enables
`std` and `bytemuck`, but not `sync` or `async`.

## Usage

### Endian Types

```rust
use hadris_common::types::endian::LittleEndian;
use hadris_common::types::number::U32;

let value = U32::<LittleEndian>::new(0x12345678);
assert_eq!(value.get(), 0x12345678);
```

### For No-std Environments

```toml
[dependencies]
hadris-common = { version = "2.4.0", default-features = false, features = ["alloc", "bytemuck"] }
```

### Minimal (No Heap)

```toml
[dependencies]
hadris-common = { version = "2.4.0", default-features = false, features = ["bytemuck"] }
```

## Documentation

- [Feature and capability guide](https://hxyulin.github.io/hadris/concepts/features)
- [API reference](https://docs.rs/hadris-common)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
