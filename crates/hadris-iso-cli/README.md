# Hadris ISO CLI

Command-line utility for ISO 9660 filesystem operations.

## Installation

```bash
cargo install hadris-iso-cli
```

Or build from source:

```bash
cargo build --release -p hadris-iso-cli
```

## Usage

```bash
# Display ISO information
hadris-iso-cli info image.iso

# List directory contents
hadris-iso-cli ls image.iso /
hadris-iso-cli ls image.iso /SUBDIR

# Extract files from an ISO
hadris-iso-cli extract image.iso /path/to/file.txt output.txt

# Create a new ISO
hadris-iso-cli create output.iso --source ./directory

# Create a bootable ISO
hadris-iso-cli create output.iso --source ./directory --boot boot/efi.img
```

## Commands

| Command | Description |
|---------|-------------|
| `info` | Display volume descriptor and filesystem information |
| `ls` | List directory contents |
| `extract` | Extract files from the ISO |
| `create` | Create a new ISO image |

## Supported Features

- ISO 9660 Level 1-3 reading and writing
- Joliet extension (UTF-16 filenames)
- Rock Ridge (RRIP) extension (POSIX semantics)
- El-Torito bootable images
- SUSP (System Use Sharing Protocol)

## Examples

### Creating a Bootable ISO

```bash
hadris-iso-cli create bootable.iso \
    --source ./iso-contents \
    --boot boot/bios.img \
    --label "BOOTABLE" \
    --joliet
```

### Examining ISO Structure

```bash
$ hadris-iso-cli info image.iso
Volume ID: MY_ISO
Volume Size: 650 MB
Block Size: 2048
Extensions: Joliet, Rock Ridge
Boot: El-Torito (No Emulation)
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
