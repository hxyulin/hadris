# Hadris FAT CLI

Command-line utility for FAT filesystem analysis and management.

## Installation

```bash
cargo install hadris-fat-cli
```

Or build from source:

```bash
cargo build --release -p hadris-fat-cli
```

The binary will be available as `fatutil`.

## Usage

```bash
# Display filesystem information
fatutil info disk.img

# List directory contents
fatutil ls disk.img /
fatutil ls disk.img /SUBDIR

# Extract a file
fatutil extract disk.img /path/to/file.txt output.txt

# Show filesystem statistics
fatutil stats disk.img

# Verify filesystem integrity
fatutil verify disk.img
```

## Commands

| Command | Description |
|---------|-------------|
| `info` | Display boot sector and filesystem information |
| `ls` | List directory contents |
| `extract` | Extract a file from the filesystem |
| `stats` | Show cluster and space usage statistics |
| `verify` | Check filesystem integrity |

## Examples

### Examining a Disk Image

```bash
$ fatutil info disk.img
FAT Type: FAT32
Volume Label: MYDISK
Cluster Size: 4096 bytes
Total Clusters: 32768
Free Clusters: 28500
```

### Listing Files

```bash
$ fatutil ls disk.img /
Name            Size       Attr    Cluster
BOOT            <DIR>      D----   3
CONFIG.SYS      1024       A----   100
README.TXT      2048       A----   105
```

## Supported Features

- FAT12, FAT16, FAT32 filesystems
- Long filename (LFN/VFAT) display
- Directory traversal
- File extraction
- Filesystem verification
- Cluster analysis

## License

Licensed under the [MIT license](../../LICENSE-MIT).
