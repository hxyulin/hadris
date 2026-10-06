---
title: Build a CPIO initramfs
---

# Build a CPIO initramfs

```toml
[dependencies]
hadris-cpio = "3.0.0-rc.1"
hadris-fs = "3.0.0-rc.1"
hadris-io = "3.0.0-rc.1"
```

```rust,no_run
use hadris_cpio::CpioOptions;
use hadris_fs::Owner;
use hadris_fs::host::{self, TreeOptions};
use hadris_io::StdIo;
use std::{fs::File, io::BufWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = TreeOptions::new().with_owner(Owner::new(0, 0));
    let (tree, _) = host::read_tree("./initramfs-root", &options)?;
    let mut output = StdIo::new(BufWriter::new(File::create("initramfs.cpio")?));
    hadris_cpio::sync::write(&mut output, &tree, &CpioOptions::default())?;
    Ok(())
}
```

Hadris writes the newc/SVR4 format used by Linux initramfs images. Device
nodes, symlinks and hard links in the source tree are stored; hard link
groups carry their data on the last name, as GNU cpio writes them.
`with_owner` makes every entry root-owned, as an unprivileged build needs.
For a reproducible archive, pass `host::source_date_epoch()` to
`CpioOptions::with_time` and `TreeOptions::with_clamp`. The reader iterates
entries without an allocator for constrained consumers.

Inspect the result with an independent implementation before booting it:

```bash
cpio -itv < initramfs.cpio
```

See [Read and create CPIO archives](./cpio-archives.md) for streaming entry and
payload handling.
