---
title: Read a FAT image
---

# Read files from a FAT image

```toml
[dependencies]
hadris-fat = "2.0.0-rc.1"
```

```rust
use hadris_fat::{FatVolume, FatVolumeReadExt};
use std::fs::File;

fn main() -> hadris_fat::Result<()> {
    let image = File::open("disk.img")?;
    let volume = FatVolume::open(image)?;

    let root = volume.root_dir();
    let mut entries = root.entries();
    while let Some(Ok(entry)) = entries.next_entry() {
        println!("{} ({} bytes)", entry.name(), entry.len());
    }

    Ok(())
}
```

Use `OpenOptions` and the write extension traits when the same image must be
modified. For a partitioned disk, create a bounded partition view with
`hadris-block` before opening the FAT volume.
