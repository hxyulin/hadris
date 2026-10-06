---
title: Create ISO images
---

# Create ISO 9660 images

`hadris-iso` writes an image from a `hadris_fs::Tree`: the same input
tree the other Hadris writers take. The writer can emit the primary ISO
namespace, Rock Ridge, Joliet, an ISO 9660:1999 enhanced tree, El Torito
boot catalogs and hybrid MBR/GPT tables. It writes to any `hadris-storage`
block device, in the sync, async and `Send` async APIs.

## Dependency

```toml
[dependencies]
hadris-fs = "3.0.0-rc.1"
hadris-iso = "3.0.0-rc.1"
hadris-storage = "3.0.0-rc.1"
```

The default features (`std`, `sync`) include the writer. Without `std`,
`alloc` is enough.

## Create a basic image

```rust,no_run
use hadris_fs::{Content, Node, Tree};
use hadris_iso::{IsoId, IsoOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = Tree::new();
    tree.insert("README.TXT", Node::file(Content::bytes("Hello from Hadris\n")))?;
    tree.insert("DOCS/GUIDE.TXT", Node::file(Content::bytes("Getting started\n")))?;

    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "HADRIS_DEMO")
        .with_id(IsoId::Preparer, "HADRIS")
        .with_id(IsoId::Application, "MY_APP");

    let image = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open("demo.iso")?;
    let image = hadris_storage::host::FileDevice::new(image)?;
    let report = hadris_iso::sync::write(image, &tree, &options)?;
    println!("{} bytes", report.size());
    Ok(())
}
```

`with_id` sets each descriptor identifier and `with_date` each descriptor
date. Identifiers are stored as given, as xorriso does; one longer than its
field fails the plan with `Detail::Identifier`. `insert` creates missing
parent directories. The output file does not need
to be pre-sized: `write` checks the planned size against the device's
`max_block_count`, and a host file or `Vec<u8>` grows. `hadris_iso::plan`
returns the same `Report` without any I/O when a device must be sized
first, such as a `MemDevice`.

The `Report` also lists where each file went (`extents(path)`, `files()`)
and warnings for what the options could not store, such as permissions
without Rock Ridge or a symlink in an image without it.

## Choose the name rules

```rust
use hadris_iso::{IsoLevel, IsoOptions, NameCase};

let portable = IsoOptions::default()
    .with_level(IsoLevel::L2)
    .with_joliet();

let unix = IsoOptions::default()
    .with_level(IsoLevel::L3)
    .with_name_case(NameCase::Preserve)
    .with_iso1999();
```

`IsoLevel` is the ECMA-119 interchange level of the primary tree: `L1` for
8.3 names, `L2` for 31-character names, `L3` for multi-extent files above
4 GiB. `NameCase::Preserve` keeps lowercase letters. `with_joliet` adds a
Joliet tree and `with_iso1999` an ISO 9660:1999 tree with long names. Joliet is
usually the most interoperable choice for Unicode names.

## Preserve POSIX metadata with Rock Ridge

```rust
use hadris_fs::{Content, DateTime, Node, Owner, Permissions, SetAttr, Tree};
use hadris_iso::IsoOptions;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tree = Tree::new();
    let time = DateTime::from_unix_seconds(1_700_000_000)?;
    let attrs = SetAttr::new()
        .with_permissions(Permissions::new(0o755))
        .with_owner(Owner::new(1000, 1000))
        .with_modified(time);
    let script = Content::bytes("#!/bin/sh\necho hello\n");
    tree.insert("run.sh", Node::file(script).with_attrs(attrs))?;
    tree.insert("latest", Node::symlink("run.sh"))?;

    let options = IsoOptions::default().with_rock_ridge();
    let _ = hadris_iso::plan(&tree, &options)?;
    Ok(())
}
```

Rock Ridge stores modes, owners, times, symlinks, device nodes and hard links.
Entries without times get the options' time: `NoClock::TIME`
(1980-01-01) by default, so images are reproducible, and `with_time` sets
another, such as `hadris_fs::host::source_date_epoch()`. The GUIDs of hybrid
images derive from the tree's paths, sizes and times together with `with_seed`,
or the time without one. Directories nested deeper than ECMA-119 allows move into
a relocation directory, `rr_moved` by default;
`IsoOptions::with_relocation` picks `Relocation::DotRrMoved` for `.rr_moved`
or `Relocation::Refuse` to refuse such trees, and `with_preserve` chooses
which metadata Rock Ridge copies. libarchive and `bsdtar` read
relocated directories only from those two names.

## Create from a host directory

```rust,no_run
use hadris_fs::host::{self, OnError, TreeOptions};

let options = TreeOptions::new().with_on_error(OnError::Skip);
let (tree, skipped) = host::read_tree("image-root", &options)?;
for error in skipped {
    eprintln!("skipped: {error}");
}
# Ok::<(), hadris_fs::PathError>(())
```

`host::read_tree` records host paths, not contents, so the writer streams
each file once while writing the image. It keeps modes, owners, times,
symlinks, device nodes and hard links; `TreeOptions` also sets the symlink
policy, an exclude filter, an owner override and an mtime clamp.

## Bootable and hybrid images

```rust
use hadris_fs::Content;
use hadris_iso::{AppendedPartition, BootEntry, ElTorito, Hybrid, IsoOptions};

let esp = Content::bytes(vec![0u8; 1440 * 1024]);
let options = IsoOptions::default()
    .with_el_torito(
        ElTorito::new()
            .with_entry(BootEntry::bios("boot/bios.img").with_load_size(4))
            .with_entry(BootEntry::uefi_appended(0)),
    )
    .with_hybrid(Hybrid::gpt_hybrid_mbr().with_appended(AppendedPartition::esp(esp)));
```

`BootEntry::bios` and `BootEntry::uefi` boot images that are paths in the
tree. `Hybrid::with_appended` stores a partition after the files and lists
it in the GPT; `BootEntry::uefi_appended` boots it, so the EFI system
partition is stored once. `Hybrid::mbr` and `Hybrid::gpt` write a single
table. `ElTorito::with_catalog_path` makes the
boot catalog visible as a file. For a full example, run:

```bash
cargo run -p hadris-iso --example create_bootable_iso -- bootable.iso
```

Use `hadris_udf::sync::write_bridge` instead when the same payload must be
visible through both ISO 9660 and UDF namespaces.

## Add to an existing image

`hadris_iso::sync::Session::open` reads an image into a tree whose files
point at their existing extents. Change `tree_mut()`, then `write` it with
`SessionMode::Append` (a new session after the old one) or
`SessionMode::Rewrite` (new directories in place). Unchanged files are not
copied.

## Validate the result

```bash
xorriso -indev demo.iso -toc
7z l demo.iso
hadris iso check --strict demo.iso
```

Treat external validation as part of release testing, especially for bootable,
enhanced-namespace, and hybrid images.
