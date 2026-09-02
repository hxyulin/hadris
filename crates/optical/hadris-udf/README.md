# hadris-udf

A pure Rust Universal Disk Format (UDF) filesystem library for optical media
and disk images, with `std` and `no_std` support. It is suitable for desktop
image tools, bootloaders, kernels, firmware, and embedded systems.

UDF (ECMA-167) is the filesystem used for DVD-ROM, DVD-Video, DVD-RAM, Blu-ray discs, large USB drives (files >4GB), and packet writing to CD/DVD-RW.

## Features

- **Read** UDF 1.02 images (DVD-ROM)
- **Write/format** UDF filesystems from scratch
- **no\_std** compatible (with `alloc`)
- Descriptor-level access for building hybrid images

## Quick Start

```toml
[dependencies]
hadris-udf = "2.3.0"
```

```rust,no_run
use std::fs::File;
use std::io::BufReader;
use hadris_udf::UdfVolume;

let file = File::open("movie.udf").unwrap();
let reader = BufReader::new(file);
let udf = UdfVolume::open(reader).unwrap();

let info = udf.info();
println!("Volume: {}", info.volume_id);

for entry in udf.root_dir().unwrap().entries() {
    println!("{}", entry.name());
}
```

### Writing a UDF image

Enable the writer explicitly:

```toml
[dependencies]
hadris-udf = { version = "2.3.0", features = ["write"] }
```

```rust,no_run
use hadris_udf::write::{UdfWriter, UdfWriteOptions, SimpleFile, SimpleDir};
use std::io::Cursor;

let mut buffer = vec![0u8; 10 * 1024 * 1024];
let mut cursor = Cursor::new(&mut buffer[..]);

let mut root = SimpleDir::new("");
root.add_file(SimpleFile::new("readme.txt", b"Hello, World!".to_vec()));

let options = UdfWriteOptions::default();
let output = UdfWriter::create(cursor, &root, options).expect("Create failed");
let _cursor = output.into_inner();
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `read`  | Yes     | Read support |
| `alloc` | No      | Heap allocation without full std |
| `std`   | Yes     | Full standard library support |
| `write` | No      | Mastered image creation; requires `std`, `alloc`, and `read` |
| `sync`  | Yes     | Synchronous filesystem API |
| `async` | No      | Asynchronous filesystem API |

`std` and the I/O mode are independent. Default features select `std`, `read`,
and `sync`; custom builds may select `sync`, `async`, or both.

## UDF scope

The reader and writer support mastered, read-only Type-1 images. The writer can
label output as UDF 1.02, 1.50, 2.00, 2.01, 2.50, or 2.60 and emits the matching
NSR identifier. This does not implement the rewritable-media features often
associated with those revisions, such as packet writing, VAT, sparing tables,
metadata partitions, or pseudo-overwrite.

Use the oldest mastered revision accepted by the target consumers and validate
the result with those consumers.

## Documentation

- [Read and extract UDF](https://hxyulin.github.io/hadris/guides/read-udf)
- [Create UDF filesystems](https://hxyulin.github.io/hadris/creation/udf)
- [API reference](https://docs.rs/hadris-udf)

## Specifications

- ECMA-167: Volume and File Structure for Write-Once and Rewritable Media
- OSTA UDF Specification (udf260.pdf)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
