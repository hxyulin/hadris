# Hadris Common

Shared types and utilities used by Hadris filesystem crates.

## Overview

This crate provides common functionality needed across the Hadris workspace, including endian-aware types, CRC calculations, and string utilities.

## Features

- **Endian Types** - Little-endian and big-endian wrappers for integers
- **CRC Calculations** - CRC32 and other checksum algorithms
- **UTF-16 Strings** - Utilities for working with UTF-16 encoded names
- **No-std Compatible** - Works without the standard library

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Standard library support (includes CRC, chrono, rand) | Yes |
| `alloc` | Heap allocation without full std | - |
| `bytemuck` | Zero-copy serialization support | Yes |

## Usage

### Endian Types

```rust
use hadris_common::types::endian::{LittleEndian, U16, U32};

// Create little-endian values
let value: U32<LittleEndian> = U32::new(0x12345678);
assert_eq!(value.get(), 0x12345678);
```

### For No-std Environments

```toml
[dependencies]
hadris-common = { version = "0.2", default-features = false, features = ["alloc"] }
```

### Minimal (No Heap)

```toml
[dependencies]
hadris-common = { version = "0.2", default-features = false }
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
