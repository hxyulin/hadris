# Hadris FAT Raw

The on-disk layer of FAT12, FAT16, FAT32 and exFAT: layouts and I/O-free
codecs for drivers, tools and firmware.

## Overview

`hadris-fat-raw` holds what the `hadris-fat` drivers are built on, for
anyone whose case those drivers do not cover. `hadris-fat` re-exports it as
`hadris_fat::raw`. It has its own version, so the low-level API can change
without a new major version of `hadris-fat`.

Everything works on bytes and plain values. Nothing does I/O, and nothing
needs an allocator:

- **Layouts** of the boot sector, BPB, FSInfo sector and directory entries,
  with their constants
- **Boot sectors**: `parse_boot` checks one and returns its `Geometry`
- **FAT entries**: `FatKind` encodes and decodes entries of every width and
  classifies chain links; `ChainGuard` finds loops in a chain without memory
  proportional to it
- **Directories**: `Slot`, `ShortEntry` and `LongEntry` decode and encode
  directory slots; `lfn`, `short_name` and `name` assemble and encode long
  names, generate 8.3 names and compare names, with `fold_ascii` and
  `fold_unicode` as `fn(u16) -> u16` folds. The Unicode case tables link
  only when `fold_unicode` is used.
- **Timestamps** in `date`, and **formatting** in `layout`, which plans a
  volume from its size and encodes the boot and FSInfo sectors
- **exFAT** in `exfat`: layouts, `parse_boot`, boot, set and up-case table
  checksums, name hashes, name and time encoding, and an up-case table
  decoder

The layouts mirror the specifications. They may gain items; the existing
ones follow the specifications and stay exhaustive.

## Usage

```rust
use hadris_fat_raw::{FatKind, Slot, lfn_checksum};

let mut fat = [0u8; 8];
FatKind::Fat12.encode(3, 0xFFF, &mut fat[4..6]);
assert_eq!(FatKind::Fat12.decode(3, &fat[4..6]), 0xFFF);

let mut raw = [0u8; 32];
raw[..11].copy_from_slice(b"README  TXT");
raw[11] = hadris_fat_raw::ATTR_ARCHIVE;
let Slot::Short(entry) = Slot::parse(&raw) else { unreachable!() };
assert_eq!(entry.lfn_checksum(), lfn_checksum(b"README  TXT"));
```

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `defmt` | `defmt::Format` for `FatKind` | - |

## Documentation

- [API reference](https://docs.rs/hadris-fat-raw)
- [`hadris-fat`](../hadris-fat), the drivers built on this crate

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
