---
title: Read a FAT image
---

# Read files from a FAT image

Use `hadris-fat` directly when the image is known to contain a standalone FAT
filesystem.

```toml
[dependencies]
anyhow = "1"
hadris-fat = "2.4.0"
hadris-fs = "2.4.0"
```

```rust,no_run
use anyhow::{Context, Result};
use hadris_fat::MountOptions;
use hadris_fat::sync::FatFs;
use hadris_fs::sync::DriverExt;
use std::fs::File;

fn main() -> Result<()> {
    let image = File::open("disk.img").context("open disk.img")?;
    let mut volume = FatFs::open_with(image, MountOptions::new().with_read_only(true))
        .context("open FAT filesystem")?;

    let mut names = Vec::new();
    for entry in volume.read_dir("/").context("open root directory")? {
        let entry = entry.context("read directory entry")?;
        names.push(entry.name_str().context("name is not UTF-8")?.to_owned());
    }
    for name in names {
        let meta = volume.metadata(&format!("/{name}"))?;
        let kind = if meta.file_type().is_dir() { "dir " } else { "file" };
        println!("{kind} {:>10} {name}", meta.len());
    }

    if volume.exists("/README.TXT")? {
        let contents = volume.read_to_vec("/README.TXT")?;
        println!("{}", String::from_utf8_lossy(&contents));
    }

    Ok(())
}
```

`std::fs::File` is a block device with 512-byte blocks; `FatFs` also opens any
other `hadris-storage` device, such as a `MemDevice` over bytes already in
memory. Lookups ignore case, and long names are always read. Streams opened
with `volume.open(path, hadris_fs::OpenOptions::read())` implement
`std::io::Read` and `Seek`, and `hadris_fs::sync::extract_to_host(&mut volume,
"/", "out")` copies the whole tree to the host.

Directory iteration surfaces malformed entries and I/O failures as errors; do
not discard them with `while let Some(Ok(...))` in production code.

Mount without `with_read_only` when the same image must be modified. For
a partitioned disk, follow [Open FAT inside a partition](./open-partitioned-fat.md)
instead of opening the whole disk as a filesystem.
