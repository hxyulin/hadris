# hadris-ntfs

`hadris-ntfs` is a read-only NTFS reader that needs no allocator. It is a
preview in Hadris 3.0: it implements the `hadris-fs` `FsDriver` trait, whose
shape is frozen, and its native methods may still change in 3.x minors.

`NtfsFs` opens a volume on any `hadris-storage` block device, in blocking,
asynchronous and `Send` asynchronous forms, so the path helpers, `Volume`
and file handles of `hadris-fs` work on it.

```rust,no_run
use hadris_fs::sync::DriverExt;
use hadris_ntfs::sync::NtfsFs;

let image = std::fs::File::open("disk.img")?;
let mut ntfs = NtfsFs::open(image)?;
for entry in ntfs.read_dir("/")? {
    println!("{:?}", entry?.name());
}
let data = ntfs.read_to_vec("/docs/readme.txt")?;
# let _ = data;
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

- The filesystem is read-only; the write methods of `FsDriver` fail with
  `ReadOnly`.
- `$MFTMirr` is not used to recover unreadable MFT records, and the `$Mft`
  bitmap is not consulted.
- Compressed and encrypted streams fail with `Unsupported`.
- Reparse points (symbolic links, junctions) are shown as ordinary files and
  directories.
- Security descriptors are not read.
- Lookups scan the directory index instead of descending its B-tree by key.
- `$LogFile` is not replayed, so a volume that was not cleanly unmounted may
  read inconsistently.

## Feature flags

| Feature | Default | Description |
| --- | --- | --- |
| `std` | Yes | Implies `alloc`; `std::io::Error` conversions |
| `alloc` | via `std` | `AnyError` conversions |
| `sync` | Yes | The blocking API in `sync` |
| `async` | No | The asynchronous API in `r#async` |
| `async-send` | No | The asynchronous API with `Send` futures in `async_send` |

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

## Documentation

- [Specification coverage](../../../docs/spec-coverage.md#hadris-ntfs)
- [API reference](https://docs.rs/hadris-ntfs)

## License

Licensed under the [MIT license](../../../LICENSE-MIT).
