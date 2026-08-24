---
title: Read a FAT image
---

# Read files from a FAT image

Use `hadris-fat` directly when the image is known to contain a standalone FAT
filesystem.

```toml
[dependencies]
anyhow = "1"
hadris-fat = "2.2.0"
```

```rust,no_run
use anyhow::{Context, Result};
use hadris_fat::{FatVolume, FatVolumeReadExt};
use std::fs::File;

fn main() -> Result<()> {
    let image = File::open("disk.img").context("open disk.img")?;
    let volume = FatVolume::open(image).context("open FAT filesystem")?;

    let root = volume.root_dir();
    let mut entries = root.entries();
    while let Some(entry) = entries.next_entry() {
        let entry = entry.context("read directory entry")?;
        let file = entry.as_entry().context("unsupported directory record")?;
        let kind = if file.is_directory() { "dir " } else { "file" };
        println!("{kind} {:>10} {}", file.len(), file.name());
    }

    if let Some(readme) = root.find("README.TXT")? {
        let mut reader = volume.read_file(&readme)?;
        let mut contents = Vec::new();
        std::io::Read::read_to_end(&mut reader, &mut contents)?;
        println!("{}", String::from_utf8_lossy(&contents));
    }

    Ok(())
}
```

Directory iteration surfaces malformed entries and I/O failures as errors; do
not discard them with `while let Some(Ok(...))` in production code.

Use `OpenOptions` and the write APIs when the same image must be modified. For
a partitioned disk, follow [Open FAT inside a partition](./open-partitioned-fat.md)
instead of opening the whole disk as a filesystem.
