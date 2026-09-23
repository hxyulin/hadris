---
title: Create ISO images
---

# Create ISO 9660 images

`hadris-iso` writes an image from a `hadris_fs::tree::Tree`: the same input
tree the other Hadris writers take. The writer can emit the primary ISO
namespace, Rock Ridge, Joliet, an ISO 9660:1999 enhanced tree, El Torito
boot catalogs and hybrid MBR/GPT tables. It writes to any `hadris-storage`
block device, in the sync, async and `Send` async APIs.

## Dependency

```toml
[dependencies]
hadris-fs = "2.4.0"
hadris-iso = "2.4.0"
```

The default features (`std`, `sync`) include the writer. Without `std`,
`alloc` is enough.

## Create a basic image

```rust
use hadris_fs::tree::{Content, Tree};
use hadris_iso::{Charset, IsoOptions, VolumeIdentifiers};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = Tree::new();
    tree.add_file("README.TXT", Content::bytes("Hello from Hadris\n"))?;
    tree.add_file("DOCS/GUIDE.TXT", Content::bytes("Getting started\n"))?;

    let options = IsoOptions::default()
        .with_volume(
            VolumeIdentifiers::new("HADRIS_DEMO")
                .with_preparer("HADRIS")
                .with_application("MY_APP"),
        )
        .with_charset(Charset::Strict);

    let image = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("demo.iso")?;
    let report = hadris_iso::sync::write(image, &tree, &options)?;
    println!("{} bytes", report.size_bytes());
    Ok(())
}
```

`add_file` creates missing parent directories. The output file does not need
to be pre-sized; `hadris_iso::sync::plan` returns the same `Report` without
writing when a device must be sized first, such as a `MemDevice`.

The `Report` also lists where each file went (`extent_of`, `extents`) and
warnings for what the options could not store, such as permissions without
Rock Ridge or a symlink in an image without it.

## Choose the name rules

```rust
use hadris_iso::{IsoLevel, IsoOptions, JolietLevel, NameCase};

let portable = IsoOptions::default()
    .with_level(IsoLevel::L2)
    .with_joliet(JolietLevel::L3);

let unix = IsoOptions::default()
    .with_level(IsoLevel::L3)
    .with_name_case(NameCase::Preserve)
    .with_enhanced_tree();
```

`IsoLevel` is the ECMA-119 interchange level of the primary tree: `L1` for
8.3 names, `L2` for 31-character names, `L3` for multi-extent files above
4 GiB. `NameCase::Preserve` keeps lowercase letters, and
`Charset::Strict` maps every character outside the d-characters.
`with_enhanced_tree` adds an ISO 9660:1999 tree with long names. Joliet is
usually the most interoperable choice for Unicode names.

## Preserve POSIX metadata with Rock Ridge

```rust
use hadris_fs::tree::{Content, Tree};
use hadris_fs::{DateTime, FileTimes, Mode, SetMetadata};
use hadris_iso::{IsoOptions, RockRidge};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = Tree::new();
    tree.add_file("run.sh", Content::bytes("#!/bin/sh\necho hello\n"))?;
    let time = DateTime::from_unix_seconds(1_700_000_000)?;
    tree.set_metadata(
        "run.sh",
        SetMetadata::new()
            .with_mode(Mode::new(0o755))
            .with_uid(1000)
            .with_gid(1000)
            .with_times(FileTimes::new().with_modified(time)),
    )?;
    tree.add_symlink("latest", "run.sh")?;

    let options = IsoOptions::default().with_rock_ridge(RockRidge::default());
    let _ = hadris_iso::sync::plan(&tree, &options)?;
    Ok(())
}
```

Rock Ridge stores modes, owners, times, symlinks, device nodes and hard links.
Entries without times get the options' clock: the default `NoClock` writes
1980-01-01, so images are reproducible, and `with_clock(SystemClock)` stamps
the current time. Directories nested deeper than ECMA-119 allows move into
a relocation directory (`rr_moved`); `RockRidge::with_relocation` picks
another name or rejects such trees.

## Create from a host directory

```rust
use hadris_fs::tree::{FromFsOptions, OnError, Tree};

let tree = Tree::from_fs("image-root", FromFsOptions::new().with_on_error(OnError::Warn))?;
for warning in tree.warnings() {
    eprintln!("{warning}");
}
# Ok::<(), std::io::Error>(())
```

`Tree::from_fs` records host paths, not contents, so the writer streams each
file once while writing the image. It keeps modes, owners, times, symlinks,
device nodes and hard links.

## Bootable and hybrid images

```rust
use hadris_iso::{BootEntry, ElTorito, HybridBoot, IsoOptions, Platform};

let options = IsoOptions::default()
    .with_el_torito(
        ElTorito::new(BootEntry::new("boot/bios.img").with_load_size(4))
            .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi)),
    )
    .with_hybrid(HybridBoot::hybrid());
```

The boot images are paths in the tree. `ElTorito::with_catalog_path` makes the
boot catalog visible as a file. For a full example, run:

```bash
cargo run -p hadris-iso --example create_bootable_iso -- bootable.iso
```

Use `hadris-cd` instead when the same payload must be visible through both ISO
9660 and UDF namespaces.

## Add to an existing image

`hadris_iso::sync::Session` reads an image into a tree whose files point at
their existing extents. Change the tree, then write it back with
`SessionMode::Append` (a new session after the old one) or
`SessionMode::Rewrite` (new directories in place). Unchanged files are not
copied.

## Validate the result

```bash
xorriso -indev demo.iso -toc
7z l demo.iso
hadris-iso verify --strict demo.iso
```

Treat external validation as part of release testing, especially for bootable,
enhanced-namespace, and hybrid images.
