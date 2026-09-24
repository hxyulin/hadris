---
title: Detect and open images
---

# Detect and open unknown images

Use category facades when the input format is not known in advance. Detection
is non-destructive: it only reads identifying metadata. Opening performs the
format's full validation.

## Block images

```toml
[dependencies]
hadris-block = "2.4.0"
hadris-fs = "2.4.0"
```

```rust,no_run
use hadris_block::detect::BlockFormat;
use hadris_block::sync::OpenVolume;
use hadris_fs::sync::DriverExt;
use std::fs::File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut image = File::open("disk.img")?;
    let format = hadris_block::detect::sync::detect(&mut image)?;
    println!("detected: {format:?}");

    match format {
        Some(BlockFormat::Fat(_) | BlockFormat::Ntfs) => {
            let mut volume = OpenVolume::open(image)?;
            println!("opened {:?}", volume.format());
            for entry in volume.read_dir("/")? {
                println!("{:?}", entry?.name());
            }
        }
        Some(BlockFormat::PartitionTable(kind)) => {
            println!("partitioned disk: {kind:?}");
        }
        Some(other) => println!("other block format: {other:?}"),
        None => println!("no supported block format detected"),
    }

    Ok(())
}
```

Detection and `OpenVolume` take any `hadris-storage` block device; a
`std::fs::File` is one with 512-byte blocks, and the device's block size is
the logical block size used to find a GPT header. `OpenVolume` opens
FAT12/16/32 as `hadris_fat`'s `FatFs`, exFAT as its `ExFatFs`, and NTFS,
read-only, as `hadris_ntfs`'s `NtfsFs`, and implements the `hadris-fs` driver
trait over each, so the path helpers work on the result. `as_fat`,
`into_fat`, `as_exfat` and `into_exfat` reach the FAT and exFAT drivers; the
`unstable-ntfs` feature adds `as_ntfs` and `into_ntfs`. Errors are
`hadris_fs::Error<E>`, carrying the device's error type, and a failed open
returns the device in a `MountError`. A device with no known format fails
with `ErrorKind::NotRecognized`.

The [`volume-list` example](https://github.com/hxyulin/hadris/tree/next/examples/volume-list)
is a complete program: it detects the format, opens it, and prints the tree
with one function generic over the driver trait.

`OpenVolume` intentionally refuses a whole partitioned disk. Select a partition
and restrict the device to it before opening its filesystem.

## Optical images

```toml
[dependencies]
hadris-fs = "2.4.0"
hadris-optical = "2.4.0"
```

```rust,no_run
use hadris_fs::sync::DriverExt;
use hadris_optical::{OpenPolicy, sync::OpenOpticalImage};
use std::fs::File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image = File::open("disc.img")?;
    let mut opened = OpenOpticalImage::open(image, OpenPolicy::PreferUdf)?;

    if let Some(udf) = opened.as_udf() {
        println!("UDF volume: {}", udf.volume_id());
    } else if opened.as_iso().is_some() {
        println!("ISO 9660 image");
    }
    for entry in opened.read_dir("/")? {
        println!("{:?}", entry?.name());
    }

    Ok(())
}
```

Bridge images can contain valid ISO 9660 and UDF filesystems simultaneously.
Use `PreferUdf` or `PreferIso9660` for fallback behavior, and `Udf` or
`Iso9660` when the requested format is mandatory. ISO 9660 opens with the
preferred namespace; open `hadris-iso` directly to choose another.

## Detection is not validation

Detection answers "what does this look like?" using signatures and geometry.
Always open the returned concrete format before trusting offsets, sizes, or
directory data. Treat `None` as an unknown format rather than as proof that the
input is unformatted.
