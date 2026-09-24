---
title: Create FAT filesystems
---

# Create FAT filesystems

`hadris-fat` formats FAT12, FAT16, and FAT32 volumes and returns the new
filesystem mounted as a `FatFs`, ready for mutation. It works with any
`hadris-storage` block device: files, memory buffers, partition slices, and
custom devices. Formatting needs no allocator.

## Dependency

```toml
[dependencies]
hadris-fat = "2.4.0"   # default features: std, sync, write
hadris-fs = "2.4.0"
```

`format` is behind the `write` feature. Long file names are always supported.

## Format an image file

`format` fills the whole device, so the target must already have the desired
length. Without `with_kind`, volumes below 16 MiB are FAT12, below 512 MiB
FAT16, and larger ones FAT32; use `with_kind` when the variant is part of an
external contract.

```rust,no_run
use std::fs::OpenOptions;

use hadris_fat::sync::format;
use hadris_fat::{FatKind, FormatOptions, VolumeLabel};
use hadris_fs::SystemClock;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const SIZE: u64 = 64 * 1024 * 1024;

    let image = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("disk.img")?;
    image.set_len(SIZE)?;

    let options = FormatOptions::new()
        .with_kind(FatKind::Fat16)
        .with_label(VolumeLabel::new("HADRIS")?)
        .with_clock(SystemClock);

    let mut fs = format(image, options)?;
    assert_eq!(fs.label()?.map(|l| l.as_str().to_owned()).as_deref(), Some("HADRIS"));
    Ok(())
}
```

A device too small for the requested variant fails with `ErrorKind::NoSpace`,
one too large with `ErrorKind::LimitExceeded`, and an invalid option with
`ErrorKind::InvalidInput` before anything is written. `with_cluster_size`,
`with_sector_size`, `with_volume_id`, `with_oem_name`, `with_fat_count`,
`with_root_entries` and the other `with_*` methods set the remaining boot
sector fields. The default `NoClock` produces the same bytes on every run.

## Create directories and files

The mounted `FatFs` supports the `hadris-fs` path helpers. Call `sync` before
closing the device so file sizes and the FAT32 free count reach the disk.

```rust
use hadris_fat::sync::FatFs;
use hadris_fs::sync::DriverExt;
use hadris_storage::sync::BlockDevice;

fn populate<D: BlockDevice>(fs: &mut FatFs<D>) -> hadris_fs::FsResult<(), D::Error> {
    fs.create_dir_all("/DOCS")?;
    fs.write_file("/DOCS/README.TXT", b"Created by Hadris\r\n")?;
    fs.write_file("/DOCS/A long file name.txt", b"long names are always on")?;
    fs.sync()
}
```

To copy a host directory tree into the image, use
`hadris_fs::sync::import_from_host("./contents", &mut fs, "/")`. Names that fit
FAT's short-name rules are stored as 8.3 entries, including the standard
lowercase case flags; other names get long-name entries.

## In-memory and async formatting

For tests, format a byte buffer:

```rust
use hadris_fat::FormatOptions;
use hadris_fat::sync::format;
use hadris_storage::{BlockSize, MemDevice};

let dev = MemDevice::new(vec![0_u8; 4 * 1024 * 1024], BlockSize::new(512).unwrap());
let fs = format(dev, FormatOptions::new())?;
let bytes: Vec<u8> = fs.into_inner().into_inner();
# Ok::<(), hadris_fs::Error<core::convert::Infallible>>(())
```

The same `format` exists in `hadris_fat::r#async` and `hadris_fat::async_send`
when the crate is built with `async` or `async-send`. Enable exactly the I/O
mode your application uses; `std` does not implicitly select `sync`.

## Format exFAT

exFAT has its own driver, `ExFatFs`, with the same node API, and its own
`FormatOptions`, `VolumeLabel` and `format` in `hadris_fat::exfat`. Labels
keep their case and may use up to 11 UTF-16 code units.

```rust
use hadris_fat::exfat::sync::{check, format};
use hadris_fat::exfat::{FormatOptions, VolumeLabel};
use hadris_fs::sync::{PathExt, Volume};
use hadris_storage::{BlockSize, MemDevice};

let dev = MemDevice::new(vec![0u8; 16 << 20], BlockSize::new(512).unwrap());
let label = VolumeLabel::new("Photos").unwrap();
let mut fs = format(dev, FormatOptions::new().with_label(label))?;
assert!(check(&mut fs)?.is_clean());
let vol = Volume::new(fs);
vol.write_file("/hello.txt", b"hello")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`FormatOptions::with_fat_count(2)` formats a TexFAT volume. From the command
line, `hadris-fat create ./contents -o card.img --fat-type exfat` does the
same for a host directory.

## Format a partition rather than a whole disk

Create or read the partition table with `hadris-part` (`DiskLayout` and
`hadris_part::sync::create`, or `hadris_part::sync::read`), restrict the disk
to the partition with `hadris_part::sync::open`, and pass that slice to
`format`. The formatter sees block zero relative to the partition and
cannot write outside it.

## Validate the result

```rust
use hadris_fat::sync::check;

# fn validate<D: hadris_storage::sync::BlockDevice>(fs: &mut hadris_fat::sync::FatFs<D>) -> hadris_fs::FsResult<(), D::Error> {
let report = check(fs)?;
assert!(report.is_clean());
# Ok(())
# }
```

```bash
fsck.fat -vn disk.img
7z l disk.img
hadris-fat verify disk.img
```

For exFAT, `fsck.exfat -n` from exfatprogs and macOS `fsck_exfat -n` on an
attached raw device check the image, and `hadris-fat verify` runs the exFAT
checker.

Use read-only validation first. Do not allow a repair tool to modify a release
artifact until its original image has been preserved.
