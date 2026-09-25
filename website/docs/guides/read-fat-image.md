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
hadris-storage = "2.4.0"
```

```rust,no_run
use std::io::Read;

use anyhow::{Context, Result};
use hadris_fat::sync::FatFs;
use hadris_fs::sync::Volume;
use hadris_fs::{ErrorKind, MountOptions, OpenOptions};
use hadris_storage::host::FileDevice;

fn main() -> Result<()> {
    let image = FileDevice::open("disk.img").context("open disk.img")?;
    let fs = FatFs::mount(image, MountOptions::new().read_only())
        .context("open FAT filesystem")?;
    let vol = Volume::new(fs);

    for entry in vol.read_dir("/").context("open root directory")? {
        let entry = entry.context("read directory entry")?;
        let name = entry.name().to_str().context("name is not UTF-8")?;
        let meta = entry.metadata();
        let kind = if meta.file_type().is_dir() { "dir " } else { "file" };
        println!("{kind} {:>10} {name}", meta.len());
    }

    match vol.open("/README.TXT", OpenOptions::new().read()) {
        Ok(mut file) => {
            let mut contents = Vec::new();
            file.read_to_end(&mut contents)?;
            println!("{}", String::from_utf8_lossy(&contents));
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    Ok(())
}
```

`host::FileDevice` is a block device with 512-byte blocks; `FatFs` also opens any
other `hadris-storage` device, such as a `MemDevice` over bytes already in
memory. Lookups ignore case, and long names are always read. Files opened
with `vol.open(path, OpenOptions::new().read())` implement `std::io::Read`
and `Seek`, and `hadris_fs::sync::read_tree(&vol, "/")` followed by
`hadris_fs::host::write_tree("out", &tree)` copies the whole tree to the host. Short names are read in CP437
unless `MountOptions::with_code_page` names another code page.

Directory iteration surfaces malformed entries and I/O failures as errors; do
not discard them with `while let Some(Ok(...))` in production code.

Mount without `read_only` when the same image must be modified. For
a partitioned disk, follow [Open FAT inside a partition](./open-partitioned-fat.md)
instead of opening the whole disk as a filesystem.
