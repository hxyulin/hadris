# hadris-block

`hadris-block` is the block-storage facade for Hadris. It groups storage-device
traits, MBR/GPT partition tables, FAT12/16/32, non-destructive format detection,
partition slices, and unified filesystem opening without erasing the
concrete leaf-crate APIs.

Use the facade when an application needs several block-storage layers. Use
`hadris-fat`, `hadris-part`, or `hadris-storage` directly when only one layer is
needed.

```toml
[dependencies]
hadris-block = "2.4.0"
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

```rust,ignore
use hadris_block::partition::sync::gpt_partition;
use hadris_block::sync::OpenVolume;

// `disk` is any hadris-storage `BlockDevice`, such as a `std::fs::File`.
let partition = gpt_partition(&mut disk, &entry).expect("partition fits the disk");
let volume = match OpenVolume::open(partition) {
    Ok(volume) => volume,
    // The error gives the device back, so the caller can try another opener.
    Err(err) => return Err(err.into_error()),
};
let fs = volume.into_fat().ok().unwrap(); // a hadris_fat::sync::FatFs
```

Detection reads only the identifying metadata of a block device. Opening the
detected concrete format performs full validation and yields the `FatFs`
driver, which works with the `hadris-fs` path helpers. Partitioned disks must
first be narrowed to a partition with `partition::sync`, `partition::r#async`
or `partition::async_send`, which return `hadris-storage` slices. A failed
open returns an `OpenError` that carries the device back to the caller.
Like `hadris_fs::Error`, `Error` and `OpenError` report a shared
`ErrorKind` through `kind()` and the device's own error through
`device_error()`, and convert into `hadris_fs::Error` and, with `std`,
`std::io::Error`.

## Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `std` | yes | Hosted support; enables `alloc` |
| `alloc` | yes | Heap-backed APIs without requiring `std` |
| `sync` | yes | Synchronous I/O APIs |
| `async` | no | Asynchronous I/O APIs |
| `async-send` | no | Asynchronous APIs with `Send` futures in `async_send` modules; enables `async` |
| `read` | yes | Filesystem and partition reading |
| `write` | yes | FAT and partition mutation |
| `detect` | yes | Lightweight block-format detection on a `BlockDevice` |
| `storage` | yes | Re-export `hadris-storage` |
| `fat` | yes | Re-export `hadris-fat` and open FAT volumes as `FatFs` |
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
