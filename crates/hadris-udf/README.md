# hadris-udf

A Rust implementation of the UDF (Universal Disk Format) filesystem.

UDF (ECMA-167) is the filesystem used for DVD-ROM, DVD-Video, DVD-RAM, Blu-ray discs, large USB drives (files >4GB), and packet writing to CD/DVD-RW.

## Features

- **Read** UDF 1.02 images (DVD-ROM)
- **Write/format** UDF filesystems from scratch
- **no\_std** compatible (with `alloc`)
- Descriptor-level access for building hybrid images

## Quick Start

```rust,no_run
use std::fs::File;
use std::io::BufReader;
use hadris_udf::UdfFs;

let file = File::open("movie.udf").unwrap();
let reader = BufReader::new(file);
let udf = UdfFs::open(reader).unwrap();

let info = udf.info();
println!("Volume: {}", info.volume_id);

for entry in udf.root_dir().unwrap().entries() {
    println!("{}", entry.name());
}
```

### Writing a UDF image

```rust,no_run
use hadris_udf::write::{UdfWriter, UdfWriteOptions, SimpleFile, SimpleDir};
use std::io::Cursor;

let mut buffer = vec![0u8; 10 * 1024 * 1024];
let mut cursor = Cursor::new(&mut buffer[..]);

let mut root = SimpleDir::new("");
root.add_file(SimpleFile::new("readme.txt", b"Hello, World!".to_vec()));

let options = UdfWriteOptions::default();
UdfWriter::format(&mut cursor, &root, options).expect("Format failed");
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `read`  | Yes     | Read support |
| `alloc` | No      | Heap allocation without full std |
| `std`   | Yes     | Full standard library support |
| `write` | No      | Write/format support (requires std) |

## Supported UDF Revisions

| Revision | Use case | Status |
|----------|----------|--------|
| UDF 1.02 | DVD-ROM | Supported |
| UDF 1.50 | DVD-RAM, packet writing | Planned |
| UDF 2.01 | DVD-RW, streaming | Planned |
| UDF 2.50 | Blu-ray | Planned |

## Specifications

- ECMA-167: Volume and File Structure for Write-Once and Rewritable Media
- OSTA UDF Specification (udf260.pdf)

## License

MIT
