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
| `ls` (alias `list`) | List directory contents |
| `tree` | Display directory tree |
| `cat` | Print file contents to stdout |
| `extract` | Extract one path (to `<output>/<name>`) or the whole image (into `<output>`, default `.`) |
| `create` | Create a new ISO image |
| `verify` (alias `check`) | Verify ISO image integrity; exits with an error when it finds errors |
| `mkisofs` | xorriso-compatible mkisofs mode (alias: `xorriso`) |

## Supported Features

- ISO 9660 Level 1-3 reading and writing (`--level 1`, `2`, `3`, or `1l`
  and `2l` to keep lowercase names)
- Joliet extension (UCS-2 filenames)
- Rock Ridge (RRIP) extension: modes, owners, times, symlinks, device nodes
  and hard links, with deep directories relocated
- El Torito bootable images, with a visible `boot.catalog`
- Hybrid MBR/GPT USB boot options on `create`
- Listings, `cat` and `extract` use the Rock Ridge or Joliet names when the
  image has them; a path not found there, such as `/README.TXT`, is looked up
  in the primary tree, ignoring ASCII case
- `create --dry-run` prints the planned image size
- `verify --strict` also checks the path table, extent bounds and Rock
  Ridge fields
- `extract` copies symlinks (on Unix), hard links, file modes and
  modification times through `hadris_fs::host::write_tree`, and refuses
  names that would leave the output directory; device nodes, FIFOs and
  sockets are skipped with a warning

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

## Documentation

- [Read ISO images](https://hxyulin.github.io/hadris/guides/read-iso)
- [Create ISO images](https://hxyulin.github.io/hadris/creation/iso)
- [Library API](https://docs.rs/hadris-iso)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
