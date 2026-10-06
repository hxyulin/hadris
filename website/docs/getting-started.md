---
title: Getting started
---

# Getting started with 3.0.0-rc.1

Use `hadris` for a single dependency that reaches the format drivers, shared
filesystem API, block devices and stream adapters. Disable defaults to choose
only the formats and I/O mode the application needs.

## Get the release candidate

`3.0.0-rc.1` is published on crates.io. Select the release candidate explicitly:

```toml
[dependencies.hadris]
version = "3.0.0-rc.1"
default-features = false
features = ["std", "sync", "fat"]
```

A plain `version = "3"` cannot select a prerelease. The recipes in these guides
use the explicit RC1 version from crates.io. For development snapshots, use a
Git dependency pinned to a reviewed `rev`.

Install the command-line tool with:

```sh
cargo install hadris-cli --version 3.0.0-rc.1 --locked
hadris --help
```

For source examples and the CLI:

```sh
git clone https://github.com/hxyulin/hadris.git
cd hadris
cargo run --locked -p hadris-example-migrate-v3
cargo run --locked -p hadris-cli -- --help
```

The migration example builds its own FAT and ISO images and verifies path
handles, generic filesystem reads and the allocation-free FAT API. It needs
no input or external image tools.

## Read a FAT image

```rust,no_run
use hadris::fat::sync::FatFs;
use hadris::fs::MountOptions;
use hadris::fs::sync::Volume;
use hadris::host::FileDevice;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let device = FileDevice::open("disk.img")?;
    let fs = FatFs::mount(device, MountOptions::new().read_only())?;
    let volume = Volume::new(fs);
    for entry in volume.read_dir("/")? {
        println!("{}", String::from_utf8_lossy(entry?.name().as_bytes()));
    }
    Ok(())
}
```

`hadris::fat`, `hadris::fs`, `hadris::storage` and `hadris::io` are re-exports
of the same public APIs offered by the individual crates. The umbrella does
not require detection or additional formats when defaults are disabled.
[Choose individual crates](./crates.md) when managing those dependencies and
versions directly is useful.

## Pick the interface and features

| Application | Features on `hadris` | Interface |
|---|---|---|
| Host FAT reader/editor | `std`, `sync`, `fat` | `fat::sync::FatFs` and `fs::sync::Volume` |
| Host FAT formatter | Add `write` | `fat::sync::format` or `write` |
| Unknown host image | `std`, `sync`, `detect`; add `part` for partition operations | `host::open` or `sync::detect` |
| Allocator-equipped FAT kernel | `alloc`, `sync`, `fat` | Bare `FileSystem` driver; sync `Volume` requires `std` |
| Allocated async FAT | `alloc`, `async`, `fat` | `fat::r#async::FatFs` and async `Volume` |
| Allocation-free FAT firmware | `sync`, `fat` | `fat::embedded::sync::Fat`; 512-byte device blocks |
| Allocation-free ISO/UDF reader | `sync`, `iso` or `udf` | Bare `FileSystem` driver |
| Streaming CPIO | `sync`, `cpio`; add `alloc` for writing | `cpio::sync::CpioReader` or `Writer` |

`std` implies `alloc`, but neither selects an I/O mode. `write` adds FAT and
exFAT formatting; ordinary file mutation is already available in their drivers.
Reading is always compiled in stable format crates. The APFS preview retains
its `read` feature internally.

Use `Volume` for paths and handles, and the bare `FileSystem` trait for node
operations and platform integrations. Close written files and explicitly
unmount when errors from metadata publication or device flush must be reported.
The shared async tier requires `Send` devices/futures. The embedded FAT/exFAT
tier supports local futures; see [async support boundaries](./guides/async-io.md).

## Next steps

- [Migrate a 2.x application](./migration.md)
- [Read and edit FAT](./guides/read-fat-image.md)
- [Detect an unknown image](./guides/detect-open-images.md)
- [Open a filesystem inside a partition](./guides/open-partitioned-fat.md)
- [Read ISO](./guides/read-iso.md) or [UDF](./guides/read-udf.md)
- [Create an ISO](./creation/iso.md) or [build an initramfs](./guides/build-initramfs.md)
- [Select features](./concepts/features.md) for [no_std](./guides/no-std.md) or [firmware](./guides/embedded.md)
- [Browse the compiled examples](https://github.com/hxyulin/hadris/blob/main/examples/README.md)
