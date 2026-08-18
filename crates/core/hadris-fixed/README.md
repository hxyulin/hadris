# hadris-fixed

Fixed-capacity byte, UTF-8, and UTF-16 values for `no_std` applications.
Storage is inline, so the core API needs neither a global allocator nor the
standard library.

Use this crate for bounded on-disk names, labels, and protocol fields. Raw
bytes and validated text are distinct types, so safe text APIs cannot be
constructed from invalid UTF-8.

## Types

| Type | Purpose |
|---|---|
| `FixedBytes<N>` | Arbitrary initialized bytes with capacity `N` |
| `FixedStr<N>` | Valid UTF-8 with a fixed byte capacity |
| `FixedUtf16Le<N>` | Little-endian UTF-16 code units |
| `FixedUtf16Be<N>` | Big-endian UTF-16 code units |

## Example

```rust
use hadris_fixed::{FixedBytes, FixedStr};

let mut bytes = FixedBytes::<8>::new();
bytes.try_push_slice(b"FAT32")?;
assert_eq!(bytes.as_bytes(), b"FAT32");

let mut name = FixedStr::<16>::new();
name.try_push_str("boot")?;
name.try_push('.')?;
name.try_push_str("cfg")?;
assert_eq!(name.as_str(), "boot.cfg");
# Ok::<(), hadris_fixed::CapacityError>(())
```

The fallible `try_*` methods return `CapacityError`; convenience methods
without `try_` panic when capacity is exceeded.

## Features

| Feature | Default | Purpose |
|---|---:|---|
| `alloc` | No | Conversions and conveniences that return allocated values |
| `bytemuck` | No | Byte-safe trait implementations for compatible fixed types |

## Documentation

- [Feature and capability guide](https://hxyulin.github.io/hadris/concepts/features)
- [API reference](https://docs.rs/hadris-fixed)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
