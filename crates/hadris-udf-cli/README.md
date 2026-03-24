# Hadris UDF CLI

Command-line utility for UDF (Universal Disk Format) filesystem operations.
UDF is the filesystem used for DVD-ROM, DVD-Video, Blu-ray discs, and large USB drives.

## Installation

```bash
cargo install hadris-udf-cli
```

Or build from source:

```bash
cargo build --release -p hadris-udf-cli
```

## Usage

```bash
# Display UDF image information
hadris-udf info image.udf

# List directory contents
hadris-udf ls image.udf
hadris-udf ls image.udf /subdir

# Display directory tree
hadris-udf tree image.udf
hadris-udf tree image.udf --depth 2

# Create a new UDF image from a directory
hadris-udf create ./my-files --output image.udf
hadris-udf create ./my-files --output image.udf --volume-name MY_DISC --revision 2.50

# Verify UDF image integrity
hadris-udf verify image.udf
hadris-udf verify image.udf --verbose
```

## Commands

| Command  | Description                                      |
|----------|--------------------------------------------------|
| `info`   | Display volume information (ID, revision, size)  |
| `ls`     | List directory contents                          |
| `tree`   | Display directory tree structure                 |
| `create` | Create a new UDF image from a local directory    |
| `verify` | Verify UDF image structural integrity            |

## Supported UDF Revisions

| Revision | Target Media          |
|----------|-----------------------|
| `1.02`   | DVD-ROM (default)     |
| `1.50`   | DVD-RAM, packet write |
| `2.01`   | DVD-RW streaming      |
| `2.50`   | Blu-ray               |
| `2.60`   | Blu-ray pseudo-OW     |

## Known Limitations

- File content extraction (`cat`, `extract`) is not yet supported because the
  UDF library does not yet expose a public file-read API.
- `UdfDirEntry::size` is currently always 0 (a placeholder in the library);
  long listing shows `N/A` for file sizes.

## Examples

### Inspecting a DVD image

```bash
$ hadris-udf info movie.iso
UDF Image: movie.iso

Volume Information:
  Volume ID:         MOVIE_TITLE
  UDF Revision:      1.02
  Block Size:        2048 bytes
  Partition Start:   sector 270
  Partition Length:  1234 sectors (2525184 bytes)
```

### Creating a UDF image

```bash
hadris-udf create ./content --output disc.udf --volume-name MY_DISC --revision 1.50
```

## License

Licensed under the [MIT license](../../LICENSE-MIT).
