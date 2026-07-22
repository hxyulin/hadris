---
title: Create ISO images
---

# Create ISO 9660 images

Use `hadris-iso` to create a seekable ISO image from an in-memory input tree.
The writer can emit the primary ISO namespace, ISO 9660:1999 enhanced names,
Joliet, Rock Ridge, and El Torito boot metadata. Creation currently uses the
synchronous API and requires a target implementing `Read + Write + Seek`.

## Dependency

```toml
[dependencies]
hadris-iso = { version = "2.0.0-rc.4", features = ["write", "sync", "joliet"] }
```

## Create a basic image

`InputTree` describes the directory hierarchy. `IsoImageWriter::create`
returns the target so the caller retains ownership of the completed image.

```rust
use std::fs::OpenOptions;

use hadris_iso::read::PathSeparator;
use hadris_iso::write::options::{CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{InputEntry, InputTree, IsoImageWriter};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tree = InputTree::new(
        PathSeparator::ForwardSlash,
        vec![
            InputEntry::file("README.TXT", b"Hello from Hadris\n"),
            InputEntry::directory(
                "DOCS",
                vec![InputEntry::file("GUIDE.TXT", b"Getting started\n")],
            ),
        ],
    );

    let options = IsoFormatOptions {
        volume_name: "HADRIS_DEMO".into(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: Some("HADRIS".into()),
        application_id: Some("MY_APP".into()),
        sector_size: 2048,
        features: CreationFeatures::default(),
        path_separator: PathSeparator::ForwardSlash,
        strict_charset: true,
    };

    let image = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("demo.iso")?;
    let _image = IsoImageWriter::create(image, tree, options)?;
    Ok(())
}
```

The output file does not need to be pre-sized. Keep `sector_size` at 2048 for
optical interoperability.

## Enable filename namespaces

The base interchange level and additional filename namespaces are independent:

```rust
use hadris_iso::joliet::JolietLevel;
use hadris_iso::write::options::{BaseIsoLevel, CreationFeatures};

let portable = CreationFeatures {
    filenames: BaseIsoLevel::Level2 {
        supports_lowercase: false,
        supports_rrip: false,
    },
    joliet: Some(JolietLevel::Level3),
    ..CreationFeatures::default()
};

let iso_1999 = CreationFeatures {
    // Compatibility spelling for the ISO 9660:1999 enhanced namespace.
    long_filenames: true,
    ..CreationFeatures::default()
};
```

`BaseIsoLevel::Level3` selects interchange level 3; it is not the switch for
ISO 9660:1999 names. The `long_filenames` field is retained for compatibility.
Joliet is usually the most interoperable choice for Unicode names.

## Preserve POSIX metadata with Rock Ridge

```rust
use hadris_iso::write::options::CreationFeatures;
use hadris_iso::write::{InputEntry, InputMetadata};

let entry = InputEntry::file("run.sh", b"#!/bin/sh\necho hello\n").with_metadata(
    InputMetadata {
        mode: Some(0o755),
        uid: Some(1000),
        gid: Some(1000),
        modified: Some(1_700_000_000),
        ..InputMetadata::default()
    },
);

let features = CreationFeatures::rock_ridge();
```

Explicit timestamps make builds reproducible. Host filesystem scanning can
populate metadata, but inputs constructed in code give the caller full control.

## Create from a host directory

```rust
use std::path::Path;
use hadris_iso::read::PathSeparator;
use hadris_iso::write::InputTree;

let tree = InputTree::from_fs(
    Path::new("image-root"),
    PathSeparator::ForwardSlash,
)?;
# Ok::<(), hadris_iso::write::FileConversionError>(())
```

The current host-directory convenience API loads regular-file contents into
memory. For very large trees, construct inputs deliberately and budget memory
accordingly.

## Bootable and hybrid images

El Torito and hybrid MBR/GPT options live under `CreationFeatures`. For a full
boot-catalog example, run:

```bash
cargo run -p hadris-iso --example create_bootable_iso
```

Use `hadris-cd` instead when the same payload must be visible through both ISO
9660 and UDF namespaces.

## Validate the result

```bash
xorriso -indev demo.iso -toc
7z l demo.iso
hadris-iso info demo.iso
```

Treat external validation as part of release testing, especially for bootable,
enhanced-namespace, and hybrid images.
