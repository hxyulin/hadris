# Hadris FAT CLI

Command-line utility for FAT12, FAT16, FAT32 and exFAT analysis and management.
Every command detects exFAT from the boot sector, so the same commands work on
both.

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
hadris-fat extract disk.img -o ./out -p /SUBDIR

# Recursively create an image from a directory
hadris-fat create ./contents --output disk.img
hadris-fat create ./contents -o disk.img --fat-type fat32 --size 134217728 -V MY_DISK
hadris-fat create ./contents -o card.img --fat-type exfat -V Photos

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
| `ls` (alias `list`) | List directory contents |
| `tree` | Display directory tree |
| `cat` | Print a file to stdout |
| `extract` | Extract one path (to `<output>/<name>`) or the complete image (into `<output>`, default `.`) |
| `create` | Recursively create a FAT12/16/32 or exFAT image from a directory |
| `fragmentation` | Analyze filesystem fragmentation |
| `chain` | Show cluster chain for a file |
| `verify` (alias `check`) | Check filesystem integrity without changing the image; exits with an error when it finds problems |

## Known Limitations

- Host symbolic links and other special file types are rejected during creation.
- FAT12/16/32 volume labels passed to `create` must be at most 11 ASCII
  characters; they are stored in uppercase. exFAT labels may have up to 11
  UTF-16 code units and keep their case.
- `create --fat-type exfat` writes FAT chains; TexFAT volumes are read and
  checked but not created.

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

- FAT12, FAT16, FAT32 and exFAT filesystems
- Long filename (LFN/VFAT) display
- Directory traversal and tree view
- Fragmentation and cluster-chain analysis
- Filesystem verification

The commands are built on the `hadris_fat::sync::FatFs` and
`hadris_fat::exfat::sync::ExFatFs` drivers, the checkers, `format`, the `raw`
boot sector layouts, and the `hadris-fs` path and host helpers
(`extract_to_host`, `import_from_host`, `Tree::from_fs`). Read commands mount
images read-only.

## Documentation

- [Read a FAT image](https://hxyulin.github.io/hadris/guides/read-fat-image)
- [Modify FAT safely](https://hxyulin.github.io/hadris/guides/modify-fat)
- [Library API](https://docs.rs/hadris-fat)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
