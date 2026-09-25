---
title: Create UDF filesystems
---

# Create UDF filesystems

`hadris-udf` writes mastered, read-only type 1 UDF volumes from a
`hadris_fs::Tree`. `write_bridge` writes a shared ISO/UDF bridge image.

## Dependency

```toml
[dependencies]
hadris-fs = { version = "2.4.0", features = ["std", "sync"] }
hadris-udf = "2.4.0"
hadris-storage = "2.4.0"
```

## Create a volume

```rust,no_run
use hadris_fs::{Content, Node, Tree};
use hadris_udf::UdfOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = Tree::new();
    tree.insert("README.txt", Node::file(Content::bytes("Hello from a UDF image\n")))?;
    tree.insert("docs/guide.txt", Node::file(Content::bytes("UDF guide\n")))?;

    let target = hadris_storage::host::FileDevice::new(std::fs::File::create("volume.udf")?)?;
    let report = hadris_udf::sync::write(target, &tree, &UdfOptions::default())?;
    println!("wrote {} bytes", report.size());
    Ok(())
}
```

A `FileDevice` over a host file grows as the writer writes it, and so does a
`Vec<u8>`. For a fixed-size device such as a
`MemDevice`, size it with `hadris_udf::plan(&tree, &options)` first, which
does no I/O; `write` refuses a device that is too small before writing
anything. `hadris_fs::host::read_tree` imports a host directory without
reading the files until the image is written.

## Select a mastered revision

```rust
use hadris_udf::{UdfId, UdfOptions, UdfRevision};

let options = UdfOptions::default()
    .with_id(UdfId::Volume, "ARCHIVE_2026")
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
options' time, `NoClock::TIME` (1980-01-01) by default, dates entries
without times, so the same tree gives the same bytes; `with_time` sets
another. The volume set identifier starts with a 16-digit serial derived
from `with_seed`, or the time, and the tree's paths, sizes and times;
`with_id(UdfId::VolumeSet, ..)` sets it outright.

## Author an ISO/UDF bridge

Do not concatenate separate ISO and UDF images. A bridge coordinates
descriptor locations, directory ICBs and payload extents.
`hadris_udf::sync::write_bridge(dev, &tree, &iso_options, &udf_options)`
writes one, and `hadris_udf::plan_bridge` plans it without I/O. Both
namespaces point at the same file data. From the command line:

```bash
hadris udf bridge image-root -o bridge.iso -J
hadris udf compare bridge.iso
```

## Validate the result

```bash
udfinfo volume.udf
7z l volume.udf
hadris udf check volume.udf
```

7-Zip does not open volumes that contain symlinks. For interoperability
work, also create reference images with `mkudffs` and confirm that Hadris
reads them.
