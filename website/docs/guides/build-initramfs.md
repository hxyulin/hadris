---
title: Build a CPIO initramfs
---

# Build a CPIO initramfs

```toml
[dependencies]
hadris-cpio = "2.4.0"
hadris-fs = "2.4.0"
hadris-io = "2.4.0"
```

```rust,no_run
use hadris_cpio::CpioOptions;
use hadris_fs::tree::{FromFsOptions, Tree};
use hadris_io::StdIo;
use std::{fs::File, io::BufWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tree = Tree::from_fs("./initramfs-root", FromFsOptions::new())?;
    let mut output = StdIo::new(BufWriter::new(File::create("initramfs.cpio")?));
    hadris_cpio::sync::write(&mut output, &tree, &CpioOptions::default())?;
    Ok(())
}
```

Hadris writes the newc/SVR4 format used by Linux initramfs images. Device
nodes, symlinks and hard links in the source tree are stored; hard link
groups carry their data on the last name, as GNU cpio writes them. The
reader iterates entries without an allocator for constrained consumers.

Inspect the result with an independent implementation before booting it:

```bash
cpio -itv < initramfs.cpio
```

See [Read and create CPIO archives](./cpio-archives.md) for streaming entry and
payload handling.
