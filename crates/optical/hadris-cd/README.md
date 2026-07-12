# hadris-cd

A Rust library for creating hybrid ISO+UDF optical disc images (UDF Bridge format).

## Overview

This crate creates images that contain both ISO 9660 and UDF filesystems sharing the same underlying file data. This provides maximum compatibility:

- Legacy systems read ISO 9660
- Modern systems read UDF
- Both filesystems point to the same file data on disk

## Quick Start

```rust,no_run
use hadris_cd::{CdWriter, CdOptions, FileTree, FileEntry};

let mut tree = FileTree::new();
tree.add_file(FileEntry::from_buffer("readme.txt", b"Hello, World!".to_vec()));

let options = CdOptions::with_volume_id("MY_DISC")
    .with_joliet();

let file = std::fs::File::create("output.iso").unwrap();
CdWriter::new(file, options)
    .write(tree)
    .unwrap();
```

## Disk Layout

The UDF Bridge format interleaves ISO 9660 and UDF structures:

```text
Sector 0-15:    System area (boot code, partition tables)
Sector 16-...:  ISO 9660 Volume Descriptors
Sector 17-19:   UDF Volume Recognition Sequence (BEA01, NSR02, TEA01)
Sector 256:     UDF Anchor Volume Descriptor Pointer
Sector 257+:    UDF Volume Descriptor Sequence
File data:      Shared between ISO and UDF (both point to same sectors)
```

## Features

- **ISO 9660** with Joliet (Windows long filenames) and Rock Ridge (POSIX)
- **UDF 1.02/1.50/2.00+** support
- **El-Torito** bootable images (BIOS and UEFI)
- **Hybrid MBR+GPT** for USB booting

## License

MIT
