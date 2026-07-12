# Hadris

A Rust workspace for working with partition tables, filesystems, and disk images. Designed for no-std compatibility, embedded systems, and bootloaders.

## Workspace Crates

Crates are grouped by their storage access model. These directories are
organizational only: published package names such as `hadris-fat` are unchanged.

### Core Libraries

- **[hadris-io](crates/core/hadris-io)** - No-std I/O abstraction layer (`Read`, `Write`, `Seek`)
- **[hadris-path](crates/core/hadris-path)** - Allocation-free lexical paths for virtual filesystems and archives
- **[hadris-common](crates/core/hadris-common)** - Shared utilities (endian types, CRC, UTF-16 strings, optical helpers)
- **[hadris-storage](crates/core/hadris-storage)** - Format-neutral block geometry, device traits, and seekable-stream adapters
- **[hadris-macros](crates/core/hadris-macros)** - Proc macros for dual sync/async code generation

### Block Storage

- **[hadris-block](crates/block/hadris-block)** - Category facade for storage traits, partitions, and block filesystems, with lightweight detection, bounded partition views, and unified FAT opening
- **[hadris-part](crates/block/hadris-part)** - Partition table support
  - MBR (Legacy BIOS partition tables)
  - GPT (Modern UEFI partition tables)
  - Hybrid MBR (Combined MBR+GPT for dual BIOS/UEFI boot)
- **[hadris-fat](crates/block/hadris-fat)** - FAT filesystem implementation
  - FAT12, FAT16, FAT32 support
  - Long filename support (VFAT/LFN)
  - FAT sector caching for performance
  - Analysis and verification tools
  - ExFAT support (experimental)

### Optical Media

- **[hadris-optical](crates/optical/hadris-optical)** - Category facade with multi-format ISO/UDF/bridge detection and image composition
- **[hadris-iso](crates/optical/hadris-iso)** - ISO 9660 filesystem implementation
  - ISO 9660 Level 1-3 and ISO 9660:1999 (long filenames)
  - Joliet extension (UTF-16 Unicode filenames)
  - Rock Ridge (RRIP) and SUSP (POSIX semantics, symlinks)
  - El-Torito bootable CD/DVD images
- **[hadris-udf](crates/optical/hadris-udf)** - Universal Disk Format (UDF) for DVD/Blu-ray
- **[hadris-cd](crates/optical/hadris-cd)** - Hybrid ISO+UDF optical disc image creation

### Archives

- **[hadris-archive](crates/archive/hadris-archive)** - Category facade for sequential archive formats
- **[hadris-cpio](crates/archive/hadris-cpio)** - CPIO newc/SVR4 archives (initramfs)

### CLI Tools

| Crate | Binary | Notes |
|-------|--------|-------|
| [hadris-iso-cli](crates/tools/hadris-iso-cli) | `hadris-iso-cli` | ISO create/inspect/extract |
| [hadris-fat-cli](crates/tools/hadris-fat-cli) | `fatutil` | FAT analysis and verification |
| [hadris-cpio-cli](crates/tools/hadris-cpio-cli) | `cpioutil` | CPIO create/extract |
| [hadris-udf-cli](crates/tools/hadris-udf-cli) | `hadris-udf-cli` | UDF create/inspect |
| [hadris-cli](crates/tools/hadris-cli) | `hadris-cli` | Experimental stub (not published) |

### Meta-crate

- **[hadris](crates/core/hadris)** - Optional umbrella built on the three category facades, plus `path` utilities, with grouped APIs: `block::{storage, fat, part}`, `optical::{iso, udf, cd}`, and `archive::cpio`. Platform, I/O-mode, capability, leaf, and category features are forwarded independently; the hosted synchronous read/write configuration with `path`, `iso`, `fat`, and `cpio` is enabled by default. The hybrid `cd` writer is currently sync-only.

## Key Features

- **No-std compatible** - Use in bootloaders, kernels, and embedded systems
- **Configurable** - Feature flags for read-only, write support, and extensions
- **Dual sync/async** - Shared implementations via `hadris-macros`
- **Standards oriented** - ECMA-119, IEEE P1282 / Rock Ridge, El-Torito, Microsoft FAT, ECMA-167 / UDF, CPIO newc

## Quick Start

Crates share workspace version **1.2.1**:

```toml
[dependencies]
hadris-iso = "1.2.1"
hadris-fat = "1.2.1"
hadris-part = { version = "1.2.1", features = ["read"] }
hadris-path = "1.2.1"
```

For no-std environments:

```toml
[dependencies]
hadris-iso = { version = "1.2.1", default-features = false, features = ["read", "alloc"] }
hadris-fat = { version = "1.2.1", default-features = false, features = ["read", "sync"] }
```

## Building

```bash
# Build entire workspace
cargo build --workspace

# Run tests
cargo test --workspace

# Build for no-std (example)
cargo build -p hadris-fat --no-default-features --features "read,sync"
```

See [CLAUDE.md](CLAUDE.md) for detailed build instructions and architecture notes, and [CONTRIBUTING.md](CONTRIBUTING.md) for PR workflow.

**MSRV:** Rust 1.88.0 (`rust-toolchain.toml` / workspace `rust-version`).

Fuzz harnesses under [`fuzz/`](fuzz/) are local developer tools and are **not** part of PR CI.

## Development

Install [pre-commit](https://pre-commit.com/) hooks once per clone (runs `cargo fmt` / `cargo clippy` before commits):

```bash
# brew install pre-commit   # or: pipx install pre-commit
pre-commit install
pre-commit install --hook-type pre-push   # also run clippy on push
```

## License

Licensed under the [MIT license](LICENSE-MIT).
