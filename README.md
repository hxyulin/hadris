# Hadris

A Rust workspace for working with partition tables, filesystems, and disk images. Designed for no-std compatibility, embedded systems, and bootloaders.

## Workspace Crates

### Core Libraries

- **[hadris-io](crates/hadris-io)** - No-std I/O abstraction layer (Read, Write, Seek traits)
- **[hadris-common](crates/hadris-common)** - Shared utilities (endian types, CRC, UTF-16 strings)

### Partition Tables

- **[hadris-part](crates/hadris-part)** - Partition table support
  - MBR (Legacy BIOS partition tables)
  - GPT (Modern UEFI partition tables)
  - Hybrid MBR (Combined MBR+GPT for dual BIOS/UEFI boot)

### Filesystems

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

### CLI Tools

- **[hadris-iso-cli](crates/hadris-iso-cli)** - Command-line tool for ISO operations
- **[hadris-fat-cli](crates/hadris-fat-cli)** - Command-line tool for FAT operations
- **[hadris-cli](crates/hadris-cli)** - General-purpose disk utility CLI (WIP)

### Meta-crate

- **[hadris](crates/hadris)** - Re-exports all filesystem implementations

## Key Features

- **No-std compatible** - Use in bootloaders, kernels, and embedded systems
- **Configurable** - Feature flags for read-only, write support, and extensions
- **Comprehensive** - Support for multiple partition schemes and filesystem extensions
- **Standards compliant** - Follows ECMA-119, IEEE P1282, El-Torito, and Microsoft FAT specifications

## Quick Start

Add dependencies to your `Cargo.toml`:

```toml
[dependencies]
hadris-iso = "0.2"
hadris-fat = "0.3"
hadris-part = "0.2"
```

For no-std environments:

```toml
[dependencies]
hadris-iso = { version = "0.2", default-features = false, features = ["read"] }
hadris-fat = { version = "0.3", default-features = false, features = ["read"] }
```

## Building

```bash
# Build entire workspace
cargo build --workspace

# Run tests
cargo test --workspace

# Build for no-std (example)
cargo build -p hadris-fat --no-default-features --features "read"
```

See [CLAUDE.md](CLAUDE.md) for detailed build instructions and architecture notes.

## License

Licensed under the [MIT license](LICENSE-MIT).
