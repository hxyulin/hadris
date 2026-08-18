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

The reader supports the primary and ISO 9660:1999 enhanced namespaces, Joliet,
Rock Ridge/SUSP, and El Torito metadata. Reading is available through both the
sync and async APIs.

Use `hadris-optical` when an application must detect and open ISO-only,
UDF-only, or bridge images. Use `hadris-cd` to author a shared ISO/UDF bridge
image.
