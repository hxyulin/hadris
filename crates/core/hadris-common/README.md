# Hadris Common

Endian-aware integer types for the on-disk layouts of the Hadris raw crates.

## Overview

This is an internal support crate. It keeps a 3.x version but is exempt from
semver: a 3.y minor release may break it, and patch releases stay
compatible, so depend on it with `"~3.0"`. The public fields of the
`hadris-fat-raw` layouts use its types, so a breaking change here is a
breaking change of `hadris-fat-raw`. No stable 3.x crate exposes it. Depend on
`hadris` or a format crate unless you read raw layouts; `hadris-fat-raw`
users rarely need to name these types, since `new`, `get` and `set` are
inherent methods.

It needs neither `std` nor an allocator.

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `bytemuck` | `Pod` and `Zeroable` for the number and endian types; adds impls only | Yes |

## Usage

```rust
use hadris_common::types::endian::LittleEndian;
use hadris_common::types::number::U32;

let value = U32::<LittleEndian>::new(0x12345678);
assert_eq!(value.get(), 0x12345678);
```

## Documentation

- [Feature and capability guide](https://hxyulin.github.io/hadris/concepts/features)
- [API reference](https://docs.rs/hadris-common/3.0.0-rc.2)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
