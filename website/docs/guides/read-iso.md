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

`IsoFs::mount` mounts an image on any `hadris-storage` block device, a host
file included, reading its most capable tree: Rock Ridge over the primary
tree, then Joliet, then the ISO 9660:1999 enhanced tree, then the primary
tree. `IsoFs::mount_namespace` picks one. `IsoFs` implements the `hadris-fs`
`FileSystem` trait, so `Volume`, `read_tree` and the host helpers work on it:

```rust,no_run
use hadris_fs::sync::{FileSystem, Volume, read_tree};
use hadris_fs::{DirCursor, MountOptions};
use hadris_iso::sync::IsoFs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = hadris_storage::host::FileDevice::open("image.iso")?;
    let mut iso = IsoFs::mount(file, MountOptions::new())?;
    let mut buf = [0u8; 2048];
    if let Some(catalog) = iso.boot_catalog(&mut buf)? {
        for entry in catalog.entries() {
            let image = iso.boot_image(&entry);
            println!("boot: {:?}, {} bytes at byte {}", entry.platform(), image.len(), image.offset());
        }
    }
    let root = iso.root();
    let mut cursor = DirCursor::START;
    while let Some(entry) = iso.readdir(root, cursor)? {
        println!("{}", String::from_utf8_lossy(entry.name().as_bytes()));
        cursor = entry.next_cursor();
    }
    let vol = Volume::new(iso);
    let tree = read_tree(&vol, "/")?;
    hadris_fs::host::write_tree("out", &tree)?;
    Ok(())
}
```

Reading needs no allocator, and the same API exists in `hadris_iso::r#async`. `IsoFs::info` returns the volume
descriptor's identifiers and dates, `IsoFs::rock_ridge` a node's Rock Ridge
entries, `IsoFs::records` and `IsoFs::extents` where its directory records and
data lie (read them with `IsoFs::read_raw`), and `hadris_iso::raw` the on-disk
layouts.

Use `hadris-optical` when an application must detect and open ISO-only,
UDF-only, or bridge images. Use `hadris_udf::sync::write_bridge` to author a
shared ISO/UDF bridge image.
