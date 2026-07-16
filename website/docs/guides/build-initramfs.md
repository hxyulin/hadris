---
title: Build a CPIO initramfs
---

# Build a CPIO initramfs

```toml
[dependencies]
hadris-cpio = "2.0.0-rc.1"
```

```rust
use hadris_cpio::{CpioArchiveWriter, CpioWriteOptions, FileTree};
use std::{fs::File, io::BufWriter, path::Path};

fn main() -> hadris_cpio::Result<()> {
    let tree = FileTree::from_fs(Path::new("./initramfs-root"))?;
    let output = BufWriter::new(File::create("initramfs.cpio")?);
    CpioArchiveWriter::new(output, CpioWriteOptions::default()).finish(&tree)?;
    Ok(())
}
```

Hadris writes the newc/SVR4 format used by Linux initramfs images. The reader
also supports allocation-free entry iteration for constrained consumers.
