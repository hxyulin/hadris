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
```

```rust,no_run
use hadris_block::{detect, sync::OpenVolume};
use std::fs::File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut image = File::open("disk.img")?;
    let format = detect::sync::detect(&mut image)?;
    println!("detected: {format:?}");

    match format {
        Some(detect::BlockFormat::Fat(_)) => {
            let opened = OpenVolume::open(&mut image)?;
            println!("FAT variant: {:?}", opened.format());
            let fat = opened.into_fat().ok().expect("the detector reported FAT");
            println!("FAT kind: {:?}", fat.kind());
        }
        Some(detect::BlockFormat::PartitionTable(kind)) => {
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
the logical block size used to find a GPT header. The opened FAT volume is a
`hadris_fat::sync::FatFs`, which works with the `hadris-fs` path helpers.
Errors are `hadris_block::Error<E>`, carrying the device's error type.

`OpenVolume` intentionally refuses a whole partitioned disk. Select a partition
and restrict the device to it before opening its filesystem.

## Optical images

```toml
[dependencies]
hadris-io = "2.4.0"
hadris-optical = "2.4.0"
```

```rust,no_run
use hadris_io::StdIo;
use hadris_optical::{OpenPolicy, sync::OpenOpticalImage};
use std::fs::File;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut image = StdIo::new(File::open("disc.img")?);
    let opened = OpenOpticalImage::open(&mut image, OpenPolicy::PreferUdf)?;

    if let Some(udf) = opened.as_udf() {
        println!("UDF volume: {}", udf.info().volume_id);
    } else if opened.as_iso9660().is_some() {
        println!("ISO 9660 image");
    }

    Ok(())
}
```

Bridge images can contain valid ISO 9660 and UDF filesystems simultaneously.
Use `PreferUdf` or `PreferIso9660` for fallback behavior, and `Udf` or
`Iso9660` when the requested format is mandatory.

## Detection is not validation

Detection answers “what does this look like?” using signatures and geometry.
Always open the returned concrete format before trusting offsets, sizes, or
directory data. Treat `None` as an unknown format rather than as proof that the
input is unformatted.
