# hadris-block

`hadris-block` detects and opens block volumes: FAT12, FAT16, FAT32, exFAT
and NTFS on any `hadris-storage` block device. It sits next to the format
crates it builds on and re-exports them, so one dependency covers "open
whatever this disk or partition holds".

```toml
[dependencies]
hadris-block = "2.4.0"
```

```rust,ignore
use hadris_block::detect::BlockFormat;
use hadris_block::sync::OpenVolume;
use hadris_fs::sync::DriverExt;

// `dev` is any hadris-storage `BlockDevice`, such as a `std::fs::File`.
let mut volume = match OpenVolume::open(dev) {
    Ok(volume) => volume,
    // The error gives the device back, so the caller can try another opener.
    Err(err) => return Err(err.into_error().into()),
};
if volume.format() == BlockFormat::Ntfs {
    println!("NTFS, read-only");
}
for entry in volume.read_dir("/")? {
    println!("{:?}", entry?.name());
}
```

- `detect` reads the boot sector (and the GPT header) of a device and
  reports a FAT variant, NTFS, exFAT or a partition table, without an
  allocator, in each mode.
- `OpenVolume` detects and mounts once. It implements the `hadris-fs`
  `FsDriver` trait by delegating to `hadris_fat`'s `FatFs` or `ExFatFs` or
  `hadris_ntfs`'s `NtfsFs`, so the path helpers, `Volume` and handles work
  on any volume it opens. NTFS is read-only; its write methods fail with
  `ReadOnly`. `as_fat`, `into_fat`, `as_exfat` and `into_exfat` reach the
  FAT and exFAT drivers' native APIs.
- A partitioned disk is refused with `Detail::PartitionedDisk`; open a
  partition from `hadris_part` (the `part` re-export), which gives a
  `hadris-storage` `Slice`.
- Every failure is an `Error<E>` with a shared `ErrorKind`, a `Detail` and
  the device's own error, and a failed open gives the device back in an
  `OpenError`. Both convert into `hadris_fs::Error`, `AnyError` with
  `alloc`, and `std::io::Error` with `std`.

NTFS is a preview: `OpenVolume` always opens it through `FsDriver`, and
the `unstable-ntfs` feature adds the `ntfs` re-export and `as_ntfs`,
`as_ntfs_mut` and `into_ntfs` for its native API.

## Features

| Feature | Default | Purpose |
|---------|---------|---------|
| `std` | yes | Implies `alloc`; `std::io::Error` conversions and `std::fs::File` devices |
| `alloc` | via `std` | `AnyError` conversions |
| `sync` | yes | The blocking API in `sync` |
| `async` | no | The asynchronous API in `r#async` |
| `async-send` | no | The asynchronous API with `Send` futures in `async_send`; enables `async` |
| `write` | no | FAT formatting through the `fat` re-export |
| `part` | no | Re-export `hadris-part` as `part` |
| `unstable-ntfs` | no | Re-export `hadris-ntfs` as `ntfs` and reach `OpenVolume`'s NTFS driver |

No feature changes what an item does: detection and `OpenVolume` always
cover FAT, exFAT and NTFS, and neither needs an allocator.

## Documentation

- [Choose a crate](https://hxyulin.github.io/hadris/crates)
- [Detect and open images](https://hxyulin.github.io/hadris/guides/detect-open-images)
- [Open FAT inside a partition](https://hxyulin.github.io/hadris/guides/open-partitioned-fat)
- [API reference](https://docs.rs/hadris-block)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
