# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Hadris is a Rust workspace containing filesystem and disk utility implementations. The project emphasizes no-std compatibility, configurable strictness levels, and comprehensive extension support.

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

Note: CI only runs `cargo check`, not full tests. Run tests locally before pushing.

## Workspace Structure

```
crates/
├── hadris-io/       # No-std I/O abstraction (Read, Write, Seek traits)
├── hadris-common/   # Shared types: CRC, endian types, UTF-16 strings
├── hadris-part/     # Partition tables: MBR, GPT, Hybrid MBR
├── hadris-iso/      # ISO 9660: Joliet, El-Torito, SUSP/RRIP (Rock Ridge)
├── hadris-fat/      # FAT12/16/32 with LFN, caching, analysis tools
├── hadris/          # Meta-crate re-exporting filesystem implementations
├── hadris-iso-cli/  # CLI for ISO operations
├── hadris-fat-cli/  # CLI for FAT operations
└── hadris-cli/      # General CLI (WIP)
```

## Key Crate Features

**hadris-io:**
- `std` (default) - Standard library support

**hadris-common:**
- `std` - Standard library (includes CRC, chrono, rand)
- `alloc` - Heap allocation without full std
- `bytemuck` - Zero-copy serialization

**hadris-part:**
- `std` (default) - Standard library support
- `alloc` - Heap allocation for Vec-based APIs
- `read` - Reading partition tables
- `write` - Writing partition tables (requires `alloc`)

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

## Architecture Notes

**Dependency flow:** `hadris-io` -> `hadris-common` -> `hadris-{part,fat,iso}` -> `hadris`

**I/O abstraction:** `hadris-io` provides `Read`, `Write`, `Seek` traits that work in no-std environments. All crates use these instead of `std::io` directly.

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
