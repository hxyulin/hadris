# Hadris Common

Shared types and utilities used by Hadris filesystem crates.

## Overview

This crate provides common functionality needed across the Hadris workspace,
including endian-aware types, extents, and optional optical-media types. New
fixed-capacity buffers and text should use `hadris-fixed`, and new path code
should use `hadris-path`; the old modules remain as deprecated compatibility
forwarding surfaces.

## Features

- **Endian Types** - Little-endian and big-endian wrappers for integers
- **Extents** - On-disk layout helpers used by ISO and related crates
- **CRC / time / rand** - Available under the `std` feature
- **Optical media** - Session and metadata helpers behind the `optical` feature
- **No-std Compatible** - Works without the standard library

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `std` | Standard library support (CRC, chrono, rand; implies `sync`, `alloc`) | Yes |
| `alloc` | Heap allocation without full std | via `std` |
| `bytemuck` | Zero-copy serialization support | Yes |
| `optical` | Optical media types (`SessionInfo`, metadata writers) | No |
| `sync` | Synchronous I/O feature forwarded to `hadris-io` (for dependents) | via `std` |
| `async` | Asynchronous I/O feature forwarded to `hadris-io` | No |

> `sync` / `async` enable the matching `hadris-io` features for crates that depend on `hadris-common`. This crate does **not** re-export `hadris-io` traits at the root.

## Usage

### Endian Types

```rust
use hadris_common::types::endian::LittleEndian;
use hadris_common::types::number::U32;

let value = U32::<LittleEndian>::new(0x12345678);
assert_eq!(value.get(), 0x12345678);
```

### Boot sector binary

```rust
assert_eq!(hadris_common::BOOT_SECTOR_BIN.len(), 512);
assert_eq!(hadris_common::BOOT_SECTOR_BIN[510], 0x55);
assert_eq!(hadris_common::BOOT_SECTOR_BIN[511], 0xAA);
```

### For No-std Environments

```toml
[dependencies]
hadris-common = { version = "2.0.0-rc.1", default-features = false, features = ["alloc", "bytemuck"] }
```

### Minimal (No Heap)

```toml
[dependencies]
hadris-common = { version = "2.0.0-rc.1", default-features = false, features = ["bytemuck"] }
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
