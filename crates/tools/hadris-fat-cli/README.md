# Hadris FAT CLI

Command-line utility for FAT filesystem analysis and management.

## Installation

```bash
cargo install hadris-fat-cli
```

Or build from source:

```bash
cargo build --release -p hadris-fat-cli
# canonical binary: target/release/hadris-fat
```

The canonical binary is **`hadris-fat`**. The legacy **`fatutil`** executable
remains available as a compatibility alias.

## Usage

```bash
# Display volume information
hadris-fat info disk.img

# Detailed filesystem statistics
hadris-fat stat disk.img

# List directory contents
hadris-fat ls disk.img /
hadris-fat ls disk.img /SUBDIR

# Display directory tree
hadris-fat tree disk.img

# Print and extract files
hadris-fat cat disk.img /README.TXT
hadris-fat extract disk.img --output ./out

# Recursively create an image from a directory
hadris-fat create ./contents --output disk.img
hadris-fat create ./contents -o disk.img --fat-type fat32 --size 134217728 -V MY_DISK

# Analyze fragmentation
hadris-fat fragmentation disk.img

# Show cluster chain for a file
hadris-fat chain disk.img /README.TXT

# Verify filesystem integrity
hadris-fat verify disk.img
```

## Commands

| Command | Description |
|---------|-------------|
| `info` | Display boot sector and volume information |
| `stat` | Show detailed filesystem statistics |
| `ls` | List directory contents |
| `tree` | Display directory tree |
| `cat` | Print a file to stdout |
| `extract` | Extract one path or the complete image |
| `create` | Recursively create a FAT12/16/32 image from a directory |
| `fragmentation` | Analyze filesystem fragmentation |
| `chain` | Show cluster chain for a file |
| `verify` | Check filesystem integrity |

## Known Limitations

- Host symbolic links and other special file types are rejected during creation.
- ExFAT images are not exposed through this CLI.

## Examples

### Examining a Disk Image

```bash
hadris-fat info disk.img
hadris-fat stat disk.img
```

### Listing Files

```bash
hadris-fat ls disk.img /
```

## Supported Features

- FAT12, FAT16, FAT32 filesystems
- Long filename (LFN/VFAT) display
- Directory traversal and tree view
- Fragmentation and cluster-chain analysis
- Filesystem verification

## License

Licensed under the [MIT license](../../LICENSE-MIT).
