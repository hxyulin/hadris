---
title: Create FAT filesystems
---

# Create FAT filesystems

`hadris-fat` formats FAT12, FAT16, and FAT32 volumes and returns the new
filesystem mounted as a `FatFs`, ready for mutation. It works with any
`hadris-storage` block device: files, memory buffers, partition slices, and
custom devices. `FatFs` and `format` need `alloc`, which the default `std`
feature enables.

## Dependency

```toml
[dependencies]
hadris-fat = "2.4.0"   # default features: std, sync, write
hadris-fs = "2.4.0"
```

`format` is behind the `write` feature. Long file names are always supported.

## Format an image file

`format(&mut dev, &options)` lays out a volume and returns its `Geometry`;
mount it afterwards with the `MountOptions` of your choice. The volume fills
the device unless `with_size` asks for another size. Without `with_kind`,
volumes below 16 MiB are FAT12, below 512 MiB FAT16, and larger ones FAT32;
use `with_kind` when the variant is part of an external contract.

```rust,no_run
use std::fs::OpenOptions;

use hadris_fat::sync::{FatFs, format};
use hadris_fat::{FatKind, FatOptions, VolumeLabel};
use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;
use hadris_storage::host::FileDevice;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const SIZE: u64 = 64 * 1024 * 1024;

    let image = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("disk.img")?;
    image.set_len(SIZE)?;

    let options = FatOptions::new()
        .with_kind(FatKind::Fat16)
        .with_label(VolumeLabel::new("HADRIS")?);

    let mut dev = FileDevice::new(image)?;
    format(&mut dev, &options)?;
    let mut fs = FatFs::mount(dev, MountOptions::new())?;
    let mut buf = [0u8; 64];
    assert_eq!(fs.label(&mut buf)?, Some("HADRIS"));
    Ok(())
}
```

A device too small for the size or variant fails with `ErrorKind::NoSpace`,
one too large with `ErrorKind::LimitExceeded`, and an invalid option with
`ErrorKind::InvalidInput` before anything is written. `with_cluster_size`,
`with_sector_size`, `with_serial`, `with_oem_name`, `with_fat_count`,
`with_root_entries`, `with_alignment` and the other `with_*` methods set the
remaining boot sector fields. The time (`with_time`, 1980-01-01 by default)
stamps the label, and the serial derives from `with_seed` or the time, so
the same options produce the same bytes on every run. `write`, which formats
and copies a tree in, also mixes the tree's paths, sizes and times into the
serial.

## Create directories and files

The mounted `FatFs` implements the `hadris-fs` `FileSystem` trait, so a
`Volume` gives it paths and file handles. Call `sync` before closing the
device so file sizes and the FAT32 free count reach the disk.

```rust
use std::io::Write;

use hadris_fat::sync::FatFs;
use hadris_fs::OpenOptions;
use hadris_fs::sync::Volume;
use hadris_storage::sync::BlockDevice;

fn populate<D: BlockDevice>(vol: &Volume<FatFs<D>>) -> Result<(), Box<dyn std::error::Error>> {
    vol.create_dir_all("/DOCS")?;
    let create = OpenOptions::new().write().create().truncate();
    let mut file = vol.open("/DOCS/README.TXT", create)?;
    file.write_all(b"Created by Hadris\r\n")?;
    file.close()?;
    let mut file = vol.open("/DOCS/A long file name.txt", create)?;
    file.write_all(b"long names are always on")?;
    file.close()?;
    vol.lock().sync()?;
    Ok(())
}
```

Names that fit FAT's short-name rules are stored as 8.3 entries, including
the standard lowercase case flags; other names get long-name entries.

## Build an image from a tree

`write(dev, &tree, &options)` formats the device and copies a
`hadris_fs::Tree` into it. `hadris_fs::host::read_tree` builds the tree
from a host directory. Nodes without times get the options' time, and the
report lists what FAT cannot store, such as symlinks and permissions. A
growable device such as `Vec<u8>` starts empty, so give it a size.

```rust
use hadris_fat::FatOptions;
use hadris_fat::sync::write;
use hadris_fs::{Content, Node, Tree};

let mut tree = Tree::new();
tree.insert("DOCS/README.TXT", Node::file(Content::bytes("hello")))?;
let mut image = Vec::new();
let report = write(&mut image, &tree, &FatOptions::new().with_size(4 << 20))?;
assert_eq!(image.len() as u64, report.size());
# Ok::<(), hadris_fs::PathError>(())
```

The same `format` and `write` exist in `hadris_fat::r#async` when the crate
is built with `async`. `format` needs no allocator. Enable exactly the I/O
mode your application uses; `std` does not implicitly select `sync`.

## Format exFAT

exFAT has its own driver, `ExFatFs`, with the same node API, and its own
`ExFatOptions`, `VolumeLabel`, `format` and `write` in `hadris_fat::exfat`.
Labels keep their case and may use up to 11 UTF-16 code units.

```rust
use hadris_fat::exfat::sync::{ExFatFs, check, format};
use hadris_fat::exfat::{ExFatOptions, VolumeLabel};
use hadris_fs::sync::Volume;
use hadris_fs::{MountOptions, OpenOptions};
use hadris_storage::{BlockSize, MemDevice};

let mut dev = MemDevice::new(vec![0u8; 16 << 20], BlockSize::new(512).unwrap());
let label = VolumeLabel::new("Photos").unwrap();
format(&mut dev, &ExFatOptions::new().with_label(label))?;
assert!(check(&mut dev, &mut [0u8; 4096], |_| {})?.is_clean());
let vol = Volume::new(ExFatFs::mount(dev, MountOptions::new())?);
let mut file = vol.open("/hello.txt", OpenOptions::new().write().create())?;
file.write(b"hello")?;
file.close()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`ExFatOptions::with_fat_count(2)` formats a TexFAT volume. From the command
line, `hadris fat create ./contents -o card.img --fat-type exfat` does the
same for a host directory.

## Format a partition rather than a whole disk

Create or read the partition table with `hadris-part` (`DiskLayout` and
`hadris_part::sync::create`, or `hadris_part::sync::read`), restrict the disk
to the partition with `hadris_part::sync::open`, and pass that slice to
`format`. The formatter sees block zero relative to the partition, cannot
write outside it, and records the partition's start as the hidden sectors
(the exFAT `PartitionOffset`) unless `with_partition_offset` overrides it.

## Validate the result

```rust
use hadris_fat::sync::check;

# fn validate<D: hadris_storage::sync::BlockDevice>(dev: &mut D) -> hadris_fs::FsResult<(), D::Error> {
let mut scratch = [0u8; 4096];
let report = check(dev, &mut scratch, |finding| eprintln!("{finding}"))?;
assert!(report.is_clean());
# Ok(())
# }
```

`check` reads an unmounted device; call `unmount` or `into_inner` on a
`FatFs` first.

```bash
fsck.fat -vn disk.img
7z l disk.img
hadris fat check disk.img
```

For exFAT, `fsck.exfat -n` from exfatprogs and macOS `fsck_exfat -n` on an
attached raw device check the image, and `hadris fat check` runs the exFAT
checker.

Use read-only validation first. Do not allow a repair tool to modify a release
artifact until its original image has been preserved.
