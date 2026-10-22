# hadris

The umbrella crate of Hadris, the Rust storage stack: one dependency that
re-exports the shared I/O, block-device and filesystem layers and each
enabled format driver, with detection and opening of unknown images.

```toml
[dependencies.hadris]
version = "3.0.0-rc.2"
default-features = false
features = ["std", "sync", "fat"]
```

Specify the prerelease version explicitly; `version = "3"` does not select a
release candidate. Disable the defaults and select only the formats and I/O
mode an application needs.

- `hadris::io`, `hadris::storage` and `hadris::fs`: portable I/O traits,
  block devices with caching and partition windows, and the shared
  `FileSystem` trait, `Volume` handles and image-writer `Tree`. Always
  present.
- `hadris::fat`, `hadris::part`, `hadris::iso`, `hadris::udf` and
  `hadris::cpio`: the native API of each format crate, one feature each.
- `sync::detect`, `sync::open`, `sync::open_partition` and `AnyFs` (and
  their `async_` twins), with `detect`: every format a device holds, and
  mounting of the first filesystem or of one MBR or GPT partition.
- `hadris::host` (`std`): host files and directories as devices and trees.

## Features

| Feature | Default | Adds |
|---|---:|---|
| `std` | Yes | Implies `alloc`; host files, `host`, `std::io` conversions |
| `alloc` | via `std` | Allocating drivers and `open` |
| `sync` | Yes | The blocking API in each `sync` module |
| `async` | No | The runtime-neutral API in each `async_` module |
| `write` | Yes | The writers of every enabled format (FAT and exFAT formatting, ISO 9660, UDF with the bridge, cpio), and `format` with `detect` |
| `fat`, `part`, `iso`, `udf`, `cpio` | `fat`, `iso`, `cpio` | One format crate each |
| `detect` | Yes | `detect`, `open`, `open_partition`, `detect_partitions` and `AnyFs`; enables `fat`, `part`, `iso`, `udf` and `cpio` |
| `tracing` | No | Function spans in every enabled format |
| `unstable-ntfs` | No | The NTFS preview as `hadris::ntfs` |
| `unstable-apfs` | No | The read-only APFS preview as `hadris::apfs`, and APFS mounting through `open` with `detect` |
| `apfs-encryption` | No | APFS software unlocking; implies `unstable-apfs` |

The platform (`std`, `alloc`) and mode (`sync`, `async`) features are
forwarded to every enabled crate, and so is `write`.
Everything an `unstable-*` feature adds is outside semver. The previews
`hadris-ntfs` and `hadris-apfs` keep 3.x versions but may break in a minor
release; depend on them directly with `~3.0`.

For allocation-free use, select a format and mode without `std` or `alloc`:
`["sync", "iso"]` reads ISO images and `["sync", "fat"]` gives the embedded
FAT API. Individual crates remain available when their versions should be
managed separately.

## Documentation

- [Hadris documentation](https://hxyulin.github.io/hadris/)
- [Choosing a crate](https://hxyulin.github.io/hadris/crates)
- [API reference](https://docs.rs/hadris/3.0.0-rc.2)
- [Project README](../../../README.md) and [changelog](../../../CHANGELOG.md)
- [Migration guide from 2.x](../../../docs/hadris-3.0.0-migration.md)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
