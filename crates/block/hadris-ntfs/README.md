# hadris-ntfs

`hadris-ntfs` is a read-only NTFS reader that needs no allocator. It is a
preview in Hadris 3.0: it implements the `hadris-fs` `FileSystem` trait,
whose shape is frozen, and its native methods may still change in 3.x
minors. Depend on it with `hadris-ntfs = "~3.0"` to take only compatible
updates.

`NtfsFs` opens a volume on any `hadris-storage` block device, in blocking
and asynchronous forms, so `Volume` and the file handles of `hadris-fs`
work on it.

`NtfsFs::info()` returns an owned `VolumeInfo` snapshot of the boot-sector
geometry and serial number without device I/O. Both modes return the same
type. The existing geometry getters remain available.

```rust,no_run
use std::io::Read;

use hadris_fs::{MountOptions, OpenOptions};
use hadris_fs::sync::Volume;
use hadris_ntfs::sync::NtfsFs;

let image = hadris_storage::host::FileDevice::open("disk.img")?;
let vol = Volume::new(NtfsFs::mount(image, MountOptions::new())?);
for entry in vol.read_dir("/")? {
    println!("{:?}", entry?.name());
}
let mut data = Vec::new();
vol.open("/docs/readme.txt", OpenOptions::new().read())?.read_to_end(&mut data)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Supported scope

- Boot sector geometry, sector sizes of 256 to 4096 bytes, and clusters of
  up to 2 MiB.
- MFT and index records of up to 4096 bytes, protected by update sequence
  arrays in 512-byte strides.
- Resident, non-resident, sparse and partly initialized streams.
- `$ATTRIBUTE_LIST`: streams, names and index roots in extension records,
  and a `$MFT` of up to 32 extents.
- Directory indexes, with their allocation bitmaps and resumable cursors.
- UTF-16 names, with unpaired surrogates shown as U+FFFD; Win32 and DOS
  names compare through the volume's `$UpCase` table, POSIX names exactly.
- Times and DOS attributes from `$STANDARD_INFORMATION`, hard link counts,
  named data streams (`streams`, `read_stream_at`), the volume label, and
  free space from `$Bitmap`.

Listings leave out DOS 8.3 aliases and the metadata files (MFT records below
16, such as `$MFT`); a lookup by name still finds them. Node ids are file
references, so they are stable and hard links share one.

## Limitations

- The filesystem is read-only; the write methods of `FileSystem` fail with
  `ReadOnly`.
- The driver uses `$MFTMirr` to recover record zero. It does not consult the
  `$Mft` bitmap.
- LZNT1-compressed streams are decoded when read. Encrypted streams and
  streams whose payload is stored by WOF or Data Deduplication return
  `Unsupported`.
- Symbolic links and junctions appear as symlinks. `read_windows_link`
  returns the original Windows target. The shared resolver follows targets
  relative to the mounted volume.
- Security descriptors are not read.
- Lookups descend the file-name index by key when it links child nodes.
  Unlinked indexes still use a bitmap scan.
- `$LogFile` is not replayed, so a volume that was not cleanly unmounted may
  read inconsistently. `NtfsFs::info()` reports the dirty flag, hibernation
  status, and whether the driver used the MFT mirror.

## Feature flags

| Feature | Default | Description |
| --- | --- | --- |
| `std` | Yes | Implies `alloc`; forwards `std` to `hadris-fs` and `hadris-storage` |
| `alloc` | via `std` | Forwards `alloc` to `hadris-fs` and `hadris-storage` |
| `sync` | Yes | The blocking API in `sync` |
| `async` | No | The unified asynchronous API in `async_` |
| `tracing` | No | Operation spans through the `tracing` crate, also without `std` |

Reading needs no allocator in any mode. No feature changes what an item
does.

## Development

The tests in `tests/read.rs` build volumes with `mkntfs`, `ntfscp` and a
`ntfs-3g` FUSE mount, and skip when the tools are missing. Run them in the
project container:

```console
scripts/test-ntfs.sh
```

Pass a command to the same script to run an individual check:

```console
scripts/test-ntfs.sh cargo test -p hadris-ntfs --all-features
```

## Tracing

The opt-in `tracing` feature emits operation spans through the application’s
subscriber, with or without `std`. It is disabled by default. See the
[tracing guide](../../../docs/tracing.md) for targets, metadata and async behavior.

## Volume bounds

The reader validates the boot sector's declared volume size against the device.
MFT records and physical mapping pairs must fit inside that volume; bytes in a
larger device's trailing region are not exposed as file contents. Sparse runs
and uninitialized file tails retain their zero-fill behavior.

## Unified asynchronous API

One filesystem type in `async_` accepts both `Send` and non-`Send` devices
through `hadris_storage::async_::BlockDevice`. It implements
`hadris_fs::async_::FileSystem` for every compatible device, and
`hadris_fs::async_::SendFileSystem` when the device and its operation state
are `Send`.

## Documentation

- [Specification coverage](../../../docs/spec-coverage.md#hadris-ntfs)
- [API reference](https://docs.rs/hadris-ntfs/3.0.0-rc.4)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
