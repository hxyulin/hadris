---
title: Read and create ISO images
---

# Read and create ISO 9660 images

The ISO crate includes runnable examples for the common workflows:

```bash
cargo run -p hadris-iso --example read_iso -- image.iso
cargo run -p hadris-iso --example extract_files -- image.iso output/
cargo run -p hadris-iso --example create_bootable_iso
```

The reader supports ISO 9660 namespaces, Joliet, Rock Ridge/SUSP, and El Torito
metadata. Image creation is synchronous; reading is available through both the
sync and async APIs.

Use `hadris-optical` when an application must detect and open ISO-only,
UDF-only, or bridge images. Use `hadris-cd` to author a shared ISO/UDF bridge
image.
