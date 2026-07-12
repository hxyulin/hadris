# Hadris FAT CLI

Command-line utility for FAT filesystem analysis and management.

## Installation

```bash
cargo install hadris-fat-cli
```

Or build from source:

```bash
cargo build --release -p hadris-fat-cli
# binary: target/release/fatutil
```

The installed binary is named **`fatutil`**.

## Usage

```bash
# Display volume information
fatutil info disk.img

# Detailed filesystem statistics
fatutil stat disk.img

# List directory contents
fatutil ls disk.img /
fatutil ls disk.img /SUBDIR

# Display directory tree
fatutil tree disk.img

# Analyze fragmentation
fatutil fragmentation disk.img

# Show cluster chain for a file
fatutil chain disk.img /README.TXT

# Verify filesystem integrity
fatutil verify disk.img
```

## Commands

| Command | Description |
|---------|-------------|
| `info` | Display boot sector and volume information |
| `stat` | Show detailed filesystem statistics |
| `ls` | List directory contents |
| `tree` | Display directory tree |
| `fragmentation` | Analyze filesystem fragmentation |
| `chain` | Show cluster chain for a file |
| `verify` | Check filesystem integrity |

## Known Limitations

- Read/analysis focused: there is no `cat`, `extract`, or `format` subcommand yet
  (the `hadris-fat` library supports file read/write and formatting).
- ExFAT images are not exposed through this CLI.

## Examples

### Examining a Disk Image

```bash
fatutil info disk.img
fatutil stat disk.img
```

### Listing Files

```bash
fatutil ls disk.img /
```

## Supported Features

- FAT12, FAT16, FAT32 filesystems
- Long filename (LFN/VFAT) display
- Directory traversal and tree view
- Fragmentation and cluster-chain analysis
- Filesystem verification

## License

Licensed under the [MIT license](../../LICENSE-MIT).
