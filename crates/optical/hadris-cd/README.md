# hadris-cd

A Rust library for creating hybrid ISO+UDF optical disc images (UDF Bridge format).

## Overview

This crate creates images that contain both ISO 9660 and UDF filesystems sharing the same underlying file data. This provides maximum compatibility:

- Legacy systems read ISO 9660
- Modern systems read UDF
- Both filesystems point to the same file data on disk

## Quick Start

```rust,no_run
use hadris_cd::CdOptions;
use hadris_fs::tree::{Content, FromFsOptions, Tree};

let mut tree = Tree::from_fs("image-root", FromFsOptions::new()).unwrap();
tree.add_file("readme.txt", Content::bytes("Hello, World!")).unwrap();

let file = std::fs::File::options()
    .read(true)
    .write(true)
    .create(true)
    .truncate(true)
    .open("output.iso")
    .unwrap();
let out = hadris_storage::host::FileDevice::new(file).unwrap();
let report = hadris_cd::sync::write(out, &tree, &CdOptions::default()).unwrap();
println!("{} blocks", report.total_blocks());
```

`CdOptions` holds the `IsoOptions` and `UdfOptions` of the two volumes
(`with_iso`, `with_udf`, `with_clock`); both crates are re-exported as
`hadris_cd::iso` and `hadris_cd::udf`. The writer places the ISO 9660
structures after the UDF metadata, writes the ISO 9660 image, then writes
the UDF volume in bridge mode pointing at the file extents the ISO report
gives. Nothing is read back from the output. `plan` returns the report
without writing, to size a device first.

## Disk Layout

The UDF Bridge format interleaves ISO 9660 and UDF structures:

```text
Sector 0-15:    System area (boot code, partition tables)
Sector 16-...:  ISO 9660 volume descriptors, then the UDF recognition
                sequence (BEA01, NSR02 or NSR03, TEA01)
Sector 256:     UDF anchor volume descriptor pointer
Sector 257-289: UDF volume descriptor sequences and integrity descriptor
Sector 290-...: UDF file set, file entries and directories
Then:           ISO 9660 directories, path tables and the file data,
                shared by both trees
End:            UDF anchor at N-256 and 256 blocks after it
```

## Features

- **ISO 9660** with Joliet (Windows long filenames) and Rock Ridge (POSIX)
- **Selectable mastered UDF revisions** from 1.02 through 2.01
- **El-Torito** bootable images (BIOS and UEFI)
- **Hybrid MBR+GPT** for USB booting

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | Yes | `std::io::Error` conversions and host files as tree content |
| `sync` | Yes | The blocking API in `sync` |
| `async` | No | The asynchronous API with `Send` futures in `r#async` |

The crate needs an allocator but not `std`. No feature changes what an item
does.

Revision selection describes mastered Type-1 output; it does not add packet
writing, VAT, sparing, metadata partitions, or pseudo-overwrite.

## Documentation

- [Create UDF filesystems](https://hxyulin.github.io/hadris/creation/udf)
- [Create ISO 9660 images](https://hxyulin.github.io/hadris/creation/iso)
- [Validate generated images](https://hxyulin.github.io/hadris/guides/validate-images)
- [API reference](https://docs.rs/hadris-cd)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
