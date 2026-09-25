# hadris-udf

A pure Rust Universal Disk Format (UDF) library for optical media and disk
images, for desktop image tools as well as `no_std` bootloaders, kernels and
firmware.

UDF (ECMA-167 with the OSTA UDF specification) is the filesystem of DVD-ROM,
DVD-Video, Blu-ray and many large removable drives.

## Features

- **Read** UDF 1.02 to 2.01 volumes with type 1 partitions, without an
  allocator, through the `hadris-fs` `FileSystem` trait: path lookup,
  streaming reads, metadata with times, permissions and owners, symlinks and
  hard links
- **Write** standalone volumes from a `hadris_fs::Tree`, reproducibly,
  with a report of where each file went
- **Bridge** images that share their file data between ISO 9660 and UDF, as
  DVD-Video uses: `plan_bridge` and `write_bridge`
- The same API in `sync`, `r#async`

## Reading

```toml
[dependencies]
hadris-udf = "2.4.0"
hadris-fs = { version = "2.4.0", features = ["std", "sync"] }
```

```rust,no_run
use std::io::Read;

use hadris_fs::{MountOptions, OpenOptions};
use hadris_fs::sync::Volume;
use hadris_udf::sync::UdfFs;

let file = hadris_storage::host::FileDevice::open("movie.udf").unwrap();
let udf = UdfFs::mount(file, MountOptions::new()).unwrap();
println!("Volume: {} (UDF {})", udf.logical_volume_id(), udf.revision());

let vol = Volume::new(udf);
for entry in vol.read_dir("/").unwrap() {
    println!("{}", String::from_utf8_lossy(entry.unwrap().name().as_bytes()));
}
let mut readme = Vec::new();
vol.open("/README.TXT", OpenOptions::new().read())
    .unwrap()
    .read_to_end(&mut readme)
    .unwrap();
```

`UdfFs` opens any `hadris_storage` block device: a host `FileDevice`, a
`MemDevice`, a `Partition` of a disk. It finds the anchor at block 256, N-256
or N-1 for logical blocks of 512 to 4096 bytes, uses the prevailing
descriptors and falls back to the reserve sequence. `hadris_fs::sync::Volume`
gives it paths, shared access and `File` handles.

## Writing

```rust,no_run
use hadris_fs::{Content, Node, Tree, host};
use hadris_udf::sync::write;
use hadris_udf::{UdfOptions, UdfRevision};

let mut tree = Tree::new();
tree.insert("readme.txt", Node::file(Content::bytes("Hello, World!"))).unwrap();
tree.insert("video.bin", Node::file(host::file("/data/video.bin").unwrap())).unwrap();

let options = UdfOptions::default()
    .with_volume_id("MY_DISC")
    .with_revision(UdfRevision::V2_01);
let out = hadris_storage::host::FileDevice::new(std::fs::File::create("disc.udf").unwrap()).unwrap();
let report = write(out, &tree, &options).unwrap();
println!("{} bytes", report.size());
for warning in report.warnings() {
    eprintln!("{warning}");
}
```

The writer stores files, directories, symlinks and hard links with their
modification and access times, permissions and owners. Creation times, DOS
attributes and special files are reported as warnings. Output is
reproducible: the writer reads no clock, and `with_time` dates the volume,
1980-01-01 by default. `plan` returns the report without I/O, to size a
device first.

`write_bridge` writes an ISO 9660 image, then a UDF volume over the same
file data, from one tree and the `IsoOptions` and `UdfOptions` of the two
volumes; `plan_bridge` returns its report.

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | Yes | Implies `alloc`; `std::io::Error` conversions and host files as tree content |
| `alloc` | via `std` | The writers, `plan` and `plan_bridge` |
| `sync` | Yes | The blocking API in `sync` |
| `async` | No | The asynchronous API with `Send` futures in `r#async` |

No feature changes what an item does.

## UDF scope

The writer produces mastered, read-only type 1 volumes and can label them
UDF 1.02, 1.50, 2.00 or 2.01; 2.00 and later use ECMA-167 3rd edition
structures. Packet writing, virtual allocation tables, sparing tables,
metadata partitions and named streams are not implemented; the reader
refuses such partitions as unsupported, and the writer refuses UDF 2.50
and 2.60, which require a metadata partition. Writing to a mounted volume
(`FileSystem` write methods) reports read-only for now.

## Documentation

- [Read and extract UDF](https://hxyulin.github.io/hadris/guides/read-udf)
- [Create UDF filesystems](https://hxyulin.github.io/hadris/creation/udf)
- [API reference](https://docs.rs/hadris-udf)

## Specifications

- ECMA-167: Volume and File Structure for Write-Once and Rewritable Media
- OSTA UDF Specification (udf260.pdf)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
