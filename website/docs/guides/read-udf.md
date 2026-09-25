---
title: Read and extract UDF
---

# Read and extract files from UDF

```toml
[dependencies]
hadris-fs = { version = "2.4.0", features = ["std", "sync"] }
hadris-udf = "2.4.0"
hadris-storage = "2.4.0"
```

`UdfFs` opens a volume on any `hadris_storage` block device, such as a host
file, and implements the `hadris-fs` `FileSystem` trait, so `Volume`, its
handles and the host helpers work on it.

```rust,no_run
use std::io::Read;

use hadris_fs::{MountOptions, OpenOptions};
use hadris_fs::sync::Volume;
use hadris_udf::UdfId;
use hadris_udf::sync::UdfFs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let udf = UdfFs::mount(hadris_storage::host::FileDevice::open("disc.udf")?, MountOptions::new())?;
    println!("volume: {}", udf.info().id(UdfId::LogicalVolume));
    let vol = Volume::new(udf);

    for entry in vol.read_dir("/")? {
        let entry = entry?;
        let kind = if entry.file_type().is_dir() { "dir " } else { "file" };
        println!("{kind} {}", String::from_utf8_lossy(entry.name().as_bytes()));
    }

    let mut readme = Vec::new();
    vol.open("/README.TXT", OpenOptions::new().read())?
        .read_to_end(&mut readme)?;
    std::fs::write("README.TXT", readme)?;
    let tree = hadris_fs::sync::read_tree(&vol, "/")?;
    hadris_fs::host::write_tree("out", &tree)?;
    Ok(())
}
```

`host::write_tree` refuses names with separators or `..` components and never
writes through an existing host symlink, so an untrusted image cannot escape
the target directory.

For an unknown ISO/UDF image, open through `hadris-optical` so bridge-image
selection is explicit. The `hadris-udf` CLI provides listing and extraction
for hosted workflows.
