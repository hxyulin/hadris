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
the most capable one. A view implements the `hadris-fs` `FsDriver` trait, so
the path helpers work on it:

```rust
use hadris_fs::sync::DriverExt;
use hadris_iso::Namespace;
use hadris_iso::sync::IsoImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut iso = IsoImage::open(std::fs::File::open("image.iso")?)?;
    if let Some(catalog) = iso.boot_catalog()? {
        for entry in catalog.entries() {
            println!("boot: {:?} at block {}", entry.platform(), entry.load_block());
        }
    }
    let mut view = iso.view(Namespace::Preferred)?;
    for item in view.read_dir("/")? {
        let item = item?;
        println!("{}", String::from_utf8_lossy(item.name_bytes()));
    }
    hadris_fs::sync::extract_to_host(&mut view, "/", "out")?;
    Ok(())
}
```

Reading needs no allocator, and the same API exists in `hadris_iso::r#async`
and `hadris_iso::async_send`. `IsoView::rock_ridge` returns a node's Rock
Ridge entries, `IsoView::raw_record` its directory record, and
`hadris_iso::raw` the on-disk layouts.

Use `hadris-optical` when an application must detect and open ISO-only,
UDF-only, or bridge images. Use `hadris-cd` to author a shared ISO/UDF bridge
image.
