# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Hadris is a Rust workspace containing filesystem and disk utility implementations. The project emphasizes no-std compatibility, dual sync/async support, and comprehensive extension support.

## Build Commands

```bash
# Build entire workspace
cargo build --workspace

# Build specific crate
cargo build -p hadris-iso
cargo build -p hadris-fat

# Build with specific features
cargo check --workspace --no-default-features --features "std"
cargo check --workspace --no-default-features --features "std,write"

# Build for no-std environment
cargo build -p hadris-fat --no-default-features --features "read"
```

## Testing

```bash
# Run all workspace tests
cargo test --workspace

# Run tests for specific crate
cargo test -p hadris-fat
cargo test -p hadris-iso

# Run a single test by name
cargo test read_bs

# Run with output visible
cargo test -- --nocapture
```

### No-std verification

Default features include `std`, so `cargo check` and `cargo test` do NOT exercise the no-std code path. After any change to I/O types, error handling, or feature-gated code, verify no-std compilation:

```bash
cargo check -p hadris-iso --no-default-features --features "read,sync"
cargo check -p hadris-fat --no-default-features --features "read,sync"
cargo check -p hadris-cpio --no-default-features --features "read,sync"
cargo check -p hadris-udf --no-default-features --features "read,sync"
cargo check -p hadris-part --no-default-features --features "read,sync"
```

Note: `hadris-io` provides a minimal `Error` type in no-std mode (no message storage). The `std::io::Error` API surface is not fully mirrored — if you use a std-only method like `Error::other()`, add a matching method to `crates/hadris-io/src/error.rs`.

## Workspace Structure

```
crates/
├── hadris-io/       # No-std I/O abstraction (Read, Write, Seek traits)
├── hadris-macros/   # Proc macros for dual sync/async code generation
├── hadris-common/   # Shared types: CRC, endian types, UTF-16 strings
├── hadris-part/     # Partition tables: MBR, GPT, Hybrid MBR
├── hadris-iso/      # ISO 9660: Joliet, El-Torito, SUSP/RRIP (Rock Ridge)
├── hadris-fat/      # FAT12/16/32 with LFN, caching, analysis tools
├── hadris-udf/      # UDF (Universal Disk Format) for DVD/Blu-ray
├── hadris-cpio/     # CPIO archive format (newc/SVR4) for initramfs
├── hadris-cd/       # Hybrid ISO+UDF optical disc image creation
├── hadris/          # Meta-crate re-exporting filesystem implementations
├── hadris-iso-cli/  # CLI for ISO operations
├── hadris-fat-cli/  # CLI for FAT operations (fatutil)
├── hadris-cpio-cli/ # CLI for CPIO operations (cpioutil)
└── hadris-cli/      # General CLI (WIP)
```

## Key Crate Features

All library crates support `sync` and `async` feature flags for dual sync/async code generation via `hadris-macros`. The `sync` flag is enabled by default when `std` is active.

**hadris-io:**
- `std` (default) - Standard library support
- `sync` (default) - Synchronous I/O traits
- `async` - Asynchronous I/O traits

**hadris-common:**
- `std` - Standard library (includes CRC, chrono, rand)
- `alloc` - Heap allocation without full std
- `bytemuck` - Zero-copy serialization
- `optical` - Optical media types (SessionInfo, OpticalMetadataWriter)

**hadris-part:**
- `std` (default) - Standard library support
- `alloc` - Heap allocation for Vec-based APIs
- `read` - Reading partition tables
- `write` - Writing partition tables (requires `alloc`, `read`)

**hadris-fat:**
- `std` (default) - Standard library support
- `alloc` - Heap allocation without full std
- `read` - Read operations
- `write` - Write operations (requires `alloc`, `read`)
- `lfn` - Long filename (VFAT) support
- `cache` - FAT sector caching for performance
- `tool` - Analysis and verification utilities
- `exfat` - ExFAT support (WIP)

**hadris-iso:**
- `std` (default) - Standard library support
- `alloc` - Heap allocation without full std
- `read` - Read operations (no-std compatible)
- `write` - Write/format ISO images (requires `std`)
- `joliet` - UTF-16 filename support

**hadris-udf:**
- `std` (default) - Standard library support
- `read` - Read operations (no-std compatible)
- `write` - Write operations (requires `std`)

**hadris-cpio:**
- `std` (default) - Standard library support
- `alloc` - Heap allocation without full std
- `read` - Read operations
- `write` - Write operations (requires `alloc`, `read`)

**hadris-cd:**
- `std` (default) - Standard library support (combines hadris-iso + hadris-udf)

## Architecture Notes

**Dependency flow:** `hadris-io` -> `hadris-common` -> `hadris-{part,fat,iso,udf,cpio}` -> `hadris-cd` -> `hadris`

**Dual sync/async:** `hadris-macros` provides a `strip_async!` proc macro. Each crate defines `io_transform!`, `sync_only!`, and `async_only!` macros in its sync/async modules. Shared source files are included in both modules via `#[path]` attributes. Doc comments must go *inside* macro invocations to appear in generated docs.

**I/O abstraction:** `hadris-io` provides `Read`, `Write`, `Seek` traits that work in no-std environments. All crates use these instead of `std::io` directly. In no-std mode, a minimal `Error` type replaces `std::io::Error`.

**Sector-based I/O:** ISO and FAT implementations use `SectorCursor<DATA>` wrappers for sector-aligned operations.

**ISO Extensions:**
- Joliet: UTF-16 filenames for Windows compatibility
- El-Torito: Bootable CD/DVD support (BIOS and UEFI)
- SUSP/RRIP: Rock Ridge for POSIX attributes and long filenames

**Partition Support:**
- MBR: Legacy BIOS partition tables (4 primary partitions)
- GPT: Modern UEFI partition tables (128+ partitions with GUIDs)
- Hybrid MBR: Combined MBR+GPT for dual BIOS/UEFI boot

## Edition and Rust Version

- Rust Edition: 2024
- Requires Rust 1.85+
