---
title: Modify FAT safely
---

# Modify a FAT filesystem safely

Open writable media with both read and write access, operate through the
filesystem handle, finish every file writer, and flush the volume before the
backing device is removed.

```toml
[dependencies]
hadris-fat = { version = "2.0.0", features = ["cache", "dirty-file-panic"] }
```

```rust,no_run
use hadris_fat::{FatVolume, FatVolumeWriteExt};
use std::{fs::OpenOptions, io::Write};

fn main() -> hadris_fat::Result<()> {
    let image = OpenOptions::new()
        .read(true)
        .write(true)
        .open("disk.img")?;
    let volume = FatVolume::builder(image).fat_cache(16).open()?;

    let root = volume.root_dir();
    let entry = match root.find("hello.txt")? {
        Some(entry) => entry,
        None => volume.create_file(&root, "hello.txt")?,
    };

    let mut writer = volume.write_file(&entry)?;
    writer.write_all(b"Hello from Hadris\n")?;
    writer.finish()?;

    volume.flush()?;
    Ok(())
}
```

`FileWriter::finish` commits the directory entry's size and timestamps. Dropping
an unfinished writer can leave written clusters with stale metadata; the
`dirty-file-panic` feature turns that mistake into a development-time panic.

## Mutation checklist

- Keep only one writer for an entry at a time.
- Re-find an entry after rename, deletion, or other metadata changes.
- Call `finish` on file writers even when writing an empty payload.
- Call `flush` after cached writes and before ejecting or closing removable
  media.
- Do not mutate an image concurrently through another handle.
- Validate important generated images with an independent implementation.

For deterministic images, install a custom time provider or patch entry times
explicitly instead of relying on the host clock.
