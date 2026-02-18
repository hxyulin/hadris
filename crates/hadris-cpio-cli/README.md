# Hadris CPIO CLI

Command-line utility for CPIO archive operations.

## Installation

```bash
cargo install hadris-cpio-cli
```

Or build from source:

```bash
cargo build --release -p hadris-cpio-cli
```

The binary will be available as `cpioutil`.

## Usage

```bash
# List archive contents
cpioutil list archive.cpio
cpioutil list -l archive.cpio

# Display archive information
cpioutil info archive.cpio

# Create an archive from a directory
cpioutil create -o archive.cpio ./my-directory
cpioutil create -o archive.cpio --crc ./my-directory

# Extract an archive
cpioutil extract -o ./output archive.cpio

# Print a file from the archive
cpioutil cat archive.cpio path/to/file.txt
```

## Commands

| Command | Description |
|---------|-------------|
| `list` | List archive entries (like `cpio -t`) |
| `info` | Display archive format, entry count, and per-entry metadata |
| `create` | Create a CPIO archive from a directory |
| `extract` | Extract an archive to a directory |
| `cat` | Print a single file's contents to stdout |

### `list`

Lists all entries in the archive, one per line.

```bash
$ cpioutil list archive.cpio
hello.txt
subdir
subdir/nested.txt

$ cpioutil list -l archive.cpio
-rw-r--r--   501     0       12 1700000000 hello.txt
drwxr-xr-x   501     0        0 1700000000 subdir
-rw-r--r--   501     0       12 1700000000 subdir/nested.txt
```

### `info`

Shows archive-level summary and detailed per-entry metadata.

```bash
$ cpioutil info archive.cpio
CPIO Archive Information
========================
Format:       newc (070701)
Entries:      3
Total data:   24 B (24 bytes)

Entry Details
-------------
  -rw-r--r-- hello.txt
    ino=1 nlink=1 uid=501 gid=0 size=12 mtime=1700000000
    dev=0,0 rdev=0,0 check=0x00000000
  ...
```

### `create`

Packs a directory into a CPIO archive. Use `--crc` for the `070702` format.

```bash
$ cpioutil create -o archive.cpio ./my-directory
Created newc archive: archive.cpio

$ cpioutil create -o archive.cpio --crc ./my-directory
Created newc+crc archive: archive.cpio
```

### `extract`

Extracts files, directories, and symlinks. Device nodes and FIFOs are skipped with a warning.

```bash
$ cpioutil extract -o ./output archive.cpio
Extracted 3 entries to ./output
```

### `cat`

Prints a single file's contents to stdout, useful for quick inspection.

```bash
$ cpioutil cat archive.cpio hello.txt
Hello, world!
```

## Interoperability

Archives created by `cpioutil` are readable by system `cpio`:

```bash
cpio -t < archive.cpio
```

Archives created by system `cpio -H newc` are readable by `cpioutil`:

```bash
find . -depth -print | cpio -o -H newc > archive.cpio
cpioutil list -l archive.cpio
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
