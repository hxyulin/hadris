---
title: Read and extract UDF
---

# Read and extract files from UDF

```toml
[dependencies]
hadris-udf = "2.0.0"
```

The UDF reader exposes owned directory metadata and reads a selected file into
a byte vector.

```rust,no_run
use hadris_udf::UdfVolume;
use std::{fs, fs::File};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let volume = UdfVolume::open(File::open("disc.udf")?)?;
    println!("volume: {}", volume.info().volume_id);

    let root = volume.root_dir()?;
    for entry in root.entries() {
        let kind = if entry.is_dir() { "dir " } else { "file" };
        println!("{kind} {:>10} {}", entry.size, entry.name());
    }

    let entry = root.find("README.TXT").ok_or("README.TXT not found")?;
    let contents = volume.read_file(entry)?;
    fs::write("README.TXT", contents)?;

    Ok(())
}
```

`read_file` rejects directory entries and validates the file's allocation
descriptors before returning data. Do not join an untrusted on-disk filename
directly to an extraction directory; reject absolute paths and parent
components first.

For an unknown ISO/UDF image, open through `hadris-optical` so bridge-image
selection is explicit. The `hadris-udf` CLI provides recursive listing and
extraction for hosted workflows.
