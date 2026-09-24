# Hadris Common

Shared types and utilities used by Hadris filesystem crates.

## Overview

This is an internal support crate for the other Hadris crates and is not meant
for direct use. Its API can change in any release; depend on `hadris` or a
format crate instead.

It provides endian-aware integer types, which `hadris-fat` uses for its
on-disk layouts. It needs neither `std` nor an allocator.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `bytemuck` | `Pod` and `Zeroable` for the number and endian types; adds impls only | Yes |

## Usage

### Endian Types

```rust
use hadris_common::types::endian::LittleEndian;
use hadris_common::types::number::U32;

let value = U32::<LittleEndian>::new(0x12345678);
assert_eq!(value.get(), 0x12345678);
```

### Without `bytemuck`

```toml
[dependencies]
hadris-common = { version = "2.4.0", default-features = false }
```

## Documentation

- [Feature and capability guide](https://hxyulin.github.io/hadris/concepts/features)
- [API reference](https://docs.rs/hadris-common)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
