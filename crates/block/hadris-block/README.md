# hadris-block

`hadris-block` is the block-storage facade for Hadris. It groups storage-device
traits, MBR/GPT partition tables, FAT12/16/32, non-destructive format detection,
checked partition views, and unified filesystem opening without erasing the
concrete leaf-crate APIs.

Use the facade when an application needs several block-storage layers. Use
`hadris-fat`, `hadris-part`, or `hadris-storage` directly when only one layer is
needed.

```toml
[dependencies]
hadris-block = "2.0.0"
```

```rust
use hadris_block::detect::{BlockFormat, FatVariant, detect_sector};

let mut sector = [0u8; 512];
// Fill `sector` from a disk or image.
let detected = detect_sector(&sector);
if let Some(BlockFormat::Fat(FatVariant::Fat32)) = detected {
    // Open through hadris_block::sync::OpenVolume or hadris_block::fat.
}
```

Detection restores stream positions and examines only identifying metadata.
Opening the detected concrete format performs full validation. Partitioned disks
must first be narrowed with the checked partition-view APIs.

## Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `std` | yes | Hosted support; enables `alloc` |
| `alloc` | yes | Heap-backed APIs without requiring `std` |
| `sync` | yes | Synchronous I/O APIs |
| `async` | no | Asynchronous I/O APIs |
| `read` | yes | Filesystem and partition reading |
| `write` | yes | FAT and partition mutation |
| `detect` | yes | Lightweight block-format detection |
| `storage` | yes | Re-export `hadris-storage` |
| `fat` | yes | Re-export `hadris-fat` with LFN support |
| `part` | yes | Re-export `hadris-part` |

The stable unified opener handles FAT12/16/32. exFAT remains an unstable
leaf-crate preview and is detected but not opened by this facade. Experimental
NTFS support likewise remains in the separate `hadris-ntfs` crate.

For `no_std` targets, disable default features and select one I/O mode
explicitly.

## Documentation

- [Choose a crate](https://hxyulin.github.io/hadris/crates)
- [Detect and open images](https://hxyulin.github.io/hadris/guides/detect-open-images)
- [Open FAT inside a partition](https://hxyulin.github.io/hadris/guides/open-partitioned-fat)
- [API reference](https://docs.rs/hadris-block)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
