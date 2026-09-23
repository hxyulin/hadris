---
title: Build a CPIO initramfs
---

# Build a CPIO initramfs

```toml
[dependencies]
hadris-cpio = "2.4.0"
hadris-io = "2.4.0"
```

```rust
use hadris_cpio::{CpioArchiveWriter, CpioWriteOptions, FileTree};
use hadris_io::StdIo;
use std::{fs::File, io::BufWriter, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tree = FileTree::from_fs(Path::new("./initramfs-root"))?;
    let output = StdIo::new(BufWriter::new(File::create("initramfs.cpio")?));
    CpioArchiveWriter::new(output, CpioWriteOptions::default()).finish(&tree)?;
    Ok(())
}
```

Hadris writes the newc/SVR4 format used by Linux initramfs images. The reader
also supports allocation-free entry iteration for constrained consumers.

Inspect the result with an independent implementation before booting it:

```bash
cpio -itv < initramfs.cpio
```

See [Read and create CPIO archives](./cpio-archives.md) for streaming entry and
payload handling.
