---
title: Detect and open images
---

# Detect and open unknown images

Use the `hadris` umbrella crate when the input format is not known in
advance. Detection is non-destructive: it only reads identifying metadata and
never writes. Opening performs the format's full validation.

```toml
[dependencies]
hadris = "2.4.0"
```

The default features include `detect`, which adds `fat`, `iso`, `udf` and
`cpio`, so what detection recognizes never depends on the features enabled.

## Detect

```rust,no_run
use hadris::ImageFormat;
use hadris::host::FileDevice;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut image = FileDevice::open("disk.img")?;
    let found = hadris::sync::detect(&mut image)?;
    for candidate in found.iter() {
        match candidate.damage() {
            Some(err) => println!("{:?}, damaged: {err}", candidate.format()),
            None => println!("{:?}", candidate.format()),
        }
    }
    if matches!(found.first().map(|c| c.format()), Some(ImageFormat::Mbr | ImageFormat::Gpt)) {
        println!("partitioned disk: open one partition with hadris::part::sync::open");
    }
    Ok(())
}
```

`detect` takes any `hadris-storage` block device and returns a `Detection`
listing every format found, most specific first, without allocating. A
bridge image lists `IsoUdfBridge`, then `Iso`, then `Udf`; a hybrid ISO lists
`Iso`, then `Gpt` or `Mbr`. Each `Candidate` has its `ImageFormat` and, when
its signature is present but the structures a mount reads first are
damaged, the `Corrupt` error that mount would give, so a damaged volume never
reads as another format. An empty `Detection` means nothing was recognized.
The device's block size is the logical block size used to find a GPT header.

## Open

```rust,no_run
use hadris::fs::sync::Volume;
use hadris::sync::AnyFs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fs = hadris::host::open("disc.img")?;
    if let AnyFs::Udf(udf) = &fs {
        println!("UDF revision {:?}", udf.info().revision());
    }
    let vol = Volume::new(fs);
    for entry in vol.read_dir("/")? {
        println!("{:?}", entry?.name());
    }
    Ok(())
}
```

`hadris::host::open(path)` detects the image and mounts it read-only with
the host's clock and time zone. On any other device, `hadris::sync::open(dev,
options)` (or `hadris::r#async::open`) mounts the first filesystem `detect`
finds with the caller's `MountOptions`, and returns an `AnyFs`: `Fat`,
`ExFat`, `Iso` or `Udf`. `AnyFs` implements the `FileSystem` trait, so
`Volume` and its handles work on it, and a `match` reaches each driver's
extras. A bridge image opens as UDF; if its UDF side does not mount, it opens
as ISO 9660. Open `hadris::iso::sync::IsoFs` directly to choose an ISO
namespace.

A failed open returns the device in a `MountError`. NTFS volumes, partition
tables and archives fail with `ErrorKind::NotRecognized` (the messages are
`"ntfs"`, `"partition table"` and `"archive"`), and so does a device with no
known format. For a partitioned disk, select a partition with
`hadris::part::sync::open` and open the `Partition` it returns.

The [`volume-list` example](https://github.com/hxyulin/hadris/tree/next/examples/volume-list)
is a complete program: it detects the format, opens it, and prints the tree
with one function generic over the `FileSystem` trait.

## Detection is not validation

Detection answers "what does this look like?" using signatures and the first
structures of each format. Always open the image before trusting offsets,
sizes, or directory data. Treat an empty `Detection` as an unknown format
rather than as proof that the input is unformatted.
