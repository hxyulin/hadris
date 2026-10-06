---
title: Modify FAT safely
---

# Modify a FAT filesystem safely

Open writable media with both read and write access, operate through the
filesystem handle, and call `sync` before the backing device is removed.

```toml
[dependencies.hadris]
version = "3.0.0-rc.1"
git = "https://github.com/hxyulin/hadris"
branch = "main"
default-features = false
features = ["std", "sync", "fat"]
```

```rust,no_run
use std::io::Write;

use hadris::fat::sync::FatFs;
use hadris::fs::sync::{FileSystem, Volume};
use hadris::fs::{MountOptions, OpenOptions, SystemClock};
use hadris::storage::host::FileDevice;
use hadris::storage::sync::Cache;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("disk.img")?;
    let device = Cache::new(FileDevice::new(image)?, 64);
    let fs = FatFs::mount(device, MountOptions::new().with_clock(&SystemClock))?;
    let vol = Volume::new(fs);

    let mut file = vol.open("/hello.txt", OpenOptions::new().write().create().truncate())?;
    file.write_all(b"Hello from Hadris\n")?;
    file.close()?;

    vol.lock().sync()?;
    Ok(())
}
```

`sync` writes the sizes and modification times still held for open files,
the FAT32 free count, and flushes the device, so the `Cache` writes back its
dirty blocks. `Cache<D>` from `hadris-storage` replaces the old FAT sector
cache: it caches any block device and costs nothing unless constructed.

Writes are ordered so that an interrupted operation leaves a volume that
`fsck` repairs: at worst lost clusters, a chain longer than its file, or a
renamed node under both names. `hadris::fat::sync::check` reports exactly those
leftovers without changing the volume.

## Mutation checklist

- Close `File` handles (or call `close` on the node) so sizes reach the
  directory entry; `sync` writes all of them. Closing does not flush the
  device: call `File::sync_all` (`fsync`) or `sync` for durability.
- Removing a file that is still open fails with `ErrorKind::Busy`; close it
  first. A node you only looked up does not block removal; its id answers
  `ErrorKind::NotFound` afterwards until you `forget` it.
- Call `sync` after writes and before ejecting or closing removable media.
- Do not mutate an image concurrently through another handle. To share one
  volume between threads, clone the `hadris::fs::sync::Volume`.
- `FatFs::unmount` syncs and gives the device back.
- Validate important generated images with `check` and an independent
  implementation.

For deterministic images, keep the default `NoClock` (which stamps
1980-01-01) or supply your own `hadris::fs::Clock` through
`MountOptions::with_clock` instead of relying on the host clock.
