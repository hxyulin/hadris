# hadris-udf

A pure Rust Universal Disk Format (UDF) library for optical media and disk
images, for desktop image tools as well as `no_std` bootloaders, kernels and
firmware.

UDF (ECMA-167 with the OSTA UDF specification) is the filesystem of DVD-ROM,
DVD-Video, Blu-ray and many large removable drives.

## Features

- **Read** UDF 1.02 to 2.01 volumes with type 1 partitions, without an
  allocator, through the `hadris-fs` `FsDriver` node API: path lookup,
  streaming reads, metadata with times, permissions and owners, symlinks and
  hard links
- **Write** standalone volumes from a `hadris_fs::tree::Tree`, reproducibly,
  with a report of where each file went
- **Bridge** volumes that share an image and its file data with ISO 9660, as
  `hadris-cd` builds them
- The same API in `sync`, `r#async` and `async_send`

## Reading

```toml
[dependencies]
hadris-udf = "2.4.0"
hadris-fs = { version = "2.4.0", features = ["std", "sync"] }
```

```rust,no_run
use hadris_fs::sync::DriverExt;
use hadris_udf::sync::UdfFs;

let file = hadris_storage::host::FileDevice::open("movie.udf").unwrap();
let mut udf = UdfFs::open(file).unwrap();
println!("Volume: {} (UDF {})", udf.logical_volume_id(), udf.revision());

for entry in udf.read_dir("/").unwrap() {
    println!("{}", String::from_utf8_lossy(entry.unwrap().name_bytes()));
}
let readme = udf.read_to_vec("/README.TXT").unwrap();
```

`UdfFs` opens any `hadris_storage` block device: a host `FileDevice`, a
`MemDevice`, a `Partition` of a disk. It finds the anchor at block 256, N-256
or N-1 for logical blocks of 512 to 4096 bytes, uses the prevailing
descriptors and falls back to the reserve sequence. Wrap it in
`hadris_fs::sync::Volume` for shared access and `File` handles.

## Writing

```rust,no_run
use hadris_fs::tree::{Content, Tree};
use hadris_udf::sync::write;
use hadris_udf::{UdfOptions, UdfRevision};

let mut tree = Tree::new();
tree.add_file("readme.txt", Content::bytes("Hello, World!")).unwrap();
tree.add_file("video.bin", Content::path("/data/video.bin")).unwrap();

let options = UdfOptions::default()
    .with_volume_id("MY_DISC")
    .with_revision(UdfRevision::V2_01);
let out = hadris_storage::host::FileDevice::new(std::fs::File::create("disc.udf").unwrap()).unwrap();
let report = write(out, &tree, &options).unwrap();
println!("{} blocks", report.total_blocks());
for warning in report.warnings() {
    eprintln!("{warning}");
}
```

The writer stores files, directories, symlinks and hard links with their
modification, access and change times, permissions and owners. Creation
times, DOS attributes and device nodes are reported as warnings. Output is
reproducible: the clock is injected, and the default `NoClock` dates the
volume 1980-01-01. `plan` returns the report without writing, to size a
device first.

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | Yes | Implies `alloc`; `std::io::Error` conversions and host files as tree content |
| `alloc` | via `std` | The writer and the tree input |
| `sync` | Yes | The blocking API in `sync` |
| `async` | No | The asynchronous API in `r#async` |
| `async-send` | No | The asynchronous API with `Send` futures in `async_send` |

No feature changes what an item does.

## UDF scope

The writer produces mastered, read-only type 1 volumes and can label them
UDF 1.02, 1.50, 2.00 or 2.01; 2.00 and later use ECMA-167 3rd edition
structures. Packet writing, virtual allocation tables, sparing tables,
metadata partitions and named streams are not implemented; the reader
refuses such partitions as unsupported, and the writer refuses UDF 2.50
and 2.60, which require a metadata partition. Writing to a mounted volume
(`FsDriver` write methods) reports read-only for now.

## Documentation

- [Read and extract UDF](https://hxyulin.github.io/hadris/guides/read-udf)
- [Create UDF filesystems](https://hxyulin.github.io/hadris/creation/udf)
- [API reference](https://docs.rs/hadris-udf)

## Specifications

- ECMA-167: Volume and File Structure for Write-Once and Rewritable Media
- OSTA UDF Specification (udf260.pdf)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
