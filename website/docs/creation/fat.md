---
title: Create FAT filesystems
---

# Create FAT filesystems

`hadris-fat` formats FAT12, FAT16, and FAT32 volumes and then opens the new
filesystem for mutation. It works with files, memory buffers, partition views,
and other targets implementing the selected Hadris I/O mode.

## Dependency

```toml
[dependencies]
hadris-fat = { version = "2.0.0-rc.3", features = ["write", "sync", "lfn"] }
```

## Format an image file

The target must already have the desired length. Automatic selection chooses a
FAT variant from the volume geometry; use `FatTypeSelection` when the format is
part of an external contract.

```rust
use std::fs::OpenOptions;

use hadris_fat::format::{FatTypeSelection, FatVolumeFormatter, FormatOptions};

fn main() -> hadris_fat::Result<()> {
    const SIZE: u64 = 64 * 1024 * 1024;

    let image = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("disk.img")?;
    image.set_len(SIZE)?;

    let options = FormatOptions::new(SIZE)
        .volume_label("HADRIS")
        .fat_type(FatTypeSelection::Fat16);

    let fs = FatVolumeFormatter::format(image, options)?;
    assert_eq!(fs.volume_info().volume_label(), "HADRIS");
    Ok(())
}
```

Common starting points are roughly 2 MiB for FAT12, 64 MiB for FAT16, and a
forced FAT32 selection for moderately sized test images. Always let
`calculate_params` validate the exact geometry instead of relying on a rule of
thumb.

## Preview the layout

```rust
use hadris_fat::format::{FatVolumeFormatter, FormatOptions};

let options = FormatOptions::new(64 * 1024 * 1024).volume_label("PREVIEW");
let params = FatVolumeFormatter::calculate_params(&options)?;
println!("FAT type: {:?}, clusters: {}", params.fat_type, params.cluster_count);
# Ok::<(), hadris_fat::Error>(())
```

This performs validation without writing the target.

## Create directories and files

Mutation methods are supplied by `FatFsWriteExt`. File writers must be finished
so directory size and cluster-chain metadata are committed.

```rust
use hadris_fat::FatFsWriteExt;

# fn populate<DATA>(fs: &hadris_fat::FatFs<DATA>) -> hadris_fat::Result<()>
# where DATA: hadris_fat::io::Read + hadris_fat::io::Write + hadris_fat::io::Seek {
let root = fs.root_dir();
let docs = fs.create_dir(&root, "DOCS")?;
let readme = fs.create_file(&docs, "README.TXT")?;

let mut writer = fs.write_file(&readme)?;
writer.write(b"Created by Hadris\r\n")?;
writer.finish()?;
# Ok(())
# }
```

Long filenames require the `lfn` feature. Names that fit FAT's short-name rules
are stored as 8.3 entries, including the standard lowercase case flags.

## In-memory and async formatting

For tests, format a fixed-size byte buffer:

```rust
use std::io::Cursor;
use hadris_fat::format::{FatVolumeFormatter, FormatOptions};

let mut bytes = vec![0_u8; 4 * 1024 * 1024];
let cursor = Cursor::new(bytes.as_mut_slice());
let fs = FatVolumeFormatter::format(cursor, FormatOptions::new(bytes.len() as u64))?;
# Ok::<(), hadris_fat::Error>(())
```

The same formatter name exists in `hadris_fat::r#async` when the crate is built
with `async`. Enable exactly the I/O mode your application uses; `std` does not
implicitly select `sync`.

## Format a partition rather than a whole disk

Create or open the partition table with `hadris-part`, obtain a bounded
partition view, and pass that view to `FatVolumeFormatter`. The formatter sees
sector zero relative to the partition and cannot write outside the view.

## Validate the result

```bash
fsck.fat -vn disk.img
7z l disk.img
hadris-fat info disk.img
```

Use read-only validation first. Do not allow a repair tool to modify a release
artifact until its original image has been preserved.
