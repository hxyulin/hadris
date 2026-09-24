---
title: Read and extract UDF
---

# Read and extract files from UDF

```toml
[dependencies]
hadris-fs = { version = "2.4.0", features = ["std", "sync"] }
hadris-udf = "2.4.0"
```

`UdfFs` opens a volume on any `hadris_storage` block device, such as a host
file, and implements the `hadris-fs` driver traits, so the path helpers,
handles and host helpers work on it.

```rust,no_run
use hadris_fs::sync::DriverExt;
use hadris_udf::sync::UdfFs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut udf = UdfFs::open(std::fs::File::open("disc.udf")?)?;
    println!("volume: {}", udf.logical_volume_id());

    for entry in udf.read_dir("/")? {
        let entry = entry?;
        let kind = if entry.file_type().is_dir() { "dir " } else { "file" };
        println!("{kind} {}", String::from_utf8_lossy(entry.name_bytes()));
    }

    std::fs::write("README.TXT", udf.read_to_vec("/README.TXT")?)?;
    hadris_fs::sync::extract_to_host(&mut udf, "/", "out")?;
    Ok(())
}
```

`extract_to_host` refuses names with separators or `..` components and never
writes through an existing host symlink, so an untrusted image cannot escape
the target directory.

For an unknown ISO/UDF image, open through `hadris-optical` so bridge-image
selection is explicit. The `hadris-udf` CLI provides listing and extraction
for hosted workflows.
