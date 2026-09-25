# Hadris FAT Raw

The on-disk layer of FAT12, FAT16, FAT32 and exFAT: layouts and I/O-free
codecs for drivers, tools and firmware.

## Overview

`hadris-fat-raw` holds what the `hadris-fat` drivers are built on, for
anyone whose case those drivers do not cover. It has its own version, so the
low-level API can change without a new major version of `hadris-fat`, and
`hadris-fat` re-exports only the items its own API uses (`FatKind`,
`Detail`, `exfat::Detail` and `check`).

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
- **Detail codes**: `Detail` and `exfat::Detail` say what exactly is wrong
  with a volume, in the errors of the device primitives and in the
  findings of a check

The layouts mirror the specifications. They may gain items; the existing
ones follow the specifications and stay exhaustive.

With a mode feature, `io` adds the FAT device primitives the `FatFs` driver
is built on, in `io::sync`, `io::r#async` (`Send` futures) and `io::local`, generated
from one source. They are generic over a `hadris-storage` block device and
borrow a caller-lent `BlockBuf` of one device block, so they still need no
allocator:

- `read_geometry` and `read_fat` read the boot and FSInfo sectors into a
  `Fat`, which tracks the free count and allocation hint
- `get`, `set` and `mirror` read and write FAT entries on every copy, the
  active copy first, recording an entry until every copy has it
- `next`, `walk` and `run` follow chains; `allocate`, `allocate_run` and
  `free_chain` take and free clusters a device block of entries at a time,
  recording their progress in a `Held` so a driver can finish an
  interrupted one
- `slot_offset` with a `DirWalk` that keeps its chain position,
  `read_slot`, `write_slots` and `clear_slots` for directories
- `mkfs` writes a volume that `layout::plan` planned
- `check` checks an unmounted volume without changing it, passing each
  `hadris_fs::Finding` to a callback, with a caller-lent scratch buffer for
  the paths of findings and a window of the cluster bitmap

`exfat::io` does the same for exFAT: `read_boot` and `read_volume` mount
the boot region and system structures into an `ExFat` state and an
`Upcase` index that decodes the up-case table lazily; `get`, `next` and
`set` read and write FAT entries; `bit`, `set_bit`, `allocate`,
`allocate_run` and `free_chain` work on the Allocation Bitmaps, a device
block at a time; `slot_offset` with a `DirWalk` finds directory entries;
`write_set` writes an entry set with its secondary entries first; the
first write sets `VolumeDirty`; and `check` checks an unmounted volume.

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
| `sync` | The device primitives in `io::sync` | - |
| `async` | The device primitives with `Send` futures in `io::r#async`, and without the `Send` bound in `io::local` | - |
| `defmt` | `defmt::Format` for `FatKind` | - |

## Documentation

- [API reference](https://docs.rs/hadris-fat-raw)
- [`hadris-fat`](../hadris-fat), the drivers built on this crate

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
