# Migrating from Hadris 2.4 to 3.0

This guide is for code and scripts written against Hadris 2.4 (the V2 API on
`main`) that move to Hadris 3.0 (the V3 API). It covers the library crates,
their features, and the command-line tools.

Hadris 3.0 is a new API, not an incremental release. Every filesystem reads a
block device instead of a byte stream, every driver implements one shared
`FileSystem` trait, every writer takes one shared input `Tree`, and every
crate returns one error type. Most V2 type names are gone; the tables in
[Symbol tables](#symbol-tables) map each of them to its replacement.

Versioning:

- Every library crate ships 3.0.0 together. After 3.0.0 each crate has its
  own version and bumps its major only for its own breaking changes. The
  umbrella `hadris` bumps its major whenever a crate it re-exports does.
- `hadris-fat-raw` is new, starts at 0.1.0 and versions separately. It is
  never re-exported whole: `hadris-fat` re-exports only `FatKind`,
  `Geometry`, `Detail`, `exfat::Detail`, `exfat::Geometry` and the `check`
  functions.
- `hadris-cli` (the `hadris` binary) versions apart from the library.
- The MSRV is unchanged: Rust 1.88.0 (`rust-version` in the workspace
  `Cargo.toml`).

The design behind these changes is in [the V3 API design](v3-api-design.md);
section 6 lists what was renamed or removed in each step.

Sections:

1. [Crate map](#crate-map)
2. [Features](#features)
3. [Errors](#errors)
4. [I/O and devices](#io-and-devices)
5. [Filesystems](#filesystems)
6. [Builders and writers](#builders-and-writers)
7. [Detection and opening](#detection-and-opening)
8. [Partitions](#partitions)
9. [Embedded API](#embedded-api)
10. [Async](#async)
11. [Command-line tools](#command-line-tools)
12. [Symbol tables](#symbol-tables)
13. [Checklist](#checklist)

## Crate map

| V2 crate | V3 |
|---|---|
| `hadris` | `hadris`. Flat re-exports (`hadris::fat`, `hadris::iso`, `hadris::cpio`), `detect`, `open`, `AnyFs` and the `host` module. See [Features](#features). |
| `hadris-io` | `hadris-io`. New trait shapes and the crate-wide `Error<E>`. |
| `hadris-storage` | `hadris-storage`. One `BlockDevice` trait, used by every filesystem. |
| `hadris-common` | `hadris-common`, internal. Only the endian integers remain. |
| `hadris-macros` | `hadris-macros`, internal. |
| `hadris-fixed` | Removed. The fixed-capacity types have no public successor. |
| `hadris-path` | Removed. Paths are `/`-separated bytes (`impl AsRef<[u8]>`), resolved by `FileSystem::resolve` and `Volume` with `hadris_fs::Resolve`. |
| `hadris-archive` | Removed. Use `hadris-cpio`, or `hadris::cpio` (was `hadris::archive::cpio`). |
| `hadris-block` | Removed. Use `hadris::{sync, r#async}::{detect, open, AnyFs}`, `hadris::host::open`, and `hadris_part::sync::open` for partitions. |
| `hadris-fat` | `hadris-fat`. On-disk layouts moved to `hadris-fat-raw`. exFAT is stable. |
| `hadris-part` | `hadris-part`. New `Disk` API on block devices. |
| `hadris-ntfs` | `hadris-ntfs`, still a preview. `hadris::ntfs` behind the umbrella's `unstable-ntfs` feature. |
| `hadris-optical` | Removed. Use `hadris::{sync, r#async}::{detect, open, AnyFs}`. |
| `hadris-cd` | Removed. The ISO 9660 and UDF bridge writer is `hadris_udf::plan_bridge` and `hadris_udf::{sync, r#async}::write_bridge`. |
| `hadris-iso` | `hadris-iso`. One reader, `IsoFs`; the writer takes a `Tree`. |
| `hadris-udf` | `hadris-udf`. One reader, `UdfFs`; the writer takes a `Tree`; the bridge writer moved here. |
| `hadris-cpio` | `hadris-cpio`. Streaming `CpioReader` and `Writer`. |
| `hadris-fat-cli` (binaries `hadris-fat`, `fatutil`) | `hadris-cli`: `hadris fat` |
| `hadris-iso-cli` (binaries `hadris-iso`, `hadris-iso-cli`) | `hadris-cli`: `hadris iso` |
| `hadris-udf-cli` (binaries `hadris-udf`, `hadris-udf-cli`) | `hadris-cli`: `hadris udf` |
| `hadris-cpio-cli` (binaries `hadris-cpio`, `cpioutil`) | `hadris-cli`: `hadris cpio` |
| `hadris-cd-cli` (binary `hadris-cd`) | `hadris-cli`: `hadris udf bridge` and `hadris udf compare`; `info` has no direct successor (use `hadris detect`, `hadris iso info`, `hadris udf info`) |

New crates:

| Crate | Role |
|---|---|
| `hadris-fs` | Shared vocabulary (`NodeId`, `Name`, `Metadata`, `DateTime`, `MountOptions`), the `FileSystem` trait, `Volume` with `File` and `ReadDir`, `Walk`, `Tree`, `Node`, `Content`, `Report`, `copy_tree`, `read_tree`, `Finding`, `CheckReport`, the `host` module, and re-exports of the `hadris-io` error items. |
| `hadris-fat-raw` | FAT12/16/32 and exFAT on-disk layouts, I/O-free codecs, device primitives (`io`, `exfat::io`) and the allocation-free checkers. Version 0.1.0. |
| `hadris-cli` | The single `hadris` binary. Install with `cargo install hadris-cli`. |

A typical dependency change:

```toml
# 2.4
[dependencies]
hadris-fat = "2.4"

# 3.0
[dependencies]
hadris-fat = "3"
hadris-fs = "3"
hadris-storage = "3"
```

Until 3.0.0 is released, a requirement of `"3"` does not match the release
candidates: depend on `"3.0.0-rc.1"` to try them.

`hadris-fs` defaults to `std` and `sync`, like the other crates; turn
them off with `default-features = false` for `no_std` or async-only builds.
Code that only uses the umbrella can depend on `hadris` alone: it re-exports
`hadris::io`, `hadris::storage` and `hadris::fs` in every build.

## Features

Rules that hold for every V3 crate:

- Reading is always compiled. There is no `read` feature anywhere.
- `sync` and `async` select parallel namespaces (`sync`, `r#async`) and may be
  enabled together. `std` implies `alloc` but selects no I/O mode.
- A feature only adds items. No feature changes what an existing item does;
  V2 features that switched behaviour (`lfn`, `dirty-file-panic`, `crc`,
  `rand`, the `unstable-streaming` variants) are gone.
- `async` futures are `Send` when the device is. Futures that need not be
  `Send` exist only in `hadris_io::local`, `hadris_storage::local`,
  `hadris_fat_raw::{io, exfat::io}::local` and the embedded API.
- Only `unstable-*` features add APIs outside the stability promise.
- No crate re-exports a mode at its root. Write `hadris_fat::sync::FatFs`,
  not `hadris_fat::FatFs`.

Per crate:

| Crate | V2 features | V3 features | Removed, and what to do |
|---|---|---|---|
| `hadris` | `std`, `alloc`, `sync`, `async`, `read`, `write`, `block`, `optical`, `archive`, `path`, `fixed`, `storage`, `fat`, `part`, `iso`, `udf`, `cd`, `cpio` (default `std`, `sync`, `read`, `write`, `fixed`, `path`, `iso`, `fat`, `cpio`) | `std`, `alloc`, `sync`, `async`, `write`, `fat`, `part`, `iso`, `udf`, `cpio`, `detect`, `unstable-ntfs` (default `std`, `sync`, `write`, `fat`, `iso`, `cpio`, `detect`) | `read`: drop it. `archive`: use `cpio`. `block`, `optical`: use `detect` (adds `fat`, `iso`, `udf`, `cpio`) and `part`. `cd`: use `udf`. `storage`: `hadris::storage` is always there. `path`, `fixed`: the crates are gone. `write` now only forwards FAT formatting. |
| `hadris-io` | `std`, `alloc`, `sync`, `async` | `std`, `alloc`, `sync`, `async`, `embedded-io` | `embedded-io` is now optional and gates `FromEmbedded` and the `embedded-io` conversions. `async` also enables `local`. |
| `hadris-storage` | `std`, `alloc`, `sync`, `async` | `std`, `alloc`, `sync`, `async` | None. `async` also enables `local`. |
| `hadris-common` | `std`, `alloc`, `sync`, `async`, `bytemuck`, `optical` | `bytemuck` | The crate is internal; do not depend on it. |
| `hadris-fs` | (new) | `std`, `alloc`, `sync`, `async`, `contract` (default `std`, `sync`) | `contract` adds the driver test kit `contract::check`. |
| `hadris-fat` | `read`, `write`, `lfn`, `std`, `sync`, `async`, `alloc`, `cache`, `tool`, `unstable-exfat`, `defmt`, `dirty-file-panic` (default `read`, `write`, `lfn`, `std`, `sync`) | `std`, `sync`, `async`, `alloc`, `write`, `defmt` (default `std`, `sync`, `write`) | `read`, `lfn`: always on. `cache`: wrap the device in `hadris_storage::sync::Cache`. `tool`: use `check` (always compiled) and `FatFs::extents`. `unstable-exfat`: exFAT is in every build. `dirty-file-panic`: gone. `write` now adds only `format`; it no longer implies `alloc` or `read`. `FatFs`, `ExFatFs` and the tree writer `write` need `alloc`. `defmt` now derives only on `FatKind`. |
| `hadris-fat-raw` | (new) | `sync`, `async`, `defmt` (no defaults) | `sync` and `async` add the device primitives in `io` and `exfat::io`. |
| `hadris-part` | `std`, `sync`, `async`, `alloc`, `read`, `write`, `crc`, `rand` | `std`, `alloc`, `sync`, `async` | `read`, `write`: always on. `crc`: CRCs are always computed and checked. `rand`: pass GUIDs yourself or call `Guid::random()` (`std`). `Disk`, the tables and `DiskLayout` need `alloc`; `scan` and `open` do not. |
| `hadris-ntfs` | `read`, `std`, `sync`, `async`, `alloc` | `std`, `alloc`, `sync`, `async` | `read`: reading needs no allocator now. |
| `hadris-iso` | `read`, `alloc`, `std`, `sync`, `async`, `write`, `joliet`, `unstable-streaming` (default `std`, `write`, `sync`) | `std`, `alloc`, `sync`, `async` (default `std`, `sync`) | `read`: gone, reading needs no allocator. `write`, `joliet`: the writer, sessions and boot catalog reader need `alloc`. `unstable-streaming`: `hadris_fs::host::file` content is read while the image is written. |
| `hadris-udf` | `read`, `alloc`, `std`, `write`, `sync`, `async`, `unstable-streaming` | `std`, `alloc`, `sync`, `async` | `read`, `write`: the writer needs `alloc` and no longer needs `std`. `unstable-streaming`: as for ISO. |
| `hadris-cpio` | `read`, `alloc`, `std`, `write`, `sync`, `async` | `std`, `alloc`, `sync`, `async` | `read`: the reader is always compiled and needs no allocator. `write`: the writer needs `alloc`. |
| `hadris-block`, `hadris-optical`, `hadris-cd`, `hadris-archive` | various | crate removed | See [Crate map](#crate-map). |

Common configurations, from [the features page](../website/docs/concepts/features.md):

```toml
# Allocation-free reader, bootloader style
hadris-iso = { version = "3", default-features = false, features = ["sync"] }

# FAT or exFAT driver without std
hadris-fat = { version = "3", default-features = false, features = ["alloc", "sync"] }

# Firmware without an allocator: the embedded API
hadris-fat = { version = "3", default-features = false, features = ["sync", "write"] }
```

## Errors

V2 had one error enum per crate (`hadris_fat::Error`, `hadris_iso::write::IsoCreationError`,
`hadris_udf::Error`, `hadris_cpio::Error`, `hadris_part::Error`,
`hadris_ntfs::NtfsError`, `hadris_block::Error`, `hadris_optical::Error`,
`hadris_cd::Error`, `hadris_storage::Error`) around an erased `hadris_io::Error`
with `std::io`-like kinds. V3 has one error type for every crate.

| Item | Where | What it is |
|---|---|---|
| `Error<E>` | `hadris_io`, re-exported as `hadris_fs::Error` and `hadris::Error` | A kind, a static `message()`, an optional `location()`, an optional `detail()` code, and the device's own error `E` (`device_error()`, `into_device_error()`, `map_device`). `Clone` and `Copy` when `E` is. No allocation. |
| `FsResult<T, E>` | same | `Result<T, Error<E>>`. Replaces the per-crate `Result` aliases. |
| `ErrorKind` | same | `#[non_exhaustive]`: `Io`, `NotRecognized`, `Corrupt`, `NotFound`, `AlreadyExists`, `NotADirectory`, `IsADirectory`, `DirectoryNotEmpty`, `NoSpace`, `ReadOnly`, `InvalidInput`, `Unsupported`, `LimitExceeded`, `NameTooLong`, `FileTooLarge`, `Symlink`, `InvalidHandle`, `Busy`. The V2 `std::io`-style kinds (`InvalidData`, `UnexpectedEof`, `Other`, `PermissionDenied`, ...) are gone. |
| `NotRecognized` | `ErrorKind` variant | The bytes are not this format at all. `Corrupt` now means "this format, but damaged". |
| `Location` | same | `Byte(u64)`, `Block(u64)`, `Cluster(u64)`, `NameByte(u32)`. |
| `DetailCode` | same | A `u16` code within a crate's domain. Read it back with the crate's `Detail::of(&err)`. |
| `Detail` | `hadris_fat::Detail`, `hadris_fat::exfat::Detail`, `hadris_iso::Detail`, `hadris_udf::Detail`, `hadris_cpio::Detail`, `hadris_part::Detail`, `hadris_ntfs::Detail` | `#[non_exhaustive]` numbered codes naming the structure at fault. `Detail::of(&err) -> Option<Detail>`, `Detail::from_code(code)`, `Detail::code()`. Replaces the variants of the V2 error enums. |
| `Errno` | same | `ErrorKind::errno()` returns one symbolic errno per kind; `Errno::linux()` gives the number. `Unsupported` is `EOPNOTSUPP`, `Corrupt` is `EUCLEAN`, `InvalidHandle` is `ESTALE`. |
| `MountError<D, E>` | `hadris_fs`, `hadris` | Returned by `mount`, `unmount` and `hadris::sync::open`: the `Error<E>` and the device given back (`into_device`, `into_parts`, `into_error`). `?` converts it to `Error<E>`, `PathError` or `std::io::Error`. |
| `PathError` | `hadris_fs`, `hadris` (`alloc`) | Kind, message, location, detail, the path within the tree or volume, the host path with `std`, and the device error boxed as its `source()`. Returned by writers, `Tree` edits, `copy_tree`, `read_tree` and the `host` module. Takes `?` from any `Error<E>` or `MountError`. Unrelated to the V2 `hadris_path::PathError`. |
| `ExactError<E>` | `hadris_io` | `UnexpectedEof`, `WriteZero` or `Io(E)`, returned by `read_exact` and `write_all`. |

Converting:

- With `std`, `Error<E>` converts to `std::io::Error` with `?`. A device error
  that is already an `io::Error` comes back as itself, so `raw_os_error()`
  survives. `hadris_io::into_std_error` converts any device error.
- Generic code names the device error through the trait:
  `fn f<F: FileSystem>(fs: &mut F) -> FsResult<(), F::DeviceError>`.
- Code that mixes devices returns `PathError`.

Before and after:

```rust
// 2.4
fn main() -> hadris_fat::Result<()> { /* hadris_fat::Error */ Ok(()) }
```

```rust
// 3.0 (website/docs/guides/read-fat-image.md)
match vol.open("/README.TXT", OpenOptions::new().read()) {
    Ok(mut file) => { /* ... */ }
    Err(err) if err.kind() == ErrorKind::NotFound => {}
    Err(err) => return Err(err.into()),
}
```

To test for a format-specific cause, use the crate's `Detail`:

```rust
if hadris_iso::Detail::of(&err) == Some(hadris_iso::Detail::Identifier) {
    // an identifier longer than its field
}
```

## I/O and devices

### Streams: `hadris-io`

`Read`, `Write` and `Seek` report the implementor's own error through the
`ErrorType` supertrait, as in `embedded-io`:

```rust
pub trait ErrorType {
    type Error: core::error::Error + Send + Sync + 'static;
}
```

- The crate root no longer glob re-exports `sync`. Name the traits by mode:
  `hadris_io::sync::Read`, `hadris_io::r#async::Read`, `hadris_io::local::Read`.
  The root keeps the mode-independent items: `ErrorType`, `ExactError`,
  `Cursor`, `SeekFrom`, `StdIo`, `ToStd`, `FromEmbedded`, `Error`,
  `ErrorKind` and the other error items.
- `std::io` types are no longer blanket-implemented. Wrap them in
  `StdIo::new(stream)`; `ToStd` goes the other way.
- `embedded-io` types are wrapped in `FromEmbedded::new(dev)` (feature
  `embedded-io`). `ToEmbedded` is removed.
- `&mut T` and `Box<T>` implement the traits, so `Borrowed` is gone.
- `SeekFrom` is a Hadris `#[non_exhaustive]` enum; resolve it with
  `SeekFrom::resolve(current, len)` rather than matching. It converts to and
  from `std::io::SeekFrom` and `embedded_io::SeekFrom`.
- `ReadExt`, `Parsable`, `Writable`, `ReadSeek`, `ReadWrite`,
  `ReadWriteSeek`, `IoError` and the `Path`/`PathBuf` re-exports are gone.

Only cpio reads and writes streams in V3. Every filesystem reads a block
device.

### Block devices: `hadris-storage`

```rust
pub trait BlockDevice: ErrorType {
    fn block_size(&self) -> BlockSize;
    fn block_count(&self) -> u64;
    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>>;
    // defaulted:
    fn max_block_count(&self) -> u64;
    fn disk_offset(&self) -> u64;
    fn writable(&self) -> bool;                  // false
    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>>;
    fn flush(&mut self) -> Result<(), Error<Self::Error>>;
}
```

One trait replaces `BlockDevice` plus `BlockDeviceMut`. A read-only device
implements the first three methods; the default `write_blocks` answers kind
`ReadOnly`, and a device whose `writable()` is false is mounted read-only.

| Device | Use |
|---|---|
| `hadris_storage::host::FileDevice` (`std`, `sync`) | A host image file or disk device, 512-byte blocks. `FileDevice::open(path)` is read-only; `FileDevice::new(file)` takes a file you opened, writable if it was opened for writing. Also `hadris::host::FileDevice`. `std::fs::File` is no longer a device. |
| `Vec<u8>` (`alloc`) | In-memory image with 512-byte blocks that grows when written past its end. |
| `hadris_storage::MemDevice<B>` | `&[u8]` (read-only), `&mut [u8]`, arrays, `Vec<u8>` or `Box<[u8]>` with an explicit `BlockSize`. |
| `hadris_storage::{sync, r#async}::StreamDevice<T>` | Any `Read + Seek` (+ `Write`) stream as a device with the block size you pick. The migration path for V2 code that passed a stream. Replaces `SeekBlockDevice`. |
| `hadris_storage::Partition<D>` | A byte window of a device. Replaces `PartitionView`. `hadris_part::sync::open` returns one. |
| `hadris_storage::{sync, r#async}::Cache<D>` (`alloc`) | Write-back LRU block cache for any device. Replaces the FAT sector cache. |
| `hadris_storage::{sync, r#async}::ByteView<D>` | Byte-granular reads and writes over a device. |
| `&mut D`, `Box<D>` | Borrow or box a device instead of moving it in. |

Replacing a V2 `Read + Seek` stream:

```rust
// 2.4
let image = std::fs::File::open("disk.img")?;
let volume = hadris_fat::FatVolume::open(image)?;
```

```rust
// 3.0
use hadris_storage::host::FileDevice;
let image = FileDevice::open("disk.img")?;
let fs = hadris_fat::sync::FatFs::mount(image, hadris_fs::MountOptions::new().read_only())?;
```

Bytes in memory: `MemDevice::new(bytes, BlockSize::new(512).unwrap())`. Any
other stream: `StreamDevice::new(StdIo::new(stream), BlockSize::new(512).unwrap())?`.

`BlockIndex` and `BlockCount` have private fields: build them with `new(n)`
and read them with `get()`. `BlockRange` and `BlockGeometry` have accessors
instead of public fields. Implementing a device for firmware is shown in
[Adapt a custom device](../website/docs/guides/custom-io.md).

## Filesystems

### The trait and the volume

Every driver implements `hadris_fs::sync::FileSystem` (and the
`r#async` twin): `FatFs`, `ExFatFs`, `IsoFs`, `UdfFs`, `NtfsFs` and
`hadris::sync::AnyFs`. The trait works on node ids and takes `&mut self`:

| Group | Methods |
|---|---|
| Volume | `capabilities`, `root`, `statfs`, `label(&mut buf)` |
| Names and nodes | `lookup`, `forget(node, count)`, `parent`, `resolve(path, Resolve)`, `stat`, `readdir(dir, cursor)`, `readlink` |
| Data | `open(node, OpenMode)`, `close`, `read(node, offset, buf)` |
| Writes (default `ReadOnly`) | `setattr`, `write`, `truncate`, `fsync`, `create`, `mkdir`, `unlink`, `rmdir`, `rename(.., RenameMode)`, `sync` |

`lookup`, `parent`, `resolve`, `create` and `mkdir` pin the node they return;
`forget` unpins it. A pin never blocks removal; removing the last name of an
open node fails with `Busy`. Node ids (`NodeId`, a non-zero `u64`) replace
V2's `FileEntry`, `DirectoryRef` and `UdfDirEntry` snapshots, so a renamed
file keeps its id and a handle keeps working.

`hadris_fs::sync::Volume<F>` wraps any driver behind a lock and adds path
methods named after `std::fs`: `open`, `metadata`, `symlink_metadata`,
`read_dir`, `read_link`, `create_dir`, `create_dir_all`, `remove_file`,
`remove_dir`, `remove_dir_all`, `rename` and `set_attr`, plus `lock()` for
node and format-specific calls and `into_inner()`. `File` has `read`,
`write`, `seek`, `set_len`, `metadata`, `sync_all` and `close(self)`, and in
`sync` implements `std::io::{Read, Write, Seek}`. `ReadDir` is an
`Iterator` in `sync` and has `next_entry()` in `r#async`. The sync `Volume`
needs `std`; the async one needs `alloc`. `Volume::with_resolve(fs,
Resolve::Follow)` gives POSIX path semantics; the default is lexical.

`Walk::new(dir)` (`alloc`) or `Walk::with_stack(dir, &mut frames)` lists a
tree depth first on the bare driver.

### Mounting

Each driver mounts with `mount(dev, MountOptions)` and gives the device back
with `unmount()` (syncs first) or `into_inner()` (does not). A failed mount
returns `MountError`, which also gives the device back.

`hadris_fs::MountOptions` replaces `FatVolumeBuilder`, the FAT time and
code-page providers and the per-format open functions:

| Method | Meaning |
|---|---|
| `new()` | Read-write where the device allows, UTC, CP437, `NoClock` (1980-01-01), no node cap, on every target |
| `read_only()` | Never calls `write_blocks` |
| `with_clock(&'static dyn Clock)` | `NoClock` or `SystemClock` (`std`) or your own |
| `with_utc_offset(minutes)` | Zone of FAT timestamps; returns `Result` |
| `with_code_page(&'static dyn CodePage)` | `Cp437` (default) or `Ascii` |
| `with_node_limit(n)` | Cap pinned and open nodes of `FatFs` and `ExFatFs` |
| `backup_boot()` | Mount FAT32 or exFAT read-only from the backup boot region; UDF reads the end anchors first |

`hadris_fs::host::mount_options()` is the host default: the system clock and
the local UTC offset.

### Before and after

```rust
// 2.4 (website/docs/guides/read-fat-image.md on main)
let image = File::open("disk.img").context("open disk.img")?;
let volume = FatVolume::open(image).context("open FAT filesystem")?;
let root = volume.root_dir();
let mut entries = root.entries();
while let Some(entry) = entries.next_entry() {
    let file = entry?.as_entry().context("unsupported directory record")?;
    println!("{} {}", file.len(), file.name());
}
if let Some(readme) = root.find("README.TXT")? {
    let mut reader = volume.read_file(&readme)?;
    let mut contents = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut contents)?;
}
```

```rust
// 3.0 (website/docs/guides/read-fat-image.md)
use std::io::Read;
use hadris_fat::sync::FatFs;
use hadris_fs::sync::Volume;
use hadris_fs::{MountOptions, OpenOptions};
use hadris_storage::host::FileDevice;

let image = FileDevice::open("disk.img")?;
let vol = Volume::new(FatFs::mount(image, MountOptions::new().read_only())?);
for entry in vol.read_dir("/")? {
    let entry = entry?;
    println!("{} {:?}", entry.metadata().len(), entry.name());
}
let mut contents = Vec::new();
vol.open("/README.TXT", OpenOptions::new().read())?.read_to_end(&mut contents)?;
```

Writing:

```rust
// 2.4
let volume = FatVolume::builder(image).fat_cache(16).open()?;
let root = volume.root_dir();
let entry = volume.create_file(&root, "hello.txt")?;
let mut writer = volume.write_file(&entry)?;
writer.write_all(b"Hello from Hadris\n")?;
writer.finish()?;
volume.flush()?;
```

```rust
// 3.0 (website/docs/guides/modify-fat.md)
let device = Cache::new(FileDevice::new(image)?, 64);
let vol = Volume::new(FatFs::mount(device, MountOptions::new().with_clock(&SystemClock))?);
let mut file = vol.open("/hello.txt", OpenOptions::new().write().create().truncate())?;
file.write_all(b"Hello from Hadris\n")?;
file.close()?;
vol.lock().sync()?;
```

The bare driver without a `Volume`:

```rust
// 3.0 (crates/block/hadris-fat/README.md)
let mut fs = FatFs::mount(dev, MountOptions::new())?;
let root = fs.root();
let efi = fs.lookup(root, Name::new("efi"))?;
fs.forget(efi, 1);
```

Common V2 calls:

| V2 | V3 |
|---|---|
| `FatVolume::open(stream)`, `FatVolume::builder(stream)...open()` | `FatFs::mount(dev, MountOptions)` |
| `IsoImage::open(stream)`, `IsoReader::open(dev)` | `IsoFs::mount(dev, MountOptions)`; `IsoFs::mount_namespace(dev, options, Namespace)` to pick a tree |
| `UdfVolume::open(stream)` | `UdfFs::mount(dev, MountOptions)` |
| `NtfsFs::open(stream)` | `NtfsFs::mount(dev, MountOptions)` |
| `ExFatVolume` (preview) | `hadris_fat::exfat::sync::ExFatFs::mount(dev, MountOptions)` |
| `root_dir()`, `entries()`, `next_entry()` | `vol.read_dir(path)`, or `fs.readdir(dir, cursor)` with `DirEntry::next_cursor()` |
| `find(name)`, `open_path(path)`, `find_path(path)` | `fs.lookup(dir, Name::new(name))`, `fs.resolve(path, Resolve::Lexical)`, `vol.metadata(path)` |
| `read_file(&entry)`, `open_file`, `FileReader`, `IsoFileReader` | `vol.open(path, OpenOptions::new().read())` returning `File`, or `fs.open`, `fs.read(node, offset, buf)`, `fs.close` |
| `write_file(&entry)` then `FileWriter::finish()` | `File::write` then `File::close()`; `sync_all()` for durability |
| `create_file`, `create_dir` | `vol.open(path, OpenOptions::new().write().create())`, `vol.create_dir`, `vol.create_dir_all`; `fs.create`, `fs.mkdir` |
| `delete` | `vol.remove_file`, `vol.remove_dir`, `vol.remove_dir_all`; `fs.unlink`, `fs.rmdir` |
| `rename` | `vol.rename(from, to)` (replaces an existing target); `fs.rename(.., RenameMode)` |
| `truncate`, `set_times`, `set_attributes` | `File::set_len`, `vol.set_attr(path, &SetAttr)`; `fs.truncate`, `fs.setattr` |
| `flush()`, `sync()` | `fs.sync()` or `vol.lock().sync()`; `File::sync_all()` for one file |
| `into_inner()` | `unmount()` (syncs) or `into_inner()`; on a volume, `vol.into_inner()` first |
| `volume_info()`, `fat_type()`, `read_root_label()` | `fs.info()` (`Geometry`: `kind()`, `volume_serial()`, `cluster_size()`), `fs.label(&mut buf)` |
| `set_root_label` | `fs.set_label(Some(VolumeLabel::new("DATA")?))` |
| `read_status_flags()` | `fs.was_dirty()` |
| `get_cluster_chain`, `fragmentation_report` | `fs.extents(node, from, &mut [Extent])` |
| `verify()` | `hadris_fat::sync::check(&mut dev, &mut scratch, on_finding)` on the unmounted device |
| `IsoImage::read_bytes_at`, `UdfVolume` raw reads | `read_raw(offset, buf)` |
| `IsoImage::read_pvd`, `read_volume_descriptors` | `IsoFs::info()`, `IsoFs::descriptor(index)` |
| `UdfVolume::info()` (`UdfVolumeInfo`) | `UdfFs::info()` (`VolumeInfo`: `id(UdfId)`, `revision`, `block_size`, `partitions`, `volume_serial`) |

Format extras are inherent methods with the same names on every driver:
`info()`, `extents(node, from, &mut out)`, `records(node, &mut out)` and
`read_raw(offset, buf)`. FAT and exFAT add `was_dirty`, `set_label` and
`set_volume_serial`; ISO adds `boot_catalog(&mut buf)`, `boot_image(&entry)`,
`rock_ridge(node)`, `namespaces()` and `descriptor(index)`; UDF adds
`was_dirty`; NTFS keeps `volume_serial`, `cluster_size`, `sector_size`,
`total_sectors`, `mft_record_size`, `index_record_size`, `streams` and
`read_stream_at`. On a `Volume`, reach them through `vol.lock()`.

Behaviour to check after porting:

- FAT short names default to CP437 (V2 read them as lossy ASCII). Pass
  `with_code_page(&Ascii)` to keep ASCII; bytes above `0x7F` then read as
  `U+F700 + b`.
- FAT timestamps are read in the zone `with_utc_offset` names, UTC by default.
  `host::mount_options()` uses the local offset, as Windows and Linux do.
- `FatFs` compares names by folding UTF-16 units as Windows does.
- A device that is not `writable()` mounts read-only; opening a file for
  writing then fails with `ReadOnly`.
- Closing a `File` publishes its size without flushing the device. Call
  `sync` (or `File::sync_all`) before removing media.

### exFAT

exFAT is stable and needs no feature. `hadris_fat::exfat::{sync, r#async}::ExFatFs`
mounts with the same `MountOptions`, and `hadris_fat::exfat` holds
`ExFatOptions`, `VolumeLabel`, `Geometry` and `Detail`. The V2 preview types
(`ExFatVolume`, `ExFatDir`, `ExFatFileReader`, `format_exfat`, ...) are
mapped in the [hadris-fat table](#hadris-fat).

## Builders and writers

### One input tree

Every writer takes a `hadris_fs::Tree` (`alloc`):

```rust
// 3.0
use hadris_fs::{Content, Node, Tree};

let mut tree = Tree::new();
tree.insert("README.TXT", Node::file(Content::bytes("Hello from Hadris\n")))?;
tree.insert("DOCS/GUIDE.TXT", Node::file(Content::bytes("Getting started\n")))?;
tree.insert("latest", Node::symlink("README.TXT"))?;
```

- `Tree` has `new`, `insert` (creates missing parents), `link` (hard link),
  `remove`, `replace`, `get`, `entry` and `root`. Paths are `/`-separated
  bytes; `.` and `..` are refused.
- `Node` is `file(Content)`, `dir()`, `symlink(target)` or
  `special(FileType, Option<DeviceNumber>)`, with `with_attrs(SetAttr)` for
  permissions, owner, times and DOS attributes.
- `Content` is `Content::bytes(..)`, `Content::empty()`, a host file from
  `hadris_fs::host::file(path)`, or content read lazily from a mounted volume
  by `read_tree`. `Content::stored(extents)?` represents bytes already on the
  device used by a session or bridge. It is fallible: device-end/total-length
  overflow is `InvalidInput`, unwritten extents are `Unsupported`. Extents
  concatenate in supplied order; their file offsets are ignored.
- `hadris_fs::host::read_tree(dir, &TreeOptions)` builds a tree from a host
  directory and returns the errors it skipped; `TreeOptions` sets the symlink
  policy (`Symlinks`), the error policy (`OnError`), an exclude filter, an
  owner override and an mtime clamp. `hadris_fs::host::write_tree(dir, &tree)`
  extracts a tree and never writes outside `dir`.
- `hadris_fs::sync::read_tree(&vol, path)` reads a mounted volume into a tree
  with lazy content; `hadris_fs::sync::copy_tree(&tree, &mut fs, dir)` copies
  a tree into any mounted filesystem.

These replace `InputTree`, `InputFiles`, `InputEntry`, `InputMetadata`,
`InputTree::from_fs` (ISO), `SimpleDir` and `SimpleFile` (UDF), `FileTree`,
`Directory`, `FileEntry` and `FileData` (hadris-cd), `FileTree` and
`FileTree::from_fs` (cpio), and the host helpers `extract_to_host` and
`import_from_host` of the V3 previews.

### plan, write and Report

Each writer has `plan(&tree, &options)` at the crate root (no I/O) and
`write(dev, &tree, &options)` in each mode. Both return a `hadris_fs::Report`
with `size()`, `warnings()` (each a `Warning` with a `WarningKind`),
`extents(path)` and `files()`. Writers take `with_time(DateTime)` and
`with_seed(u64)` instead of a clock, so the same input gives the same bytes;
the default time is 1980-01-01. For reproducible builds pass
`hadris_fs::host::source_date_epoch()`.

| Format | plan | write | Options |
|---|---|---|---|
| ISO 9660 | `hadris_iso::plan` | `hadris_iso::{sync, r#async}::write` | `hadris_iso::IsoOptions` |
| UDF | `hadris_udf::plan` | `hadris_udf::{sync, r#async}::write` | `hadris_udf::UdfOptions` |
| ISO 9660 and UDF bridge | `hadris_udf::plan_bridge` | `hadris_udf::{sync, r#async}::write_bridge` | `IsoOptions` and `UdfOptions` |
| cpio | `hadris_cpio::plan` | `hadris_cpio::{sync, r#async}::write` | `hadris_cpio::CpioOptions` |
| FAT12/16/32 | none | `hadris_fat::{sync, r#async}::write` | `hadris_fat::FatOptions` |
| exFAT | none | `hadris_fat::exfat::{sync, r#async}::write` | `hadris_fat::exfat::ExFatOptions` |

The output of ISO, UDF, bridge and FAT writers is a block device.
`FileDevice` over a host file and `Vec<u8>` grow as needed; a fixed device is
checked against `plan`'s size before anything is written.

### ISO 9660

```rust
// 2.4 (website/docs/creation/iso.md on main)
let tree = InputTree::new(PathSeparator::ForwardSlash, vec![
    InputEntry::file("README.TXT", b"Hello from Hadris\n"),
]);
let options = IsoFormatOptions {
    volume_name: "HADRIS_DEMO".into(),
    preparer_id: Some("HADRIS".into()),
    sector_size: 2048,
    features: CreationFeatures::default(),
    path_separator: PathSeparator::ForwardSlash,
    strict_charset: true,
    ..
};
IsoImageWriter::create(image, tree, options)?;
```

```rust
// 3.0 (website/docs/creation/iso.md)
let options = IsoOptions::default()
    .with_id(IsoId::Volume, "HADRIS_DEMO")
    .with_id(IsoId::Preparer, "HADRIS");
let image = hadris_storage::host::FileDevice::new(image)?;
let report = hadris_iso::sync::write(image, &tree, &options)?;
```

Option reshape:

| V2 | V3 |
|---|---|
| `IsoFormatOptions { volume_name, system_id, volume_set_id, publisher_id, preparer_id, application_id, .. }` | `IsoOptions::with_id(IsoId::{Volume, System, VolumeSet, Publisher, Preparer, Application, CopyrightFile, AbstractFile, BibliographicFile}, &str)`; dates with `with_date(IsoDate, DateTime)` |
| `sector_size`, `path_separator` | Removed. Paths are always `/`-separated. |
| `strict_charset` | Removed. Identifiers are stored as given and only their length is checked. |
| `CreationFeatures::filenames: BaseIsoLevel::{Level1, Level2, ..}` | `with_level(IsoLevel::{L1, L2, L3})` and `with_name_case(NameCase::{Upper, Preserve})` |
| `supports_rrip`, `CreationFeatures::rock_ridge`, `RripOptions` | `with_rock_ridge()`, `with_preserve(Preserve)`, `with_relocation(Relocation::{RrMoved, DotRrMoved, Refuse})` |
| `joliet: Some(JolietLevel::Level3)` | `with_joliet()` (writes level 3) |
| `long_filenames: true` | `with_iso1999()` |
| `el_torito: Some(BootOptions { default: BootEntryOptions { .. }, entries })` | `with_el_torito(ElTorito::new().with_entry(BootEntry::bios(path)).with_entry(BootEntry::uefi(path)))` |
| `BootEntryOptions::load_size`, `emulation`, `boot_info_table`, `grub2_boot_info` | `BootEntry::with_load_size`, `with_emulation(Emulation)`, `with_boot_info(BootInfo::Table)`, `with_boot_info(BootInfo::Grub2)` |
| `BootOptions::write_boot_catalog` | `ElTorito::with_catalog_path(path)` makes the catalog visible as a file |
| `hybrid_boot: Some(HybridBootOptions { .. })`, `PartitionScheme` | `with_hybrid(Hybrid::mbr() / Hybrid::gpt() / Hybrid::gpt_hybrid_mbr())`, `with_bootstrap(&[u8])` |
| `HybridBootOptions::with_efi_boot_partition`, `efi_boot_partition` | `Hybrid::with_appended(AppendedPartition::esp(content))` and `BootEntry::uefi_appended(0)` |
| `IsoImageWriter::create_with_allocation_floor` | `IsoOptions::with_min_blocks` |
| `estimator::estimate`, `estimate_tree` | `hadris_iso::plan(&tree, &options)?.size()` |
| `IsoModifier` with `ModifyOp` | `hadris_iso::sync::Session::open(dev)`, `tree_mut()`, `write(&options, SessionMode::{Append, Rewrite})` |

To remaster an edited session to a new device, call
`session.export(out, &options)?`. It streams original stored file extents from
the session device with bounded memory and reads new content normally.
`write(out, session.tree(), &options)` cannot read the original device and
still rejects stored content. The output must not alias the source. Choose
boot and partition settings explicitly for the new image; export does not
automatically preserve the original boot catalog or partition tables.

Bootable and hybrid images: [Create an ISO](../website/docs/creation/iso.md).

### UDF and the bridge

```rust
// 2.4 (website/docs/creation/udf.md on main)
let mut root = SimpleDir::root();
root.add_file(SimpleFile::new("README.txt", b"Hello from a UDF image\n".to_vec()));
let output = UdfWriter::create(target, &root, UdfWriteOptions::default())?;
```

```rust
// 3.0 (website/docs/creation/udf.md)
let target = hadris_storage::host::FileDevice::new(std::fs::File::create("volume.udf")?)?;
let report = hadris_udf::sync::write(target, &tree, &UdfOptions::default())?;
```

`UdfWriteOptions { volume_id, revision, .. }` becomes
`UdfOptions::default().with_id(UdfId::Volume, ..).with_revision(UdfRevision::V2_01)`.
`UdfId::{Volume, VolumeSet, LogicalVolume, FileSet}` set the identifiers
separately; an identifier too long for its field fails with
`Detail::Identifier` instead of being cut.

The `hadris-cd` writer is replaced by the bridge writer in `hadris-udf`:

```rust
// 2.4 (crates/optical/hadris-cd/README.md on main)
let options = OpticalImageOptions::default().volume_id("MY_DISC")
    .joliet(hadris_cd::JolietLevel::Level3);
OpticalImageWriter::new(file, options).finish(tree)?;
```

```rust
// 3.0
let iso = IsoOptions::default().with_id(IsoId::Volume, "MY_DISC").with_joliet();
let udf = UdfOptions::default().with_id(UdfId::Volume, "MY_DISC");
let report = hadris_udf::sync::write_bridge(dev, &tree, &iso, &udf)?;
```

The output no longer has to be readable: file positions come from the ISO
report instead of reading the image back. The `iso_only` and `udf_only`
switches are gone; call `hadris_iso::sync::write` or
`hadris_udf::sync::write` instead.

### cpio

`CpioReader::new` owns a default name buffer. Use
`with_buffer(reader, buffer, options)` for caller-owned storage implementing
`AsRef<[u8]> + AsMut<[u8]>` (also `Send` for async). Capacity includes the
terminating NUL; an encoded name exceeding it returns `LimitExceeded`.
Entries expose byte paths through `path()` and UTF-8 through `path_str()`.
`offset()` and `data_offset()` count from the supplied stream's start.

After `next_entry()` returns `None`, `next_segment()` skips zero padding and
returns whether bytes follow. It replaces `at_trailer()` and
`continue_after_trailer()`. The next `next_entry()` validates the header.
Calling `next_segment()` before a segment ends returns `InvalidInput`.
To recover the stream after probing, `into_parts()` returns the stream,
name buffer and optional peeked byte, which must be replayed first.
`into_inner()` returns only the stream and discards that byte.

```rust
// 2.4 (website/docs/guides/cpio-archives.md on main)
let tree = FileTree::from_fs(Path::new("./root"))?;
let output = BufWriter::new(File::create("archive.cpio")?);
CpioArchiveWriter::new(output, CpioWriteOptions::default()).finish(&tree)?;
```

```rust
// 3.0 (website/docs/guides/cpio-archives.md)
let (tree, _) = host::read_tree("./root", &TreeOptions::new())?;
let mut output = StdIo::new(BufWriter::new(File::create("archive.cpio")?));
let options = CpioOptions::default().with_format(Format::Crc);
let report = hadris_cpio::sync::write(&mut output, &tree, &options)?;
```

`CpioWriteOptions::crc(bool)` becomes `CpioOptions::with_format(Format::{Newc,
Crc, Odc})`. `hadris_cpio::sync::Writer` writes entries one at a time
(`append(path, &Node)`, `append_hard_links`, `append_file` with an
`EntryWriter`, `finish`). `hadris_cpio::sync::read_tree(&mut reader)` reads an
archive back into a `Tree`.

### FAT and exFAT

```rust
// 2.4 (website/docs/creation/fat.md on main)
let options = FatFormatOptions::new(SIZE)
    .volume_label("HADRIS")
    .fat_type(FatTypeSelection::Fat16);
let fs = FatVolumeFormatter::format(image, options)?;
```

```rust
// 3.0 (website/docs/creation/fat.md)
let options = FatOptions::new()
    .with_kind(FatKind::Fat16)
    .with_label(VolumeLabel::new("HADRIS")?);
let mut dev = FileDevice::new(image)?;
format(&mut dev, &options)?;             // returns the Geometry
let mut fs = FatFs::mount(dev, MountOptions::new())?;
```

- `format(&mut dev, &opts)` borrows the device, returns `Geometry` and needs
  no allocator. Mount afterwards with the options you want.
- `FatFormatOptions::new(size)` becomes `FatOptions::new()`: the volume fills
  the device unless `with_size` says otherwise. `volume_label` is
  `with_label(VolumeLabel)`, `fat_type(FatTypeSelection)` is
  `with_kind(FatKind)`, `volume_id` is `with_serial`, `sectors_per_cluster`
  is `with_cluster_size` (bytes), `fat_copies` is `with_fat_count`,
  `media_type` is `with_media(u8)`, `sector_size(SectorSize)` is
  `with_sector_size(u32)`. `hidden_sectors` is gone: the hidden sectors
  default to the device's `disk_offset`, so formatting a `Partition` records
  its start; `with_partition_offset(bytes)` overrides it.
- The serial derives from `with_seed` or `with_time`, not the clock.
- `FatVolumeFormatter::calculate_params` is `hadris_fat_raw::layout::plan`.
- `write(dev, &tree, &FatOptions)` formats and copies a tree in one call.
- exFAT: `ExFatFormatOptions` and `format_exfat` become
  `hadris_fat::exfat::ExFatOptions` and `hadris_fat::exfat::sync::format`.

## Detection and opening

`hadris-block` and `hadris-optical` are replaced by the umbrella's `detect`
feature (on by default).

```rust
// 2.4 (website/docs/guides/detect-open-images.md on main)
let format = detect::sync::detect(&mut image, 512)?;
match format {
    Some(detect::BlockFormat::Fat(_)) => {
        let opened = OpenVolume::open(&mut image, 512)?;
        let fat = opened.as_fat().expect("the detector reported FAT");
    }
    Some(detect::BlockFormat::PartitionTable(kind)) => { /* ... */ }
    None => {}
}
let opened = OpenOpticalImage::open(&mut image, OpenPolicy::PreferUdf)?;
```

```rust
// 3.0 (website/docs/guides/detect-open-images.md)
let mut image = hadris::host::FileDevice::open("disk.img")?;
let found = hadris::sync::detect(&mut image)?;
for candidate in found.iter() {
    match candidate.damage() {
        Some(err) => println!("{:?}, damaged: {err}", candidate.format()),
        None => println!("{:?}", candidate.format()),
    }
}

let fs = hadris::host::open("disc.img")?;          // read-only AnyFs<FileDevice>
if let AnyFs::Udf(udf) = &fs {
    println!("UDF revision {:?}", udf.info().revision());
}
let vol = Volume::new(fs);
```

- `hadris::sync::detect(&mut dev)` returns a `Detection` of every format the
  device holds, most specific first, without allocating. `Detection::first()`
  and `iter()` give `Candidate`s with `format()` and `damage()`. The block
  size is the device's; there is no block size argument.
- `hadris::ImageFormat` (`#[non_exhaustive]`): `Fat(FatKind)`, `ExFat`,
  `Iso`, `Udf`, `IsoUdfBridge`, `Cpio(Format)`, `Mbr`, `Gpt`, `Ntfs`. It
  replaces `BlockFormat`, `FatVariant`, `PartitionTableKind`,
  `OpticalFormat` and `OpticalFormats`.
- `hadris::sync::open(dev, MountOptions)` (`alloc`) mounts the first
  filesystem found as `hadris::sync::AnyFs` (`Fat`, `ExFat`, `Iso`, `Udf`),
  which implements `FileSystem`, and has `unmount` and `into_inner`. Reach a
  driver's extras by `match`. NTFS, partition tables and archives fail with
  `ErrorKind::NotRecognized`, giving the device back.
- `hadris::host::open(path)` opens an image read-only with
  `host::mount_options()`.
- `OpenPolicy` has no successor. A bridge image opens as UDF; mount
  `hadris_iso::sync::IsoFs` directly to read its ISO 9660 side, and use
  `IsoFs::mount_namespace` to pick a namespace.
- `hadris::r#async::{detect, open, AnyFs}` are the async forms.

More in [Detect and open images](../website/docs/guides/detect-open-images.md).

## Partitions

`hadris-part` reads and writes tables on a block device, with the block size
taken from the device. `Disk` holds a `PartitionTable` (`Mbr`, `Gpt` or
`Hybrid`) and does no I/O; each mode has `read`, `write`, `create`, `open`
and `scan`.

```rust
// 2.4 (website/docs/guides/read-partition-table.md on main)
let mut disk = File::open("disk.img")?;
let table = PartitionTable::read_from(&mut disk, 512)?;
for partition in table.partitions() {
    println!("#{}: LBA {} ({} sectors)", partition.index, partition.start_lba, partition.size_sectors);
}
```

```rust
// 3.0 (website/docs/guides/read-partition-table.md)
let mut disk = FileDevice::open("disk.img")?;
let table = hadris_part::sync::read(&mut disk)?;
for partition in table.partitions() {
    println!("#{}: block {} ({} blocks, {} bytes)",
        partition.index(), partition.start(), partition.len(), partition.size_bytes());
}
```

Opening a filesystem inside a partition:

```rust
// 2.4 (website/docs/guides/open-partitioned-fat.md on main)
let mut view = PartitionView::new(&mut disk, byte_offset, byte_len)?;
let opened = OpenVolume::open(&mut view, BLOCK_SIZE)?;
let fat = opened.as_fat().context("the selected partition is not FAT")?;
```

```rust
// 3.0 (website/docs/guides/open-partitioned-fat.md)
let table = part::sync::read(&mut disk)?;
let partition = table.partition(0).context("the disk has no partitions")?;
let slice = part::sync::open(&mut disk, &partition)?;
let opened = hadris::sync::open(slice, hadris::host::mount_options())
    .map_err(|err| err.into_error())?;
let AnyFs::Fat(fat) = opened else { anyhow::bail!("not FAT") };
```

Creating a disk:

```rust
// 3.0 (docs/v3-api-design.md, 5.6)
let layout = DiskLayout::gpt(disk_guid)
    .with_alignment(Alignment::MiB1)
    .partition(PartitionSpec::new(gpt::types::EFI_SYSTEM, Size::MiB(100)).with_name("EFI"))
    .partition(PartitionSpec::new(gpt::types::LINUX_FILESYSTEM, Size::Remaining));
let disk = hadris_part::sync::create(&mut dev, &layout)?;
```

Other changes:

- No GUID is made at random. `DiskLayout::gpt(disk_guid)`, `Gpt::new` and
  `GptEntry::new` take GUIDs; `Guid::random()` (`std`) makes one on request.
  `Guid` implements `FromStr`; the old inherent `from_str` is `parse_const`,
  and `Guid::UNUSED` is `Guid::NIL`. Type GUIDs moved from `Guid::EFI_SYSTEM`
  to `gpt::types::EFI_SYSTEM`.
- `MbrType(u8)` with constants replaces `MbrPartitionType` and
  `MbrPartitionTypeFull`. `0x04` is `FAT16_SMALL`, `0x06` is `FAT16` and
  `0x0E` is `FAT16_LBA`; the old enum called `0x04` `Fat16` and `0x06`
  `Fat16Lba`.
- `bootable: bool` parameters became `PartitionFlags`.
- Edits (`add`, `add_logical`, `remove`, `resize`, `set_*`) check bounds and
  overlap and return `TableError`; nothing changes on failure.
- MBR logical partitions in EBR chains are read and written (indices from 4).
  A GPT whose primary copy is damaged is read from the backup, and
  `Gpt::damaged_copy()` says which one; `write` repairs it.
- Block sizes must be powers of two of at least 512 bytes.

### Failed mutations

`FileSystem` mutations are not transactions. Drivers validate arguments and
known read-only or unsupported requests before mutation; those rejections
leave logical contents and metadata unchanged. Once mutation starts, device
errors, changing write protection and cancellation may leave partial changes.
An error kind alone does not establish whether anything changed.

`write` reports confirmed progress as `Ok(n)`. `Err` has no reliable byte count
and does not mean zero bytes were written; blindly retrying can be incorrect.
`fsync`/`sync` are still required for durability. Compound helpers may have
completed earlier steps before a later step fails. Node-pin cleanup on async
cancellation is a separate requirement and does not imply rollback.

## Embedded API

V2 firmware ran `FatVolume` without `alloc`. In V3 `FatFs` and `ExFatFs`
need `alloc` for their node table, and firmware without an allocator uses a
separate, handle-based API built on `hadris-fat-raw`:

| Type | Does |
|---|---|
| `hadris_fat::embedded::{sync, r#async}::Fat<'mount, D, const FILES: usize = 4>` | FAT12/16/32 read and write |
| `hadris_fat::exfat::embedded::{sync, r#async}::ExFat<'mount, D, const FILES: usize = 4>` | exFAT read only |
| `hadris_fat::embedded::{MountToken, Dir, File, Entry, Node, Options}` | Shared handles and options (`exfat::embedded` has its own `Dir`, `Entry`, `Node`) |

```rust
// 3.0 (website/docs/guides/embedded.md)
use hadris_fat::embedded::{MountToken, Options, sync::Fat};
use hadris_fs::{DirCursor, OpenOptions};

let mut token = MountToken::new();
let mut fat: Fat<'_, _> = Fat::mount_with(card, &mut token, Options::new())?;
let logs = fat.create_dir_all(fat.root(), "data/logs")?;
let log = fat.open(logs, "boot.txt", OpenOptions::new().write().create().append())?;
fat.write(&log, b"booted\n")?;
fat.close(log)?;
let card = fat.unmount()?;
```

- The device must have 512-byte blocks. Sync takes
  `hadris_storage::sync::BlockDevice`; async takes
  `hadris_storage::local::BlockDevice`, whose futures need not be `Send`.
- `Options` has `with_clock(fn() -> DateTime)`, `with_utc_offset`,
  `with_code_page`, `read_only` and `with_fold`. Names fold ASCII case by
  default; `with_fold(hadris_fat_raw::fold_unicode)` compares as `FatFs`
  does.
- Each mount requires its own `MountToken`, passed by exclusive borrow.
  `File<'mount>` retains that identity even after unmount; Rust prevents reusing
  the token while such a file can still be used. A foreign handle returns
  `InvalidHandle`. `close` consumes a file; `list(dir, cursor, callback)` lends
  each `Entry`. Mounts and files remain allocation-free and need no atomics.
- `format` and `check` in `hadris_fat::sync` and `hadris_fat::exfat::sync`
  need no allocator and work with this API.

`hadris-fat-raw` holds what `hadris_fat::raw` held in V2 and more: boot
sector parsing (`parse_boot`, `Geometry`), directory entry layouts
(`RawDirEntry`, `RawLfnEntry`, `ShortEntry`, `LongEntry`, `Slot`), the
`lfn`, `short_name`, `name`, `date` and `layout` codecs, the device
primitives in `io` and `exfat::io`, and the checkers `io::sync::check` and
`exfat::io::sync::check`. Flash and stack figures are in
[Use FAT and exFAT on a microcontroller](../website/docs/guides/embedded.md).

## Async

- `r#async` futures are `Send` when the device is. A device whose error or
  futures are not `Send` uses the `local` traits (`hadris_io::local`,
  `hadris_storage::local`) and the embedded API.
- Device errors must be `core::error::Error + Send + Sync + 'static` in the
  shared tier.
- The async `ReadDir` has `next_entry().await` instead of `Iterator`.
- The async `Volume` needs `alloc`; the sync one needs `std`.
- The `hadris_fs::host` module is sync only. Lazy content is readable only in
  the mode that produced it; an async writer given a host file fails with
  `Unsupported` before writing.

```rust
// 2.4 (website/docs/guides/async-io.md on main)
let volume = FatVolume::open(Cursor::new(image)).await?;
```

```rust
// 3.0 (website/docs/guides/async-io.md)
let dev = MemDevice::new(image, BlockSize::new(512).unwrap());
let mut volume = hadris_fat::r#async::FatFs::mount(dev, MountOptions::new()).await?;
```

More in [Use asynchronous I/O](../website/docs/guides/async-io.md).

## Command-line tools

The five CLI packages are replaced by one binary, `hadris`, in the
`hadris-cli` package:

```bash
cargo uninstall hadris-fat-cli hadris-iso-cli hadris-udf-cli hadris-cpio-cli hadris-cd-cli
cargo install hadris-cli
```

The 2.x binaries (`hadris-fat`, `fatutil`, `hadris-iso`, `hadris-iso-cli`,
`hadris-udf`, `hadris-udf-cli`, `hadris-cpio`, `cpioutil`, `hadris-cd`) are
not installed any more. The alias binaries ran the same commands as their
main binary, so they map the same way.

### Commands

| V2 command | V3 command |
|---|---|
| `hadris-fat` or `fatutil` `info`, `stat`, `ls`, `tree`, `cat`, `extract`, `create`, `verify`, `fragmentation`, `chain` | `hadris fat` with the same subcommand, for example `hadris-fat ls disk.img /EFI` becomes `hadris fat ls disk.img /EFI` |
| `hadris-iso` or `hadris-iso-cli` `info`, `ls`, `tree`, `cat`, `extract`, `create`, `verify` | `hadris iso` with the same subcommand |
| `hadris-iso mkisofs` (alias `xorriso`) | `hadris iso mkisofs` (alias `xorriso`) |
| `hadris-udf` or `hadris-udf-cli` `info`, `ls`, `tree`, `cat`, `extract`, `create`, `verify` | `hadris udf` with the same subcommand |
| `hadris-cpio` or `cpioutil` `ls` (alias `list`), `info`, `cat`, `extract`, `create` | `hadris cpio` with the same subcommand |
| `hadris-cd create SRC -o OUT` | `hadris udf bridge SRC -o OUT` |
| `hadris-cd verify IMG` | `hadris udf compare IMG` |
| `hadris-cd info IMG` | No direct successor. Use `hadris detect IMG`, `hadris iso info IMG` and `hadris udf info IMG`. |
| (none) | `hadris detect IMG` lists every format an image or device holds, one per line, most specific first, marking damaged ones; it exits 1 when nothing is recognized. |

In every format, `list` is an alias of `ls` and `check` of `verify`.

### Flags

| Command | V2 | V3 |
|---|---|---|
| every `create`, `udf bridge` | Existing output replaced silently (FAT refused it) | Refused unless `-f/--force`; a file is then replaced atomically once the image is complete, a device is written in place. A failed `create` leaves no partial output. |
| `iso mkisofs` | same | `--force` (long form only) |
| `fat create` | `-V/--volume-label` | `-V/--volume-name`; `--volume-label` still works as an alias. `--fat-type` gains `exfat`. Labels longer than 11 ASCII characters are refused. |
| `fat extract` | `-o/--output` required | `-o/--output` defaults to `.` |
| `cpio extract` | `-o/--output` required | `-o/--output` defaults to `.`; new `-p/--path` |
| `cpio create` | `--crc` | `--format newc\|crc\|odc` (default `newc`); `--crc` still works and wins over `--format`; new `-v/--verbose`; `-o -` writes to standard output |
| `cpio ls`, `info`, `cat`, `extract` | archive path | `-` reads the archive from standard input |
| `iso create` | `--boot-info-table` without `--boot` accepted | `--boot-info-table` requires `--boot`; `--efi-boot` alone writes a UEFI-only catalog |
| `iso mkisofs` | `--isohybrid-mbr FILE` only enabled an MBR | uses FILE as MBR boot code |
| `udf ls -a` | "including hidden" | adds `.` and `..`, as `iso ls -a` does |
| `udf verify` | `-v` turned on the tree walk | always walks the tree and reads every file; `-v` prints each path |
| `udf create -r` | bad revision rejected at run time | rejected when parsing arguments |
| `hadris-cd create` to `udf bridge` | `--udf-revision`, `--no-joliet` | `-r/--revision`; Joliet is off unless `-J`; the `iso create` flags apply: `-l/--level`, `--system-id`, `--volume-set-id`, `--publisher-id`, `--preparer-id`, `--application-id`, `--strict-charset`, `-v`, `--dry-run`, `-f` |

`-V/--volume-name` names a new volume in every format, and `-v/--verbose`
prints more.

### Behaviour

- `extract` never replaces an existing file or symlink and merges existing
  directories. `extract -p PATH` merges the image root into `-o`; any other
  path lands at `<output>/<name>`, file or directory (V2 `iso` and `udf`
  merged a directory's contents into `-o`). cpio extraction now also
  refuses existing files, and an entry with `..` or leading through a
  symlink stops the extraction with an error instead of a warning.
- `verify` exits non-zero on findings. V2 `fat verify` always exited 0 and
  `udf verify` failed only when the root could not be opened.
- `hadris udf bridge` follows `hadris iso create`: ISO level 1, no Joliet
  unless `-J`, no ISO 9660:1999 tree. `hadris-cd create` wrote level 2, Joliet
  level 3 unless `--no-joliet`, and the enhanced tree. To get Joliet back,
  pass `-J`. Symlinks in the source are stored instead of refused.
- Paths inside an image may start with `/` or `./` in every format.
- Unreadable source entries are skipped with a warning, and `create` dates
  images with `SOURCE_DATE_EPOCH` when it is set.
- `hadris fat` handles exFAT in every command.
- `hadris iso ls`, `cat` and `extract` read Rock Ridge or Joliet names first
  and fall back to the primary tree, ignoring ASCII case.

## Symbol tables

Paths are written without the mode module where an item exists in both
`sync` and `r#async` (V2 had `hadris_x::sync::Foo`, `hadris_x::r#async::Foo`
and usually a root re-export `hadris_x::Foo`; the tables list one of them).
"Removed" means the item has no successor; the reason or the alternative
follows. In V3, name I/O items through their mode module:
`hadris_fat::sync::FatFs`, `hadris_iso::r#async::IsoFs`.

### hadris

| V2 path | V3 path or replacement |
|---|---|
| `hadris::archive` | `hadris::cpio` |
| `hadris::archive::cpio` | `hadris::cpio` |
| `hadris::block` | Removed. Formats are at flat paths; detection is `hadris::sync::detect` |
| `hadris::block::fat` | `hadris::fat` |
| `hadris::block::part` | `hadris::part` |
| `hadris::block::storage` | `hadris::storage` |
| `hadris::block::sync::OpenVolume`, `hadris::block::detect` | `hadris::sync::{open, detect}`, `hadris::sync::AnyFs` |
| `hadris::optical` | Removed |
| `hadris::optical::iso` | `hadris::iso` |
| `hadris::optical::udf` | `hadris::udf` |
| `hadris::optical::cd` | `hadris::udf::{plan_bridge, sync::write_bridge}` |
| `hadris::optical::sync::OpenOpticalImage` | `hadris::sync::open`, `hadris::sync::AnyFs` |
| `hadris::path` | Removed. Paths are byte strings; see `hadris::fs::Resolve` |
| `hadris::fixed` | Removed, no successor |
| (new) | `hadris::io`, `hadris::fs`, `hadris::storage` (always), `hadris::ntfs` (`unstable-ntfs`) |
| (new) | `hadris::{Error, ErrorKind, FsResult, Location, DetailCode, Errno, MountError, PathError}` |
| (new) | `hadris::{ImageFormat, Detection, Candidate}`, `hadris::{sync, r#async}::{detect, open, AnyFs}` |
| (new) | `hadris::host::{open, FileDevice, StdIo, read_tree, write_tree, file, source_date_epoch, mount_options, local_utc_offset, TreeOptions, Symlinks, OnError}` |

### hadris-io

| V2 path | V3 path or replacement |
|---|---|
| `hadris_io::{Read, Write, Seek}` (root glob of `sync`) | `hadris_io::sync::{Read, Write, Seek}` |
| `hadris_io::sync::{Read, Write, Seek}` | Same path. Each now has an `ErrorType` supertrait with the implementor's error; `read_exact` and `write_all` return `ExactError<E>` |
| `hadris_io::r#async::{Read, Write, Seek}` | `hadris_io::r#async::{Read, Write, Seek}` (futures `Send`) or `hadris_io::local::{Read, Write, Seek}` (not `Send`) |
| `hadris_io::{ReadSeek, ReadWrite, ReadWriteSeek}` and their mode twins | Removed. Write the bounds out: `R: Read + Seek` |
| `hadris_io::{ReadExt, Parsable, Writable}` and their mode twins | Removed. Read raw layouts from the format crates' `raw` modules |
| `hadris_io::Borrowed` and mode twins | Removed. `&mut T` implements the traits |
| `hadris_io::FromEmbedded`, `hadris_io::sync::FromEmbedded`, `hadris_io::r#async::FromEmbedded` | `hadris_io::FromEmbedded` (feature `embedded-io`); one type for both modes |
| `hadris_io::ToEmbedded`, `hadris_io::sync::ToEmbedded` | Removed. `StdIo` implements the `embedded-io` traits with the `embedded-io` feature |
| (blanket impls over `std::io` types) | `hadris_io::StdIo<T>`; `hadris_io::ToStd<T>` for the other direction |
| `hadris_io::Cursor` | `hadris_io::Cursor` |
| `hadris_io::SeekFrom` (re-export of `embedded_io::SeekFrom`) | `hadris_io::SeekFrom`, a Hadris `#[non_exhaustive]` enum with `resolve` |
| `hadris_io::Error` (`Context`, `Source`) | `hadris_io::Error<E>` |
| `hadris_io::ErrorKind` (`std::io`-style kinds) | `hadris_io::ErrorKind` (filesystem kinds); see [Errors](#errors) |
| `hadris_io::Result` | `hadris_io::FsResult<T, E>`, or `Result<T, Self::Error>` on the traits |
| `hadris_io::IoError` | Removed. The bound is `ErrorType::Error: core::error::Error + Send + Sync + 'static` |
| `hadris_io::{Path, PathBuf}` | Removed. Use `std::path` |
| `hadris_io::try_io_result_option!` | `hadris_io::try_io_result_option!` |
| (new) | `ErrorType`, `ExactError`, `StdIo`, `ToStd`, `into_std_error`, `Location`, `DetailCode`, `Errno`, `InvalidSeek`, `{sync, r#async, local}::{ByteSource, SeekSource, MaybeSend}` |

### hadris-storage

| V2 path | V3 path or replacement |
|---|---|
| `hadris_storage::BlockDevice`, `hadris_storage::sync::BlockDevice` | `hadris_storage::sync::BlockDevice`; `read_blocks` takes a `BlockIndex` and returns `hadris_io::Error<Self::Error>` |
| `hadris_storage::BlockDeviceMut`, `hadris_storage::sync::BlockDeviceMut` | Merged into `BlockDevice`: implement `writable`, `write_blocks` and `flush` |
| `hadris_storage::r#async::{BlockDevice, BlockDeviceMut}` | `hadris_storage::r#async::BlockDevice` (`Send`) or `hadris_storage::local::BlockDevice` |
| `hadris_storage::SeekBlockDevice` and mode twins | `hadris_storage::{sync, r#async}::StreamDevice<T>` |
| `hadris_storage::PartitionView` | `hadris_storage::Partition<D>` over a block device |
| `hadris_storage::Error` | `hadris_io::Error<D::Error>`; a refused write is kind `ReadOnly`, an out-of-range request `InvalidInput` |
| `hadris_storage::Result` | `hadris_io::FsResult<T, E>` |
| `hadris_storage::BlockIndex(pub u64)` | `hadris_storage::BlockIndex`, private field: `BlockIndex::new(n)`, `get()` |
| `hadris_storage::BlockCount(pub u64)` | `hadris_storage::BlockCount`: `new(n)`, `get()` |
| `hadris_storage::BlockRange` | `hadris_storage::BlockRange`, `const fn` accessors instead of public fields |
| `hadris_storage::BlockGeometry` | `hadris_storage::BlockGeometry`, accessors instead of public fields |
| `hadris_storage::BlockSize` | `hadris_storage::BlockSize` |
| (`impl BlockDevice for std::fs::File`, V3 previews only) | `hadris_storage::host::FileDevice` |
| (new) | `MemDevice`, `MemBuffer`, `ReadOnly`, `{sync, r#async, local}::{Cache, ByteView, StreamDevice, StreamWrite}`, `host::{FileDevice, file_len}`, `impl BlockDevice for Vec<u8>` |

### hadris-common

`hadris-common` is internal in V3; do not depend on it directly.

| V2 path | V3 path or replacement |
|---|---|
| `hadris_common::types::number::{U16, U24, U32, U64}` and the `U16Le`, `U32Be`, ... aliases | Kept; they gained inherent `new`, `get` and `set` |
| `hadris_common::types::number::align_up` | Kept |
| `hadris_common::types::endian::{Endian, Endianness, BigEndian, LittleEndian, NativeEndian}` | Kept. `Endianness::get()` is `Endianness::is_le()` |
| `hadris_common::types::endian::EndianType` | Removed |
| `hadris_common::types::endian::MaybePod` | Removed; the `Endian` associated types have no bounds and `bytemuck` only adds `Pod` impls |
| `hadris_common::types::extent::Extent` | `hadris_fs::Extent` |
| `hadris_common::types::extent::FileType` | `hadris_fs::FileType` |
| `hadris_common::types::extent::Timestamps` | `hadris_fs::Metadata` times, `hadris_fs::SetAttr` |
| `hadris_common::types::layout::{FileLayout, DirectoryLayout, AllocationMap}` | Removed; writers keep their layout private and report positions in `hadris_fs::Report` |
| `hadris_common::types::no_alloc::{ArrayVec, RingBuf, ArrayVecError}` | Removed |
| `hadris_common::alg::hash::crc::Crc32HasherIsoHdlc` | Removed. `hadris_part::raw::crc32` computes the same CRC-32 |
| `hadris_common::optical::{OpticalMediaType, SessionInfo, OpticalMetadataWriter, OPTICAL_SECTOR_SIZE}` | Removed, unused |
| `hadris_common::BOOT_SECTOR_BIN` | Removed |

### hadris-macros

Internal in both versions, with no user-facing items. Contributors: the
generator also produces the `Send` async mode that `r#async` uses in V3.

### hadris-fixed

Removed. None of its items has a public successor.

| V2 path | V3 path or replacement |
|---|---|
| `hadris_fixed::{FixedBytes, FixedStr, FixedUtf16, FixedUtf16Be, FixedUtf16Le}` | Removed |
| `hadris_fixed::{Utf16ByteOrder, BigEndian, LittleEndian}` | Removed |
| `hadris_fixed::CapacityError` | Removed |

### hadris-path

Removed. Paths are `/`-separated bytes passed as `impl AsRef<[u8]>`;
resolution is a value, `hadris_fs::Resolve::{Lexical, Follow, NoFollow}`.

| V2 path | V3 path or replacement |
|---|---|
| `hadris_path::VPath` | Removed. Pass `&str` or `&[u8]` paths to `Volume` methods or `FileSystem::resolve` |
| `hadris_path::{Component, Components, Separators, split_path}` | Removed |
| `hadris_path::PathError` | Removed. Resolution errors are `hadris_fs::Error` with kind `InvalidInput`, `NotFound` or `NotADirectory`. (`hadris_fs::PathError` is a different type: an error that carries a tree or host path.) |

### hadris-archive

| V2 path | V3 path or replacement |
|---|---|
| `hadris_archive::cpio` | `hadris_cpio`, or `hadris::cpio` |

### hadris-block

| V2 path | V3 path or replacement |
|---|---|
| `hadris_block::detect::sync::detect(&mut stream, block_size)` | `hadris::sync::detect(&mut dev)` returning `Detection` |
| `hadris_block::detect::r#async::detect` | `hadris::r#async::detect` |
| `hadris_block::detect::BlockFormat` | `hadris::ImageFormat` |
| `hadris_block::detect::FatVariant` | `hadris::ImageFormat::Fat(FatKind)` and `ImageFormat::ExFat` |
| `hadris_block::detect::PartitionTableKind` | `hadris::ImageFormat::{Mbr, Gpt}` |
| `hadris_block::detect::detect_sector` | Removed. `detect` reads the device itself |
| `hadris_block::sync::OpenVolume` | `hadris::sync::open(dev, MountOptions)` returning `hadris::sync::AnyFs` |
| `hadris_block::r#async::OpenVolume` | `hadris::r#async::open`, `hadris::r#async::AnyFs` |
| `OpenVolume::open(&mut stream, block_size)` | `hadris::sync::open(dev, options)`; `hadris::host::open(path)` for a host file |
| `OpenVolume::as_fat`, `OpenVolume::Fat` | `match fs { AnyFs::Fat(fat) => .., AnyFs::ExFat(exfat) => .., _ => .. }` |
| `hadris_block::partition::{mbr_partition_view, gpt_partition_view}` | `hadris_part::sync::open(dev, &partition)` returning `hadris_storage::Partition` |
| `hadris_block::Error`, `hadris_block::Result` | `hadris::MountError`, `hadris::Error`; unknown formats fail with `ErrorKind::NotRecognized` |
| `hadris_block::{fat, part, storage}` | `hadris::{fat, part, storage}` or the crates |

### hadris-fat

| V2 path | V3 path or replacement |
|---|---|
| `hadris_fat::FatVolume`, `hadris_fat::fs::FatVolume` (and `sync::`, `r#async::`) | `hadris_fat::sync::FatFs<D>` or `hadris_fat::r#async::FatFs<D>` (`alloc`); wrap in `hadris_fs::sync::Volume` for paths |
| `hadris_fat::FatVolumeBuilder`, `fs::FatVolumeBuilder` | `hadris_fs::MountOptions` passed to `FatFs::mount` |
| `FatVolumeBuilder::fat_cache`, `FatVolume::{fat_cache, with_cached_fat, with_fat_cache_locked}` | `hadris_storage::sync::Cache::new(dev, blocks)` around the device |
| `FatVolumeBuilder::time_provider`, `FatVolume::time_provider` | `MountOptions::with_clock`, `FatFs::clock()` |
| `FatVolumeBuilder::oem_converter`, `FatVolume::oem_converter` | `MountOptions::with_code_page`, `FatFs::code_page()` |
| `hadris_fat::fs::VolumeInfo` | `FatFs::info()` returning `hadris_fat::Geometry` (`kind`, `volume_serial`, `cluster_size`, ...) and `FileSystem::label` |
| `hadris_fat::fs::FsStatusFlags`, `FatVolume::read_status_flags` | `FatFs::was_dirty()` |
| `FatVolume::{read_root_label, set_root_label}` | `FileSystem::label(&mut buf)`, `FatFs::set_label(Option<VolumeLabel>)` |
| `FatVolume::{free_cluster_count, next_free_cluster_hint}` | `FileSystem::statfs()` (`FsStats`) |
| `hadris_fat::FatDir`, `dir::FatDir`, `dir::FatDirIter` | A directory `NodeId`; `FileSystem::readdir`, `Volume::read_dir` returning `ReadDir` |
| `hadris_fat::FileEntry`, `dir::FileEntry` | `NodeId` plus `hadris_fs::Metadata` (`FileSystem::stat`, `DirEntry::metadata`) |
| `hadris_fat::DirectoryEntry`, `dir::DirectoryEntry` | `hadris_fs::DirEntry` |
| `dir::{FileSystemErrors, FileSystemWarnings, ParseInfo}` | Removed. Use `hadris_fat::sync::check` |
| `hadris_fat::FatVolumeReadExt`, `read::FatVolumeReadExt`, `read::FileReader` | `Volume::open(path, OpenOptions::new().read())` returning `hadris_fs::sync::File`, or `FileSystem::{open, read, close}` |
| `hadris_fat::FatVolumeWriteExt`, `write::FatVolumeWriteExt`, `write::FileWriter` | `hadris_fs::sync::File` (`write`, `set_len`, `close`), `FileSystem::{write, truncate, setattr}` |
| `FatVolume::{create_file, create_dir, delete, rename, set_attributes, set_times, truncate}` | `FileSystem::{create, mkdir, unlink, rmdir, rename, setattr, truncate}`; `Volume::{open, create_dir, create_dir_all, remove_file, remove_dir, remove_dir_all, rename, set_attr}` |
| `FatVolume::{open_path, open_file_path, open_dir_path, open_dir_entry}` | `FileSystem::resolve(path, Resolve)`, `Volume::open`, `Volume::read_dir` |
| `FatVolume::{flush, sync}` | `FileSystem::sync`; `FatFs::unmount` syncs and returns the device |
| `FatVolume::fat_type`, `hadris_fat::FatType`, `fat_table::FatType` | `FatFs::info().kind()`, `hadris_fat::FatKind` |
| `hadris_fat::{Fat, Fat12, Fat16, Fat32}`, `fat_table::*`, `FatVolume::fat` | Removed from `hadris-fat`. FAT entry access is `hadris_fat_raw::io::{sync, r#async}::{get, set, next, walk}` with `hadris_fat_raw::io::Fat` |
| `hadris_fat::cache::{FatSectorCache, CachedFat, CacheStats}` | `hadris_storage::sync::Cache` |
| `hadris_fat::FatDateTime`, `time::FatDateTime` | `hadris_fs::DateTime`; encoding in `hadris_fat_raw::date::{encode, decode}` |
| `time::{TimeProvider, ChronoTimeProvider, EpochTimeProvider, StaticTimeProvider, DEFAULT_TIME_PROVIDER}` | `hadris_fs::{Clock, SystemClock, NoClock}` via `MountOptions::with_clock`; writers take `with_time(DateTime)` |
| `oem::{OemCpConverter, Cp437OemCpConverter, LossyAsciiOemCpConverter, DEFAULT_OEM_CONVERTER}` | `hadris_fs::{CodePage, Cp437, Ascii}`; the default is now `Cp437` |
| `file::{ShortFileName, CreateShortFileNameError}` | `hadris_fat_raw::short_name` (`generate`, `to_disk`, `from_disk`, `display`) |
| `file::{LongFileName, LfnBuilder}` | `hadris_fat_raw::lfn` (`Assembler`, `pack`, `unpack`) |
| `hadris_fat::format::FatVolumeFormatter::format` | `hadris_fat::sync::format(&mut dev, &FatOptions)` returning `Geometry`, then `FatFs::mount` |
| `FatVolumeFormatter::calculate_params`, `format::FormatParams` | `hadris_fat_raw::layout::plan`, `hadris_fat_raw::layout::{Request, Layout}` |
| `hadris_fat::format::FatFormatOptions` | `hadris_fat::FatOptions` (see [FAT and exFAT](#fat-and-exfat)) |
| `format::FatTypeSelection` | `FatOptions::with_kind(FatKind)`; omit it for automatic selection |
| `format::SectorSize` | `FatOptions::with_sector_size(u32)` |
| `format::MediaType` | `FatOptions::with_media(u8)` |
| `format::OemName` | `FatOptions::with_oem_name([u8; 8])` |
| `format::VolumeLabel` | `hadris_fat::VolumeLabel` (also `TryFrom<&str>`) |
| `hadris_fat::tool::verify::{FatVerifyExt, VerificationReport, VerificationIssue}`, `FatVolume::verify` | `hadris_fat::sync::check(&mut dev, &mut scratch, on_finding)` returning `hadris_fs::CheckReport`; findings are `hadris_fs::Finding` with a `hadris_fat::Detail` |
| `hadris_fat::tool::analysis::{FatAnalysisExt, FatStatistics, FileFragmentInfo, FragmentationReport, ClusterState}` | Removed. `FatFs::extents` maps a file's clusters, `FileSystem::statfs` gives free space; `hadris fat fragmentation` in the CLI |
| `FatVolume::{get_cluster_chain, scan_fat, statistics, fragmentation_report}` | `FatFs::extents(node, from, &mut [Extent])`, `FileSystem::statfs` |
| `hadris_fat::raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo, BpbExt32Flags}` | `hadris_fat_raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo, BpbExt32Flags}` |
| `hadris_fat::raw::RawFileEntry` | `hadris_fat_raw::RawDirEntry` (and `ShortEntry`) |
| `hadris_fat::raw::RawLfnEntry` | `hadris_fat_raw::RawLfnEntry` (and `LongEntry`) |
| `hadris_fat::raw::DirEntryAttrFlags` | `hadris_fs::Attributes` for the DOS bits; the raw constants are in `hadris_fat_raw` |
| `hadris_fat::raw::NtCaseFlags` | `hadris_fat_raw::short_name::case_bits` |
| `hadris_fat::io::{Cluster, SectorCursor, SectorLike, error_from_kind}` | Removed |
| `hadris_fat::{Read, Write, Seek, SeekFrom, ReadExt, Parsable, Writable, Error, ErrorKind, IoResult}` (io re-exports) | `hadris_io::sync::{Read, Write, Seek}`, `hadris_io::SeekFrom`; the rest removed |
| `hadris_fat::Error`, `hadris_fat::error::Error`, `hadris_fat::Result` | `hadris_fs::Error<E>`, `FsResult`, `MountError`; causes in `hadris_fat::Detail` |
| `hadris_fat::sync::*` root glob re-export | Removed. Always write the mode: `hadris_fat::sync::FatFs` |
| exFAT preview `hadris_fat::exfat::ExFatVolume` | `hadris_fat::exfat::sync::ExFatFs<D>` (and `r#async`) |
| `exfat::ExFatInfo` | `ExFatFs::info()` returning `hadris_fat::exfat::Geometry` |
| `exfat::ExFatBootSector` | `hadris_fat_raw::exfat::BootSector` |
| `exfat::{ExFatDir, ExFatDirIter, ExFatFileEntry}` | `FileSystem::readdir`, `hadris_fs::DirEntry`, `Metadata` |
| `exfat::{ExFatFileReader, ExFatFileWriter}` | `hadris_fs::sync::File` or `FileSystem::{read, write}` |
| `exfat::{ExFatFormatOptions, format_exfat}` | `hadris_fat::exfat::ExFatOptions`, `hadris_fat::exfat::sync::format` |
| `exfat::ExFatLayoutParams` | Removed. `format` returns the `exfat::Geometry` |
| `exfat::{AllocationBitmap, ExFatTable}` | `hadris_fat_raw::exfat::io` (`bit`, `set_bit`, `get`, `next`, `set`) |
| `exfat::UpcaseTable` | `hadris_fat_raw::exfat::UpcaseDecoder`, `hadris_fat_raw::exfat::io::Upcase` |
| `exfat::ExFatTimestamp` | `hadris_fs::DateTime`; `hadris_fat_raw::exfat::{decode_time, encode_time}` |
| `exfat::{RawFileDirectoryEntry, RawStreamExtensionEntry, RawFileNameEntry}` | `hadris_fat_raw::exfat::{FileEntry, StreamEntry, NameEntry}` |
| `exfat::FileAttributes` | `hadris_fs::Attributes` |
| `exfat::Error`, `exfat::Result` | `hadris_fs::Error<E>`, `hadris_fat::exfat::Detail` |
| (new) | `hadris_fat::{sync, r#async}::{format, write, check}`, `FatOptions`, `Geometry`, `Detail`, `exfat::{ExFatOptions, VolumeLabel, Geometry, Detail}`, `embedded` and `exfat::embedded` |

### hadris-part

| V2 path | V3 path or replacement |
|---|---|
| `hadris_part::PartitionTable` (read with `PartitionTableReadExt::read_from`) | `hadris_part::sync::read(&mut dev)` returning `hadris_part::Disk`; `Disk::table()` is a `hadris_part::PartitionTable` (`Mbr`, `Gpt`, `Hybrid`, non-exhaustive) |
| `hadris_part::{PartitionTableReadExt, PartitionTableWriteExt}`, `scheme_io::*` | `hadris_part::sync::{read, write, create}` |
| `hadris_part::{MasterBootRecordReadExt, MasterBootRecordWriteExt}`, `mbr_io::*` | `hadris_part::sync::{read, write}` |
| `hadris_part::{GptHeaderReadExt, GptHeaderWriteExt, GptDiskReadExt, GptDiskWriteExt}`, `gpt_io::*` | `hadris_part::sync::{read, write}` |
| `hadris_part::PartitionTableRead` | Removed |
| `hadris_part::partition_table::{detect, open}`, `sync::partition_table::*` | `hadris_part::sync::read`, `hadris_part::sync::open(dev, &partition)` returning `hadris_storage::Partition` |
| `hadris_part::{PartitionInfo, PartitionInfoTrait}`, `scheme::PartitionInfo` | `hadris_part::Partition` (`index`, `start`, `len`, `end`, `size_bytes`, `kind`, `flags`, `attributes`, `unique_guid`, `name`) |
| `hadris_part::PartitionType` | `hadris_part::PartitionKind::{Mbr(MbrType), Gpt(Guid)}` |
| `hadris_part::PartitionSchemeType` | `hadris_part::TableKind` |
| `hadris_part::scheme::detect_scheme_from_mbr`, `hybrid::is_hybrid_mbr` | Match on `hadris_part::sync::read(&mut dev)?.table()` |
| `hadris_part::MasterBootRecord`, `mbr::MasterBootRecord` | `hadris_part::Mbr` (table), `hadris_part::raw::RawMbr` (layout) |
| `hadris_part::MbrPartition`, `mbr::MbrPartition` | `hadris_part::MbrEntry`, `hadris_part::raw::RawMbrEntry` |
| `hadris_part::MbrPartitionTable`, `mbr::MbrPartitionTable` | `hadris_part::Mbr` |
| `hadris_part::MbrPartitionType`, `mbr::MbrPartitionTypeFull` | `hadris_part::MbrType` with constants (`MbrType::FAT32_LBA`, `MbrType::LINUX`, ...) |
| `hadris_part::Chs`, `mbr::Chs` | `hadris_part::raw::Chs` |
| `hadris_part::GptDisk`, `scheme::GptDisk` | `hadris_part::Gpt` |
| `hadris_part::GptHeader`, `gpt::GptHeader` | `hadris_part::raw::RawGptHeader` |
| `hadris_part::GptPartitionEntry`, `gpt::GptPartitionEntry` | `hadris_part::GptEntry`, `hadris_part::raw::RawGptEntry` |
| `gpt::GptPartitionName` | `hadris_part::PartitionName` |
| `gpt::GptAttributes` | `u64` attributes (`Gpt::set_attributes`, `GptEntry::with_attributes`), `PartitionFlags`, `raw::GPT_ATTR_*` |
| `hadris_part::Guid`, `gpt::Guid` | `hadris_part::Guid` |
| `Guid::from_str` (inherent) | `Guid::parse_const`, or `str::parse` through `FromStr` |
| `Guid::UNUSED` | `Guid::NIL` |
| `Guid::EFI_SYSTEM` and the other type constants | `hadris_part::gpt::types::EFI_SYSTEM`, ... |
| `gpt::calculate_partition_array_crc32` | Automatic on write; `hadris_part::raw::{crc32, gpt_header_crc}` |
| `hybrid::{HybridMbrBuilder, HybridMbrConfig, MirroredPartition}` | `hadris_part::HybridMbr` (`with_protective_slot`, `add_mirrored`), `hadris_part::Hybrid`, `PartitionSpec::with_mirror` |
| `hadris_part::DiskGeometry`, `geometry::DiskGeometry` | Removed. `DiskLayout::with_alignment(Alignment)` aligns new partitions |
| `hadris_part::validate_partition_alignment`, `geometry::validate_all_partitions_aligned` | Removed |
| `hadris_part::Error`, `error::Error`, `hadris_part::Result` | `hadris_fs::Error<E>` with `hadris_part::Detail`; `hadris_part::TableError` for edits |
| `hadris_part::{Le, Read, Write, Seek, SeekFrom, ReadExt, Parsable, Writable, Error, ErrorKind, IoResult}` re-exports | Removed; use `hadris_io` directly |
| (new) | `Disk`, `DiskLayout`, `PartitionSpec`, `Size`, `Alignment`, `PartitionFlags`, `Partitions`, `Runs`, `Run`, `GptCopy`, `GuidParseError`, `{sync, r#async}::{read, write, create, open, scan}`, `raw` |

### hadris-ntfs

NTFS stays a preview; its native API may change in 3.x minors.

| V2 path | V3 path or replacement |
|---|---|
| `hadris_ntfs::NtfsFs`, `hadris_ntfs::sync::NtfsFs` (over a stream, `NtfsFs::open`) | `hadris_ntfs::sync::NtfsFs<D>` (`mount(dev, MountOptions)`, `unmount`), implementing `FileSystem` |
| `NtfsFs::root_dir`, `hadris_ntfs::sync::NtfsDir` | `FileSystem::{root, readdir}`, `Volume::read_dir` |
| `hadris_ntfs::sync::NtfsEntry` | `hadris_fs::DirEntry` and `Metadata` |
| `NtfsFs::open_path` | `FileSystem::resolve`, `Volume::open`, `Volume::metadata` |
| `hadris_ntfs::sync::{FileReader, NtfsFsReadExt}` | `Volume::open` returning `File`, `FileSystem::read`; named streams with `NtfsFs::{streams, read_stream_at}` |
| `NtfsFs::read_mft_record` | Removed |
| `NtfsFs::{volume_serial, total_sectors, cluster_size, mft_record_size}` | Same names on `NtfsFs`, plus `sector_size` and `index_record_size`; `cluster_size` returns `u64` |
| `hadris_ntfs::{NtfsError, Result}` | `hadris_fs::Error<E>` with `hadris_ntfs::Detail`; mount fails with `MountError` |
| `hadris_ntfs::attr::*` (`AttrIter`, `NtfsAttr`, `AttrBody`, `DataRun`, `DataRunDecoder`, `FileNameInfo`, `IndexEntryInfo`, `parse_file_name`, `parse_index_entries`, `apply_fixups`, `decode_data_runs`, `decode_record_size`, `decode_utf16le`, `is_i30_name`) | Removed; the parsers are private. Attribute, flag and name-space codes are constants in `hadris_ntfs::raw` |
| `hadris_ntfs::RawNtfsBootSector` | `hadris_ntfs::raw::BootSector` |
| Root glob re-exports of `sync` and `raw` | Removed |

### hadris-optical

| V2 path | V3 path or replacement |
|---|---|
| `hadris_optical::sync::OpenOpticalImage`, `r#async::OpenOpticalImage` | `hadris::sync::open`, `hadris::r#async::open` returning `AnyFs` |
| `OpenOpticalImage::{as_iso9660, as_iso9660_mut}` | `match fs { AnyFs::Iso(iso) => .., _ => .. }` |
| `OpenOpticalImage::as_udf` | `AnyFs::Udf(udf)` |
| `hadris_optical::OpenPolicy` | Removed. A bridge opens as UDF; mount `IsoFs` or `UdfFs` directly to choose, `IsoFs::mount_namespace` for an ISO tree |
| `hadris_optical::OpticalFormat` | `hadris::ImageFormat::{Iso, Udf, IsoUdfBridge}` |
| `hadris_optical::detect::sync::detect`, `detect::r#async::detect` | `hadris::sync::detect`, `hadris::r#async::detect` |
| `hadris_optical::detect::OpticalFormats` | `hadris::Detection` |
| `hadris_optical::detect::UdfVrs` | Removed; `hadris_udf::VolumeInfo::revision()` after mounting, or `hadris_udf::raw::vsd::{NSR02, NSR03}` for a raw probe |
| `hadris_optical::{Error, Result}` | `hadris::MountError`, `hadris::Error` |
| `hadris_optical::{iso, udf, cd}` | `hadris::iso`, `hadris::udf`; `cd` is `hadris::udf::{plan_bridge, sync::write_bridge}` |

### hadris-cd

| V2 path | V3 path or replacement |
|---|---|
| `hadris_cd::OpticalImageWriter`, `sync::OpticalImageWriter`, `writer::OpticalImageWriter` | `hadris_udf::sync::write_bridge(dev, &tree, &IsoOptions, &UdfOptions)`; `hadris_udf::plan_bridge` without I/O |
| `OpticalImageWriter::{new, finish, create}` | `write_bridge` |
| `hadris_cd::OpticalImageOptions`, `options::OpticalImageOptions` | A `hadris_iso::IsoOptions` and a `hadris_udf::UdfOptions` |
| `OpticalImageOptions::volume_id` | `IsoOptions::with_id(IsoId::Volume, ..)`, `UdfOptions::with_id(UdfId::Volume, ..)` |
| `OpticalImageOptions::joliet(level)`, `hadris_cd::JolietLevel` | `IsoOptions::with_joliet()` |
| `iso_only`, `udf_only` | `hadris_iso::sync::write` or `hadris_udf::sync::write` |
| `sector_size` | Removed; the writer uses the device's block size |
| `hadris_cd::IsoOptions`, `options::IsoOptions` | `hadris_iso::IsoOptions` |
| `hadris_cd::UdfOptions`, `options::UdfOptions` | `hadris_udf::UdfOptions` |
| `hadris_cd::FileTree`, `tree::FileTree` | `hadris_fs::Tree` |
| `hadris_cd::Directory` | `hadris_fs::Node::dir()` |
| `hadris_cd::FileEntry` (`from_buffer`, ...) | `hadris_fs::Node::file(Content)` |
| `hadris_cd::FileData` | `hadris_fs::Content` |
| `hadris_cd::FileExtent` | `hadris_fs::Extent`, `Report::extents(path)` |
| `hadris_cd::{LayoutManager, LayoutInfo}`, `layout::*` | Removed; layout comes from the ISO `Report` |
| `hadris_cd::{Error, Result}`, `error::Error` | `hadris_fs::PathError`, with `hadris_iso::Detail` or `hadris_udf::Detail` codes |
| `hadris_cd::{Borrowed, Read, Seek, SeekFrom, Write}` re-exports | Removed; use `hadris_io` |

### hadris-iso

Reading:

| V2 path | V3 path or replacement |
|---|---|
| `hadris_iso::IsoImage`, `read::IsoImage`, `sync::IsoImage` | `hadris_iso::sync::IsoFs<D>` (`mount`, `mount_namespace`, `unmount`) |
| `read::IsoReader` (allocation-free reader) | `hadris_iso::sync::IsoFs`, which needs no allocator |
| `read::IsoNamespace` | `hadris_iso::Namespace`; `IsoFs::namespaces()` lists them |
| `read::{IsoRoot, RootDir, RootDirs}`, `IsoImage::{root_dir, root_dirs}`, `IsoReader::{root, roots, primary_root, joliet_root, enhanced_root, preferred_root}` | `FileSystem::root()` of an `IsoFs` mounted on the chosen `Namespace` |
| `read::{IsoDir, IsoDirIter, IsoDirReader, RawDirIter}`, `directory::DirectoryRef` | `FileSystem::readdir`, `Volume::read_dir` |
| `read::{DirEntry, IsoDirEntry}` | `hadris_fs::DirEntry` |
| `IsoImage::find_path`, `IsoReader::{find_path, find_path_in}`, `IsoDir::find` | `FileSystem::resolve`, `FileSystem::lookup`, `Volume::metadata` |
| `read::{IsoFileReader, FileChunkIterator}`, `IsoImage::{read_file, read_file_chunked}`, `IsoReader::open_file` | `Volume::open` returning `File`, or `FileSystem::read(node, offset, buf)` |
| `IsoImage::read_bytes_at` | `IsoFs::read_raw(offset, buf)` |
| `read::IsoImageInfo`, `IsoImage::read_pvd` | `IsoFs::info()` returning `hadris_iso::VolumeInfo` (`id(IsoId)`, `date(IsoDate)`, `block_size`, `volume_space_size`) |
| `IsoImage::read_volume_descriptors`, `read::VolumeDescriptorIter` | `IsoFs::descriptor(index)` |
| `IsoImage::{path_table, path_table_entries}`, `path::{PathTableEntry, PathTableEntryHeader, PathTableEntryIter, PathTableInfo, PathTableRef}` | Removed; `hadris_iso::raw::PathTableHeader` is the layout |
| `IsoImage::{has_evd, supports_rrip}` | `IsoFs::namespaces()` |
| `read::{BootInfo, BootEntryInfo, BootSectionIter}` | `IsoFs::boot_catalog(&mut buf)` returning `hadris_iso::BootCatalog`, `CatalogEntries`, `CatalogEntry`; `IsoFs::boot_image(&entry)` |
| `boot::{BootCatalog, BaseBootCatalog}` | `hadris_iso::BootCatalog` (borrows the caller's buffer) |
| `boot::BootCatalogEntry` | `hadris_iso::CatalogEntry` |
| `boot::EmulationType` | `hadris_iso::Emulation` |
| `boot::PlatformId` | `hadris_iso::Platform` |
| `boot::BootError` | `hadris_fs::Error` with `Detail::BootCatalog`, `BootImage` or `BootInfoTable` |
| `read::{RripMetadata, RripTimestamps, RripDateTime}` | `IsoFs::rock_ridge(node)` returning `hadris_iso::RockRidgeInfo`; `Metadata` times |
| `read::Extent` | `hadris_fs::Extent`; `IsoFs::extents`, `IsoFs::records` |
| `read::{IsoName, FilenameType, NameError}` | Removed |
| `read::PathSeparator` | Removed; paths are `/`-separated |
| `read::collect_su_entries`, `susp::{SystemUseIter, SystemUseField}` | `hadris_iso::raw::{SuspEntries, SuspEntry}` |
| `susp::ContinuationArea` | `hadris_iso::raw::ContinuationArea` |
| `susp::{ExtensionReference, PaddingField, SplitSu, SuspIdentifier, SuspTerminator, SystemUseHeader, SystemUseBuilder}` | Removed |
| `hadris_iso::IsoCursor`, `io::IsoCursor`, `io::LogicalSector` | Removed |
| `hadris_iso::io` and the root re-exports (`Read`, `Seek`, `SeekFrom`, `Error`, `ErrorKind`, `IoResult`, `Parsable`, `Writable`, `ReadExt`, `try_io_result_option`) | Removed; use `hadris_io` |

On-disk layouts, now in `hadris_iso::raw`:

| V2 path | V3 path or replacement |
|---|---|
| `volume::{PrimaryVolumeDescriptor, SupplementaryVolumeDescriptor, BootRecordVolumeDescriptor, VolumeDescriptorHeader, VolumeDescriptorSetTerminator, VolumeDescriptor}` | `hadris_iso::raw::` same names |
| `volume::VolumeDescriptorType` | `hadris_iso::raw::DescriptorType` |
| `volume::{UnknownVolumeDescriptor, VolumeDescriptorList, VolumeError}` | Removed; `IsoFs::descriptor(index)`, errors with `hadris_iso::Detail` |
| `directory::{DirectoryRecord, DirectoryRecordHeader, DirDateTime, FileFlags}` | `hadris_iso::raw::` same names |
| `directory::RootDirectoryEntry` | `hadris_iso::raw::RootDirectoryRecord` |
| `directory::NotADirectoryError` | `ErrorKind::NotADirectory` |
| `boot::{BootValidationEntry, BootSectionEntry, BootSectionEntryExtension, BootInfoTable, Grub2BootInfoTable}` | `hadris_iso::raw::` same names |
| `boot::BootSectionHeaderEntry` | `hadris_iso::raw::BootCatalogHeader` |
| `boot::ElToritoWriter` | Removed; internal to the writer |
| `rrip::{PxEntry, PnEntry, NmFlags, SlComponentFlags, TfFlags}` | `hadris_iso::raw::` same names |
| `rrip::{ClEntry, NmEntry, PlEntry, ReEntry, SlEntry, SlComponent, TfEntry, RockRidgeEntry}` | Removed; `IsoFs::rock_ridge(node)` decodes them |
| `rrip::PosixFileMode` | `hadris_fs::Permissions` |
| `rrip::{RripBuilder, RripOptions}` | `IsoOptions::{with_rock_ridge, with_preserve, with_relocation}` |
| `types::IsoStr` | `hadris_iso::raw::IsoStr` (`as_str` returns `Result`) |
| `types::DecDateTime` | `hadris_iso::raw::DecDateTime` |
| `types::{LsbMsb, U16LsbMsb, U32LsbMsb}` | `hadris_iso::raw::{U16Both, U32Both}` |
| `types::{IsoString, IsoStringA, IsoStringD, IsoStrA, IsoStrD, CharsetA, CharsetD, CharsetD1, Charset, StdNum, IsoStrError}` | Removed |
| `joliet::ESCAPE_SEQUNCES` | `hadris_iso::raw::JOLIET_ESCAPES` |
| `joliet::JolietLevel` | `hadris_iso::JolietLevel` (reading); the writer uses `IsoOptions::with_joliet()` |
| `joliet::{decode_joliet_name, encode_joliet_name, is_likely_joliet_name}` | Removed |
| `file::{ConvertedName, EntryType, convert_l1, convert_l2, convert_l3, convert_joliet3, FilenameL1, FilenameL2, FilenameL3}` | Removed; `IsoOptions::{with_level, with_name_case}` choose the rules |

Writing:

| V2 path | V3 path or replacement |
|---|---|
| `write::IsoImageWriter::create` | `hadris_iso::sync::write(dev, &tree, &IsoOptions)`; `hadris_iso::plan` without I/O |
| `IsoImageWriter::create_with_allocation_floor` | `IsoOptions::with_min_blocks` |
| `write::{InputTree, InputFiles, InputEntry, File}` | `hadris_fs::Tree`, `hadris_fs::Node` |
| `write::InputEntryKind`, `write::FileSource` (`unstable-streaming`) | `hadris_fs::Content` (`bytes`, `empty`), `hadris_fs::host::file(path)` |
| `write::InputMetadata` | `hadris_fs::SetAttr` with `Node::with_attrs` |
| `InputTree::from_fs` | `hadris_fs::host::read_tree(dir, &TreeOptions)` |
| `write::{FileConversionError, IsoCreationError}`, `write::Error`, `write::Result` | `hadris_fs::PathError` with `hadris_iso::Detail` |
| `write::options::IsoFormatOptions` | `hadris_iso::IsoOptions` |
| `write::options::CreationFeatures` | `IsoOptions::{with_level, with_name_case, with_joliet, with_rock_ridge, with_iso1999, with_el_torito, with_hybrid}` |
| `write::options::BaseIsoLevel` | `hadris_iso::IsoLevel` and `hadris_iso::NameCase` |
| `write::options::HybridBootOptions` | `hadris_iso::Hybrid` (`mbr`, `gpt`, `gpt_hybrid_mbr`, `with_bootstrap`, `with_appended`, `with_flags`) |
| `write::options::PartitionScheme` | `Hybrid::mbr()`, `Hybrid::gpt()`, `Hybrid::gpt_hybrid_mbr()` |
| `HybridBootOptions::with_efi_boot_partition` | `Hybrid::with_appended(AppendedPartition::esp(content))` with `BootEntry::uefi_appended(0)` |
| `boot::options::BootOptions` | `hadris_iso::ElTorito` |
| `boot::options::{BootEntryOptions, BootSectionOptions}` | `hadris_iso::BootEntry` (`bios`, `uefi`, `uefi_appended`, `with_load_size`, `with_emulation`, `with_platform`, `with_load_segment`, `with_boot_info(BootInfo)`) |
| `write::estimator::{estimate, estimate_tree, IsoSizeEstimate, SizeBreakdown}` | `hadris_iso::plan(&tree, &options)?.size()` |
| `write::writer::{DirectoryId, WrittenDirectory, WrittenFile, WrittenFiles}` | `hadris_fs::Report` (`extents(path)`, `files()`) |
| `modify::IsoModifier` | `hadris_iso::sync::Session` (`open`, `tree_mut`, `write`, `export`, `into_inner`) |
| `modify::ModifyOp`, `modify::FileData` | `Tree` edits on `Session::tree_mut()` (`insert`, `replace`, `remove`), `Content` |
| `modify::{IsoModifyError, Error, Result}` | `hadris_fs::PathError` |
| (new) | `IsoId`, `IsoDate`, `Relocation`, `Preserve`, `AppendedPartition`, `SessionMode`, `Detail` |

### hadris-udf

| V2 path | V3 path or replacement |
|---|---|
| `hadris_udf::UdfVolume`, `fs::UdfVolume`, `sync::UdfVolume` | `hadris_udf::sync::UdfFs<D>` (`mount`, `unmount`), read-only |
| `UdfVolume::{root_dir, read_directory}`, `hadris_udf::UdfDir`, `dir::UdfDir` | `FileSystem::{root, readdir}`, `Volume::read_dir` |
| `dir::UdfDirEntry` | `hadris_fs::DirEntry` |
| `UdfVolume::read_file` | `Volume::open` returning `File`, or `FileSystem::read` |
| `hadris_udf::UdfVolumeInfo`, `fs::UdfVolumeInfo`, `UdfVolume::info` | `UdfFs::info()` returning `hadris_udf::VolumeInfo` (`id(UdfId)`, `revision`, `block_size`, `partitions`, `volume_serial`, ...) |
| `UdfVolumeInfo::{partition_start, partition_length}` | `VolumeInfo::partitions()` yielding `hadris_udf::PartitionInfo` |
| `dir::decode_filename` | Removed; `readdir` decodes names |
| `hadris_udf::UdfRevision` | `hadris_udf::UdfRevision` |
| `hadris_udf::UdfTimestamp` | `hadris_udf::raw::Timestamp`; `hadris_fs::DateTime` in `Metadata` |
| `hadris_udf::FileType`, `file::FileType` | `hadris_fs::FileType`; ICB codes in `hadris_udf::raw::file_type` |
| `file::{FileEntry, ExtendedFileEntry, IcbTag}` | `hadris_udf::raw::{FileEntry, ExtendedFileEntry, IcbTag}` |
| `file::AllocationType` | `hadris_udf::raw::allocation::{SHORT, LONG, EXTENDED, EMBEDDED}` |
| `dir::{FileIdentifierDescriptor, FileCharacteristics}` | `hadris_udf::raw::{FileIdentifierDescriptor, FileCharacteristics}` |
| `descriptor::{AnchorVolumeDescriptorPointer, CharSpec, FileSetDescriptor, LbAddr, LogicalVolumeDescriptor, PartitionDescriptor, PrimaryVolumeDescriptor, Type1PartitionMap}` | `hadris_udf::raw::` same names |
| `descriptor::DescriptorTag` | `hadris_udf::raw::Tag` |
| `descriptor::EntityIdentifier` | `hadris_udf::raw::EntityId`; decoded as `hadris_udf::EntityId` in `VolumeInfo` |
| `descriptor::ExtentDescriptor` | `hadris_udf::raw::ExtentAd` |
| `descriptor::{LongAllocationDescriptor, ShortAllocationDescriptor}` | `hadris_udf::raw::{LongAd, ShortAd}` |
| `descriptor::TagIdentifier` | `hadris_udf::raw::tag::*` constants |
| `descriptor::ExtentType` | `hadris_udf::raw::extent::*` constants |
| `descriptor::VrsType`, `descriptor::vrs` | `hadris_udf::raw::vsd::*` constants, `hadris_udf::raw::VolumeStructureDescriptor` |
| `descriptor::parse_vrs` | Removed; `UdfFs::mount` checks the recognition sequence, `hadris::sync::detect` finds it |
| `descriptor::PartitionContents` | Removed; `hadris_udf::PartitionKind` in `PartitionInfo` |
| `write::UdfWriter` (`new`, `create`, `write_vrs`, `write_avdp`, `write_pvd`, `write_lvid`, `write_fids`, ...) | `hadris_udf::sync::write(dev, &tree, &UdfOptions)`; the descriptor writers are internal |
| `write::UdfWriteOptions` | `hadris_udf::UdfOptions` (`with_id(UdfId, ..)`, `with_revision`, `with_time`, `with_seed`, `with_min_blocks`) |
| `UdfWriteOptions::{partition_start, partition_length}` | Removed; `with_min_blocks` reserves space |
| `write::{SimpleDir, SimpleFile}` | `hadris_fs::Tree`, `Node::dir`, `Node::file` |
| `write::UdfCreateOutput` | `hadris_fs::Report` (`size()`) |
| `write::{UdfFileExtent, UdfFileInfo, UdfDirInfo}` | `Report::extents(path)`, `Report::files()` |
| `hadris_udf::{Error, Result}` | `hadris_fs::Error<E>` or `PathError`, with `hadris_udf::Detail` |
| io re-exports at the root | Removed; use `hadris_io` |
| (new) | `plan`, `plan_bridge`, `{sync, r#async}::{write, write_bridge}`, `UdfId`, `VolumeInfo`, `EntityId`, `PartitionInfo`, `PartitionKind`, `Detail`, `raw` |

### hadris-cpio

| V2 path | V3 path or replacement |
|---|---|
| `hadris_cpio::CpioArchiveReader`, `read::CpioArchiveReader` | `hadris_cpio::sync::CpioReader<R, B = [u8; PATH_MAX]>` (`new`, `with_options`, `with_buffer`, `next_entry`, `next_segment`) |
| `next_entry_alloc`, `next_entry_with_buf` | `CpioReader::next_entry()`, returning an `Entry` that borrows the reader |
| `read_entry_data`, `read_entry_data_alloc` | `Entry` implements `hadris_io::sync::Read` |
| `skip_entry_data`, `skip_entry_data_owned` | Drop the `Entry`; the next `next_entry` skips unread data |
| `seek_to_entry` | Removed |
| `hadris_cpio::{CpioEntry, CpioEntryOwned}` | `hadris_cpio::sync::Entry` (`path`, `path_str`, `offset`, `data_offset`, `len`, `mode`, `file_type`, `metadata`, ...) |
| `hadris_cpio::CpioEntryHeader`, `entry::CpioEntryHeader` | `Entry` accessors; the header layouts are `hadris_cpio::raw::{NewcFields, OdcFields, BinaryFields}` |
| `hadris_cpio::CpioMagic`, `header::CpioMagic` | `hadris_cpio::Format::{Newc, Crc, Odc, Binary}`; `raw::{NEWC_MAGIC, NEWC_CRC_MAGIC, ODC_MAGIC, BINARY_MAGIC}` |
| `hadris_cpio::RawNewcHeader`, `header::RawNewcHeader` (14-argument `build`) | `hadris_cpio::raw::NewcHeader` built from `raw::NewcFields` |
| `hadris_cpio::FileType`, `mode::FileType` | `hadris_fs::FileType` (`Entry::file_type`) |
| `mode::make_mode` | `hadris_cpio::raw::S_IF*` constants |
| `hadris_cpio::CpioArchiveWriter`, `write::CpioArchiveWriter` | `hadris_cpio::sync::Writer<W>` (`new`, `append`, `append_hard_links`, `append_file`, `finish`) or `hadris_cpio::sync::write(out, &tree, &CpioOptions)` |
| `CpioArchiveWriter::finish(&tree)` | `hadris_cpio::sync::write`, which returns a `Report` |
| `hadris_cpio::CpioWriteOptions`, `write::CpioWriteOptions` (`crc(bool)`) | `hadris_cpio::CpioOptions` (`with_format(Format)`, `with_time`) |
| `hadris_cpio::{FileTree, FileNode}`, `write::file_tree::*` | `hadris_fs::{Tree, Node}` |
| `FileTree::from_fs` | `hadris_fs::host::read_tree` |
| `write::from_fs::FromFsError` | `hadris_fs::PathError` |
| `hadris_cpio::{Error, Result}`, `error::*` | `hadris_fs::Error<E>` with `hadris_cpio::Detail` |
| io re-exports at the root | Removed; use `hadris_io` |
| (new) | `plan`, `{sync, r#async}::{read_tree, write, EntryWriter}`, `ReaderOptions`, `raw` |

## Checklist

1. Replace `hadris-block`, `hadris-optical`, `hadris-cd`, `hadris-archive`,
   `hadris-path` and `hadris-fixed` dependencies (see [Crate map](#crate-map)).
2. Add `hadris-fs` and `hadris-storage` where you name their items.
3. Remove the `read`, `lfn`, `cache`, `tool`, `unstable-exfat`,
   `dirty-file-panic`, `crc`, `rand`, `joliet`, `unstable-streaming`,
   `block`, `optical`, `cd`, `archive`, `path`, `fixed` and `storage` features. Add
   `alloc` where you use `FatFs`, `ExFatFs`, `Tree` or a writer.
4. Name I/O items through their mode module (`hadris_fat::sync::FatFs`,
   `hadris_io::sync::Read`); nothing is re-exported at a crate root any more.
5. Wrap inputs in a block device: `host::FileDevice`, `MemDevice`,
   `Vec<u8>`, or `StreamDevice` over a stream. Wrap `std::io` streams for cpio
   in `StdIo`.
6. Mount with `mount(dev, MountOptions)`; wrap the driver in `Volume` for
   paths and handles; `unmount` to get the device back.
7. Replace per-crate error types with `Error<E>`, `FsResult`, `MountError`
   and `PathError`, and match causes with `ErrorKind` and the crate's
   `Detail::of`. Handle the new `NotRecognized` kind where you handled
   "not this format".
8. Build writer input as a `hadris_fs::Tree` (or `host::read_tree`) and call
   `plan` or `write`; read results from `Report`.
9. Replace `detect` and `OpenVolume`/`OpenOpticalImage` with
   `hadris::sync::detect`, `hadris::sync::open` and `AnyFs`.
10. Port partition code to `hadris_part::sync::{read, write, create, open, scan}`
    and pass GUIDs explicitly.
11. For firmware without an allocator, move to `hadris_fat::embedded`.
12. Check behaviour that changed on purpose: CP437 short names, the FAT UTC
    offset, fixed writer times (1980-01-01 unless `with_time`), and
    `close` not flushing the device.
13. Replace CLI invocations with `hadris <format> <command>` (see
    [Command-line tools](#command-line-tools)) and install
    `cargo install hadris-cli`.
