---
title: Open FAT inside a partition
---

# Open FAT inside an MBR or GPT partition

A partition table and the filesystem inside it are separate layers. Read the
table, choose a partition, then restrict all filesystem I/O to that partition's
block range.

```toml
[dependencies]
anyhow = "1"
hadris = { version = "3.0.0-rc.1", features = ["part"] }
```

```rust,no_run
use anyhow::{Context, Result};
use hadris::fs::sync::Volume;
use hadris::host::FileDevice;
use hadris::part;
use hadris::sync::AnyFs;

fn main() -> Result<()> {
    let mut disk = FileDevice::open("disk.img")?;
    let table = part::sync::read(&mut disk)?;
    let partition = table.partition(0).context("the disk has no partitions")?;

    let slice = part::sync::open(&mut disk, &partition)?;
    // `MountError` holds the borrowed slice; keep only its error for `anyhow`.
    let opened = hadris::sync::open(slice, hadris::host::mount_options())
        .map_err(|err| err.into_error())?;
    let AnyFs::Fat(fat) = opened else {
        anyhow::bail!("the selected partition is not FAT");
    };

    let vol = Volume::new(fat);
    for entry in vol.read_dir("/")? {
        println!("{}", entry?.name().to_str().unwrap_or("?"));
    }

    Ok(())
}
```

`part::sync::open` returns a `hadris_storage` `Partition` of the disk: block 0 of
the partition is the partition's first block, and requests past its end fail
before they reach the disk. `part::r#async::open` does the same for async
devices, and the table is
read with `part::r#async::read` there.

Do not seek to the partition offset and then pass the unrestricted disk handle
to a filesystem parser. Filesystem offsets are relative to its start, and an
unbounded handle can allow corrupt metadata to address neighboring partitions.

The table is read with the disk device's block size, which must be the
logical block size the table was written with. The common value is 512
bytes; wrap a 4Kn image in a `StreamDevice` with 4096-byte blocks.
