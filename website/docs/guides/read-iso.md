---
title: Read ISO images
---

# Read ISO 9660 images

For authoring, see the dedicated [ISO filesystem creation guide](../creation/iso.md).

The ISO crate includes runnable examples for the common workflows:

```bash
cargo run -p hadris-iso --example read_iso -- image.iso
cargo run -p hadris-iso --example extract_files -- image.iso output/
cargo run -p hadris-iso --example create_bootable_iso
```

`IsoImage` opens an image on any `hadris-storage` block device, a host file
included, and `view` picks one of its trees: the primary tree, Rock Ridge over
it, Joliet or the ISO 9660:1999 enhanced tree. `Namespace::Preferred` takes
the most capable one. A view implements the `hadris-fs` `FileSystem` trait, so
`Volume`, `copy_tree` and the host helpers work on it:

```rust,no_run
use hadris_fs::DirCursor;
use hadris_fs::sync::FileSystem;
use hadris_iso::Namespace;
use hadris_iso::sync::IsoImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut iso = IsoImage::open(hadris_storage::host::FileDevice::open("image.iso")?)?;
    if let Some(catalog) = iso.boot_catalog()? {
        for entry in catalog.entries() {
            println!("boot: {:?} at block {}", entry.platform(), entry.load_block());
        }
    }
    let mut view = iso.view(Namespace::Preferred)?;
    let root = view.root();
    let mut cursor = DirCursor::START;
    while let Some(entry) = view.readdir(root, cursor)? {
        println!("{}", String::from_utf8_lossy(entry.name().as_bytes()));
        cursor = entry.next_cursor();
    }
    hadris_fs::sync::extract_to_host(&mut view, "/", "out")?;
    Ok(())
}
```

Reading needs no allocator, and the same API exists in `hadris_iso::r#async`. `IsoView::rock_ridge` returns a node's Rock
Ridge entries, `IsoView::raw_record` its directory record, and
`hadris_iso::raw` the on-disk layouts.

Use `hadris-optical` when an application must detect and open ISO-only,
UDF-only, or bridge images. Use `hadris-cd` to author a shared ISO/UDF bridge
image.
