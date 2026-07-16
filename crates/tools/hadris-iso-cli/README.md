# Hadris ISO CLI

Command-line utility for ISO 9660 filesystem operations.

## Installation

```bash
cargo install hadris-iso-cli
```

Or build from source:

```bash
cargo build --release -p hadris-iso-cli
# canonical binary: target/release/hadris-iso
```

The canonical executable is `hadris-iso`; `hadris-iso-cli` remains available
as a compatibility alias.

## Usage

```bash
# Display ISO information
hadris-iso info image.iso

# List directory contents
hadris-iso ls image.iso /
hadris-iso ls image.iso /SUBDIR

# Display directory tree
hadris-iso tree image.iso

# Print a file to stdout
hadris-iso cat image.iso /README.TXT

# Extract files (default output directory: .)
hadris-iso extract image.iso -o ./out
hadris-iso extract image.iso -p /SUBDIR -o ./out

# Create a new ISO from a directory
hadris-iso create ./directory --output output.iso
hadris-iso create ./directory -o output.iso -V MY_ISO --joliet --rock-ridge

# Create a bootable ISO
hadris-iso create ./directory -o bootable.iso \
    --boot boot/bios.img \
    --efi-boot boot/efi.img \
    --joliet

# Verify ISO integrity
hadris-iso verify image.iso

# xorriso-compatible mkisofs mode
hadris-iso mkisofs -o output.iso ./directory
```

## Commands

| Command | Description |
|---------|-------------|
| `info` | Display volume descriptor and filesystem information |
| `ls` | List directory contents |
| `tree` | Display directory tree |
| `cat` | Print file contents to stdout |
| `extract` | Extract files from the ISO |
| `create` | Create a new ISO image |
| `verify` | Verify ISO image integrity |
| `mkisofs` | xorriso-compatible mkisofs mode (alias: `xorriso`) |

## Supported Features

- ISO 9660 Level 1-3 reading and writing
- Joliet extension (UTF-16 filenames)
- Rock Ridge (RRIP) extension (POSIX semantics; write support is limited — see library docs)
- El-Torito bootable images
- Hybrid MBR/GPT USB boot options on `create`
- SUSP (System Use Sharing Protocol)

## Examples

### Creating a Bootable ISO

```bash
hadris-iso create ./iso-contents \
    --output bootable.iso \
    --volume-name BOOTABLE \
    --boot boot/bios.img \
    --joliet
```

### Examining ISO Structure

```bash
hadris-iso info image.iso
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
