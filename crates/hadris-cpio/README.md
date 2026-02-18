# Hadris CPIO

A Rust implementation of the CPIO archive format (newc/SVR4) with support for no-std environments, streaming reads, and archive creation.

## Features

- **Read & Write Support** - Stream entries from existing archives, create new ones
- **No-std Compatible** - Use in bootloaders, kernels, and embedded systems
- **newc Format** - Both `070701` (standard) and `070702` (CRC) variants
- **Full Entry Types** - Regular files, directories, symlinks, hard links, device nodes, FIFOs
- **Filesystem Scanning** - Build archives directly from a host directory tree

## Quick Start

### Reading an Archive

```rust
use std::fs::File;
use std::io::BufReader;
use hadris_cpio::CpioReader;

let file = File::open("archive.cpio")?;
let mut reader = CpioReader::new(BufReader::new(file));

while let Some(entry) = reader.next_entry_alloc()? {
    let name = entry.name_str().unwrap_or("<invalid>");
    println!("{} ({} bytes)", name, entry.file_size());
    reader.skip_entry_data_owned(&entry)?;
}
```

### Creating an Archive from a Directory

```rust
use std::fs::File;
use std::io::BufWriter;
use hadris_cpio::{CpioWriteOptions, CpioWriter, FileTree};

let tree = FileTree::from_fs(std::path::Path::new("./my-directory"))?;
let writer = CpioWriter::new(CpioWriteOptions::default());

let mut out = BufWriter::new(File::create("archive.cpio")?);
writer.write(&mut out, &tree)?;
```

### Building an Archive Programmatically

```rust
use hadris_cpio::{CpioWriteOptions, CpioWriter, FileNode, FileTree};

let mut tree = FileTree::new();
tree.add(FileNode::file("hello.txt", b"Hello, world!\n".to_vec(), 0o644));
tree.add(FileNode::dir("subdir", vec![
    FileNode::file("nested.txt", b"Nested content\n".to_vec(), 0o644),
], 0o755));
tree.add(FileNode::symlink("link.txt", "hello.txt"));

let writer = CpioWriter::new(CpioWriteOptions::default());
let mut buf = Vec::new();
writer.write(&mut buf, &tree)?;
```

## Feature Flags

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `read` | Streaming archive reader | None |
| `alloc` | Heap allocation without full std | `alloc` crate |
| `std` | Full standard library support | `std`, `alloc` |
| `write` | Archive creation | `alloc`, `read` |

Default features: `std`, `read`, `write`

### For Bootloaders (minimal footprint)

```toml
[dependencies]
hadris-cpio = { version = "1.0", default-features = false, features = ["read"] }
```

### For Kernels with Heap (no-std + alloc)

```toml
[dependencies]
hadris-cpio = { version = "1.0", default-features = false, features = ["read", "alloc"] }
```

### For Desktop Applications (full features)

```toml
[dependencies]
hadris-cpio = { version = "1.0" }  # Uses default features
```

## Archive Format

This crate implements the "new" (newc) ASCII CPIO format, which is the format used by:
- Linux initramfs images (`gen_init_cpio`)
- RPM package payloads
- The `cpio -H newc` command

Each entry consists of a 110-byte ASCII header, a NUL-terminated filename, and file data. All sections are padded to 4-byte boundaries. The archive ends with a `TRAILER!!!` sentinel.

Two magic numbers are supported:
- `070701` - Standard newc format
- `070702` - newc with per-file CRC checksums

## No-std Compatibility

The crate is designed for no-std environments:

- Core reading requires only the `read` feature (zero allocations with `next_entry_with_buf`)
- Allocating reader requires `alloc` (uses `Vec` for filenames and data)
- Writing and filesystem scanning require `alloc` and `std` respectively
- All I/O uses `hadris-io` traits instead of `std::io`

## Interoperability

Archives created with this crate are compatible with:
- GNU cpio (`cpio -t`, `cpio -i`)
- Linux kernel initramfs loader
- RPM tools

## Specification References

- `cpio(5)` man page
- Linux kernel `usr/gen_init_cpio.c`
- RPM file format specification

## License

This project is licensed under the [MIT license](../../LICENSE-MIT).
