---
title: Create UDF filesystems
---

# Create UDF filesystems

`hadris-udf` writes mastered, read-only type 1 UDF volumes from a
`hadris_fs::tree::Tree`. Use `hadris-cd` for a shared ISO/UDF bridge image.

## Dependency

```toml
[dependencies]
hadris-fs = { version = "2.4.0", features = ["std", "sync"] }
hadris-udf = "2.4.0"
```

## Create a volume

```rust,no_run
use hadris_fs::tree::{Content, Tree};
use hadris_udf::UdfOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = Tree::new();
    tree.add_file("README.txt", Content::bytes("Hello from a UDF image\n"))?;
    tree.add_file("docs/guide.txt", Content::bytes("UDF guide\n"))?;

    let mut target = std::fs::File::create("volume.udf")?;
    let report = hadris_udf::sync::write(&mut target, &tree, &UdfOptions::default())?;
    println!("wrote {} blocks", report.total_blocks());
    Ok(())
}
```

A host file grows as the writer writes it. For a fixed-size device such as a
`MemDevice`, size it with `hadris_udf::sync::plan(&tree, &options)` first.
`Tree::from_fs` imports a host directory without reading the files until the
image is written.

## Select a mastered revision

```rust
use hadris_udf::{UdfOptions, UdfRevision};

let options = UdfOptions::default()
    .with_volume_id("ARCHIVE_2026")
    .with_revision(UdfRevision::V2_01);
```

The revision describes a mastered, read-only image. It does not enable packet
writing, VAT, sparing, metadata partitions or pseudo-overwrite. Choose the
oldest revision that gives your consumers what they need, and validate with
their tools.

## Names, metadata and limits

Names are encoded as OSTA Compressed Unicode: 8-bit when every character is
below U+0100, 16-bit otherwise. A name over 254 encoded bytes fails with
`NameTooLong`. Symlinks and hard links are stored; device nodes, creation
times and DOS attributes are left out and listed in `Report::warnings`. The
default `NoClock` dates entries without times 1980-01-01, so the same tree
gives the same bytes; `with_clock(SystemClock)` uses the current time.

## Author an ISO/UDF bridge

Do not concatenate separate ISO and UDF images. A bridge coordinates
descriptor locations, directory ICBs and payload extents. Use the `hadris-cd`
crate or CLI:

```bash
hadris-cd create image-root -o bridge.iso
hadris-cd verify bridge.iso
```

## Validate the result

```bash
udfinfo volume.udf
7z l volume.udf
hadris-udf info volume.udf
```

7-Zip does not open volumes that contain symlinks. For interoperability
work, also create reference images with `mkudffs` and confirm that Hadris
reads them.
