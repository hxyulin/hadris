# Hadris

A Rust workspace for working with partition tables, filesystems, and disk images. Designed for no-std compatibility, embedded systems, and bootloaders.

## Workspace Crates

### Core Libraries

- **[hadris-io](crates/hadris-io)** - No-std I/O abstraction layer (`Read`, `Write`, `Seek`)
- **[hadris-common](crates/hadris-common)** - Shared utilities (endian types, CRC, UTF-16 strings, optical helpers)
- **[hadris-macros](crates/hadris-macros)** - Proc macros for dual sync/async code generation

### Partition Tables

- **[hadris-part](crates/hadris-part)** - Partition table support
  - MBR (Legacy BIOS partition tables)
  - GPT (Modern UEFI partition tables)
  - Hybrid MBR (Combined MBR+GPT for dual BIOS/UEFI boot)

### Filesystems and Archives

- **[hadris-iso](crates/hadris-iso)** - ISO 9660 filesystem implementation
  - ISO 9660 Level 1-3 and ISO 9660:1999 (long filenames)
  - Joliet extension (UTF-16 Unicode filenames)
  - Rock Ridge (RRIP) and SUSP (POSIX semantics, symlinks)
  - El-Torito bootable CD/DVD images
- **[hadris-fat](crates/hadris-fat)** - FAT filesystem implementation
  - FAT12, FAT16, FAT32 support
  - Long filename support (VFAT/LFN)
  - FAT sector caching for performance
  - Analysis and verification tools
  - ExFAT support (experimental)
- **[hadris-udf](crates/hadris-udf)** - Universal Disk Format (UDF) for DVD/Blu-ray
- **[hadris-cpio](crates/hadris-cpio)** - CPIO newc/SVR4 archives (initramfs)
- **[hadris-cd](crates/hadris-cd)** - Hybrid ISO+UDF optical disc image creation

### CLI Tools

| Crate | Binary | Notes |
|-------|--------|-------|
| [hadris-iso-cli](crates/hadris-iso-cli) | `hadris-iso-cli` | ISO create/inspect/extract |
| [hadris-fat-cli](crates/hadris-fat-cli) | `fatutil` | FAT analysis and verification |
| [hadris-cpio-cli](crates/hadris-cpio-cli) | `cpioutil` | CPIO create/extract |
| [hadris-udf-cli](crates/hadris-udf-cli) | `hadris-udf-cli` | UDF create/inspect |
| [hadris-cli](crates/hadris-cli) | `hadris-cli` | Experimental stub (not published) |

### Meta-crate

- **[hadris](crates/hadris)** - Optional umbrella that re-exports format crates behind feature flags (`iso9660`, `fat`, `cpio` by default; `udf` opt-in). Does not re-export `hadris-part` or `hadris-cd`.

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
