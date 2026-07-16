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
# canonical binary: target/release/hadris-udf
```

The canonical binary is **`hadris-udf`**. The legacy **`hadris-udf-cli`**
executable remains available as a compatibility alias.

## Usage

```bash
# Display UDF image information
hadris-udf info image.udf

# List directory contents
hadris-udf ls image.udf
hadris-udf ls image.udf /subdir -l

# Display directory tree
hadris-udf tree image.udf
hadris-udf tree image.udf --depth 2

# Print a file to stdout
hadris-udf cat image.udf /readme.txt

# Extract files (default output directory: .)
hadris-udf extract image.udf -o ./out
hadris-udf extract image.udf -p /subdir -o ./out

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
| `cat`    | Print file contents to stdout                    |
| `extract`| Extract files from the image                     |
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

## License

Licensed under the [MIT license](../../LICENSE-MIT).
