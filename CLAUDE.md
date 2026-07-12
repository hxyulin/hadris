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

### Quality checks (match CI)

CI promotes warnings to errors via `RUSTFLAGS="-D warnings"` for both the default-feature workspace check and every per-crate feature tier (see the `check` and `check-features` jobs in `.github/workflows/rust.yml`). A warning that only appears under a non-default feature combination — e.g. an unused helper that's gated behind `write` — passes a plain `cargo check --workspace` but fails CI. Always reproduce CI locally with `-D warnings`:

```bash
# Workspace, default features (matches CI `check` job)
RUSTFLAGS="-D warnings" cargo check --workspace

# Per-crate feature tiers (matches CI `check-features` matrix)
RUSTFLAGS="-D warnings" cargo check -p hadris-iso --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-fat --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-cpio --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-udf --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-part --no-default-features --features "read,sync"
```

When fixing a feature-gated dead-code warning, prefer `#[cfg(feature = "<feat>")]` on the item over `#[allow(dead_code)]` so the compiler stays honest about which builds actually use it.

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

Default features include `std`, so `cargo check` and `cargo test` do NOT exercise the no-std code path. After any change to I/O types, error handling, or feature-gated code, verify no-std compilation (with `-D warnings` to match CI — see "Quality checks" above):

```bash
RUSTFLAGS="-D warnings" cargo check -p hadris-iso --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-fat --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-cpio --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-udf --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-part --no-default-features --features "read,sync"
```

Note: `hadris-io` provides a minimal `Error` type in no-std mode (no message storage). The `std::io::Error` API surface is not fully mirrored — if you use a std-only method like `Error::other()`, add a matching method to `crates/core/hadris-io/src/error.rs`.

### Miri (UB detection)

The crates have several `unsafe` blocks (`bytemuck::Pod` reinterpretation, union access in directory entries, the `transmute` in `dir.rs` raw entry parsing). When touching any of these — or anything that converts disk bytes into `&str` — run miri to catch UB regressions before pushing:

```bash
# One-time setup
rustup +nightly component add miri
cargo +nightly miri setup

# Targeted safety tests (fast, ~5s each)
cargo +nightly miri test -p hadris-common --lib
cargo +nightly miri test -p hadris-fat --lib file::lfn_unicode_tests
cargo +nightly miri test -p hadris-iso --lib types::iso_str_safety_tests
```

CI runs the same set on every push (`miri` job in `.github/workflows/rust.yml`). Miri rejects real I/O syscalls and is 10–100× slower than native, so we deliberately scope it to focused unit tests covering historically-unsafe code paths (issues #26 and #28). When you add a new `unsafe` block or fix a soundness bug, add a regression test under one of the existing miri-tested modules so the CI job exercises it.

## Workspace Structure

```
crates/
├── core/            # Umbrella crate, shared types, I/O, and macros
│   ├── hadris/
│   ├── hadris-common/
│   ├── hadris-io/
│   ├── hadris-storage/
│   └── hadris-macros/
├── block/           # Block filesystems and partition tables
│   ├── hadris-fat/
│   └── hadris-part/
├── optical/         # Optical filesystems and disc composition
│   ├── hadris-iso/
│   ├── hadris-udf/
│   └── hadris-cd/
├── archive/         # Sequential archive formats
│   └── hadris-cpio/
└── tools/           # Format CLIs and the experimental general CLI
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

**Dependency flow:** `hadris-io` -> `hadris-{common,storage}` -> `hadris-{part,fat,iso,udf,cpio}` -> `hadris-cd` -> category facades / `hadris`

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
- MSRV: Rust **1.88.0** (`rust-version` in workspace `Cargo.toml`, pinned by `rust-toolchain.toml`)
- CI installs `dtolnay/rust-toolchain@1.88.0` for check/test/clippy/fmt/doc jobs (Miri still uses nightly)

## Fuzzing

Harnesses live under `fuzz/` and are **not** run in CI. Replay corpora locally with
`cargo +nightly fuzz run <target> -- -runs=0`. Prefer unit/integration regressions
for PR-gating fixes; see `fuzz/README.md` and `CONTRIBUTING.md`.
