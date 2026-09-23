# Hadris V3 API design

Status: draft for discussion. Branch: `design/v3-api`.

V3 is the release where the public shapes stop moving. V2 kept breaking semver
inside minor releases (#83, #93, #94) or hid new work behind `unstable-*` flags
that change the shape of public types (#111, #115). V3 fixes the extension
points first. After 3.0.0, new features land in 3.x minors because the types,
traits and error kinds they need already exist.

That gives a rule for scope. Every feature listed in [section 5](#5-per-crate-changes)
is V3 work. Every one of them must have its public shape in 3.0.0. The
implementation of a feature can follow in 3.x only if it fits a shape that
already shipped. NTFS write is the obvious example: `FileSystemMut` exists in
3.0, NTFS reports itself read-only through `Capabilities`, and write support
arrives later without a break.

V3 serves three kinds of users, and none of them is optional:

- OS kernels and VFS layers (#18, the stlankes PRs #83 #84 #89) need `Send`
  volumes, stable node IDs, positional I/O, resumable directory reads, and a
  cache they can turn off.
- Bootloaders and firmware need no-alloc reads, small code, and async on
  embedded executors.
- Image-mastering tools need tree builders with bounded memory, boot options,
  reproducible output, and a one-flow disk image (GPT plus ESP plus FAT).

Contents:

1. [Evidence](#1-evidence)
2. [Stability rules](#2-stability-rules)
3. [Layers](#3-layers)
4. [Cross-cutting design](#4-cross-cutting-design)
5. [Per-crate changes](#5-per-crate-changes)
6. [Migration plan](#6-migration-plan)
7. [Open questions](#7-open-questions)

---

## 1. Evidence

### 1.1 Semver breaks that shipped or were dodged

| Where | What happened |
|---|---|
| #93 (2.2.0) | Added `hadris_fat::Error::{StaleEntry, WriterConflict}`. The PR notes `Error` is not `#[non_exhaustive]`. |
| #94 (2.2.0) | Added `HybridBootOptions::efi_boot_partition`, which broke struct-literal construction. |
| #83 | Added a `Sync` supertrait to `TimeProvider` and `OemCpConverter`, only because `FatVolume` stores `&'static dyn` providers. |
| #111 | `InputEntryKind::Source` sits behind `unstable-streaming` because the enum is exhaustive. Maintainer comment: "integrate streaming properly into the main input model in V3". |
| #115 | `SimpleFile.source` is a public field that exists only with `unstable-streaming`. Struct literals break when any crate in the graph enables the feature. |
| #96 | `InputEntryKind` has a `#[cfg(test)] TestFile` variant, so the public shape differs under test. |
| #104 | Removing `WrittenFile::additional_extents` was only safe because it had not shipped. |
| #121 | `cargo semver-checks` runs by hand, not in CI. That is how the rows above got through. |

### 1.2 No shared filesystem vocabulary

Every format crate invents its own handle, directory, entry, reader, metadata
and error types:

| Concept | FAT | exFAT | ISO | UDF | NTFS |
|---|---|---|---|---|---|
| Volume | `FatVolume` | `ExFatVolume` | `IsoImage`, `IsoReader` | `UdfVolume` | `NtfsFs` |
| Child dir | `dir.open_entry(fe)` after `de.as_entry()` | `dir.open_dir(&name)` | `entry.as_dir_ref(&image)` then `image.open_dir(dref)` | `fs.read_directory(&entry.icb)` | `dir.open_dir(name)` |
| Read a file | `FileReader`, no `Read` impl | `ExFatFileReader`, has `Read + Seek` | `read_file -> Vec` or chunk iterator | `read_file -> Vec` | `FileReader`, no seek |
| Size | `len()` | `size` field | `total_size()` | `size` field | `size()` |
| Path lookup | `open_path`, but `"/"` fails | `open_path` | `find_path` | none | `open_path` |

The conformance suite adapters (`tests/src/fat/hadris.rs`) and
`fuzz/src/bin/fs_dump.rs` each re-implement "resolve a path, walk a tree, read
a file" once per format. The FAT adapter reopens the volume for every
operation and re-resolves paths by hand because no path API exists.

`FileType` exists three times (`hadris-common`, `hadris-udf`, `hadris-cpio`)
and the ISO writer uses `std::fs::FileType`. `hadris-common::Timestamps` is
unused. There are five date types and only FAT has a clock.

### 1.3 Extensibility audit

`#[non_exhaustive]` appears only in `hadris-io`, `hadris-block` and
`hadris-optical`. It is missing from `hadris-fat`, `hadris-iso`, `hadris-udf`,
`hadris-part`, `hadris-cpio`, `hadris-cd`, `hadris-ntfs`, `hadris-storage` and
`hadris-path`. Affected user-facing types:

- Errors: every crate-level `Error`, plus `IsoCreationError`, `FileConversionError`, `IsoModifyError`, `BootError`, `VolumeError`, `NtfsError`, `FromFsError`, `PathError`.
- Options: `IsoFormatOptions`, `CreationFeatures`, `HybridBootOptions`, `RripOptions`, `BootOptions`, `FatFormatOptions`, `ExFatFormatOptions`, `UdfWriteOptions`, `CpioWriteOptions`, `OpticalImageOptions`, `HybridMbrConfig`.
- Results and entries: `DirEntry`, `RripMetadata`, `IsoSizeEstimate`, `ExFatFileEntry`, `ExFatInfo`, `PartitionInfo`, `GptDisk`, `UdfVolumeInfo`, `UdfDirEntry`, `CpioEntryHeader`, `UdfCreateOutput`, `LayoutInfo`, the `tool::` reports.
- Enums: `InputEntryKind`, `EntryType`, `PartitionScheme`, `FatType`, `FatTypeSelection`, `PartitionType`, `PartitionTable`, `DirectoryEntry`, `FileNode`, several `FileType`s.

### 1.4 Features that change public shapes

- `hadris-fat`: `write`, `cache` and `unstable-exfat` add variants to `Error`.
- `hadris-cpio`: `write` adds variants to `Error`.
- `hadris-part`: `HybridMbrConfig.mirrored` is a `Vec` with `alloc` and an array plus count without it.
- `hadris-iso`: `EntryType::Joliet` and `modify::FileData::Path` are cfg-gated variants.
- `hadris-io`: `std` swaps the blanket `Read`/`Write`/`Seek` impls from `embedded_io` types to `std::io` types. Enabling `std` anywhere in the graph removes impls a no_std user relied on.
- `hadris-part`: without `crc`, `GptDisk::update_crcs` does nothing and `write` does not enable `crc`, so a `write` build can emit invalid GPTs. Without `rand`, `GptDisk::new` uses an all-zero disk GUID.

### 1.5 Concurrency and ownership

- Every filesystem owns its device inside a `spin::Mutex`. In async builds FAT and NTFS hold the guard across `.await` (`hadris-fat/src/read.rs` around `read_chain`, `write_fsinfo`; NTFS `read.rs`, `dir.rs`, `fs.rs`). Two tasks sharing a volume on one executor can spin forever.
- `FatVolume` keeps FSInfo counts in `Cell<u32>`, so it is `!Sync`.
- `FileEntry` is a snapshot. Any change to its directory slot makes it stale (`StaleEntry`), so callers re-`find` after `truncate` or `set_times`. The FAT adapter in the test suite does exactly that. #90 and #24 were overwrite bugs from the same model.
- Ownership differs by crate: owned plus mutex (FAT, NTFS, ISO, UDF), `&mut` per call (part, `IsoReader`), a borrowed view (`PartitionView`), a consumed stream (cpio).

### 1.6 APIs that do not make sense

- `BaseIsoLevel::Level3` (interchange level 3) converts to `EntryType::Level2`, while `EntryType::Level3` means the ISO 9660:1999 enhanced tree. The mapping is correct, but two types use "Level 3" for different things and the conversion looks like a bug to every reader.
- `BaseIsoLevel` and `EntryType` both carry `{ supports_lowercase, supports_rrip }`. `supports_rrip` also duplicates `CreationFeatures::rock_ridge`, and `RripOptions.enabled` sits inside an `Option<RripOptions>`. Rock Ridge has to be enabled twice; `hadris-cd` patches over it.
- `sector_size` is a public option in ISO and CD, but anything other than 2048 is rejected.
- `IsoImageWriter::create` returns nothing useful. The CLI re-reads the PVD to learn the image size, and `hadris-cd` reopens the image it just wrote to find file extents.
- `BootInfo` is public but nothing returns one. The ISO CLI parses the boot record by hand.
- ISO has two read APIs (`IsoImage`, `IsoReader`) with parallel but different type names and borrow rules.
- `IsoModifier` rewrites the volume descriptors at sector 16 in place. It reads only the primary tree as Level 1 with lossy UTF-8 names, drops Rock Ridge, rebuilds Joliet from primary names, reads one extent per record, and ignores the backup GPT.
- `hadris-udf/src/modify.rs` is not compiled. It is a stub with hard-coded partition geometry.
- `hadris-cd` exposes layout state (`FileEntry.extent`, `Directory.iso_extent`) on its input tree.
- FAT and exFAT use different names and size types for the same operations. `FatDir::open_file` takes a name, `ExFatVolume::open_file` takes a path. exFAT entries have public mutable fields, FAT entries have getters.
- FAT writes are implicit overwrites. The tail is freed only in `finish()`. Dropping a writer without `finish()` silently loses size and timestamps, or panics under `dirty-file-panic`. `sync()` and `flush()` do different things and `flush()` exists only with `cache`.
- FAT mutations live on the volume and take `&FatDir`, lookups live on `FatDir`. `create_dir` returns `FatDir`, `create_file` returns `FileEntry`. There are three ways to read a file.
- `set_times(entry, Option<FatDateTime>, Option<u16>, Option<FatDateTime>)`, `accessed_date() -> u16` (raw packed date), `set_root_label(&[u8; 11])` despite a `VolumeLabel` type.
- `hadris-part` has six extension traits to read or write one partition table. `Guid::from_str` is an inherent `const fn` returning `Option`, shadowing `FromStr`. `PartitionInfo::size_bytes()` hard-codes 512.
- `hadris_io::Error::erase` discards the device error. Every format error goes through it.
- `hadris-cpio` has pairs such as `skip_entry_data`/`skip_entry_data_owned` and needs a buffer the size of the whole file to read an entry. The writer takes every file as a `Vec<u8>`.
- Two identical `FileSource` types (ISO, UDF), both hard-wired to blocking `std::io::Read`, even in async mode. Six input-tree types across ISO, UDF, CD and cpio.
- Directory iterators returned the same `Err` forever (#113, and again in `PathTableEntryIter` in #120). There is no written iterator contract.
- Internal planning types are public via glob re-exports: `PendingRecords`, `LayoutManager`, `WrittenFiles`, `RelocationMap`, `LogicalSector`, `ExFatVolume::allocate_cluster`, `FatSectorCache::new(5 usizes)`, and more.
- Panics in library code: `RootDirs::best_choice`, `VolumeDescriptorList::primary`, `IsoStr::as_str` on non-UTF-8 disk bytes, many `expect("Fixed root info required ...")` in FAT write paths.
- `#[allow(clippy::too_many_arguments)]` on public or semi-public builders in ISO, FAT and cpio. `RawNewcHeader::build` takes 14 positional `u32`s.

### 1.7 Dead weight

- `hadris-archive` is an 8-line re-export of `hadris-cpio`.
- `hadris-storage` (`BlockDevice`, `SeekBlockDevice`, `BlockGeometry`) is used only by `hadris-block`. No filesystem uses it.
- `hadris-cpio` depends on `hadris-common` without using it, and its `std` feature pulls in chrono, crc and rand through it.
- Unused in `hadris-common`: `optical::*`, `RingBuf`, `BOOT_SECTOR_BIN`, `Crc32HasherIsoHdlc`, `MaybePod`, `Timestamps`, and the chrono and rand dependencies. Three endianness concepts plus a fourth copy in `hadris-fixed`.
- `hadris-io`: unused `cfg-if`, a no-op `alloc` feature, `FromEmbedded`/`ToEmbedded` unused.
- Sync/async is implemented three ways: hand-written trait pairs in `hadris-io`, hand-copied files in `hadris-storage` and `hadris-block`, and `strip_async!` in the format crates.

---

## 2. Stability rules

These apply to every published crate and CI enforces them.

### R1. Every user-facing enum is `#[non_exhaustive]`

Errors, error kinds, option enums, entry kinds, partition types, namespaces.
The exception is closed on-disk code sets that the specification cannot
extend, and those live in `raw` (R4).

### R2. User-facing structs have private fields

- **Options** are `#[non_exhaustive]`, implement `Default`, and take consuming `with_*` setters. Presets are associated functions that return a configured value. They compose because they are starting points:

  ```rust
  let opts = IsoOptions::default()
      .with_level(IsoLevel::L3)
      .with_joliet(JolietLevel::L3)
      .with_rock_ridge(RockRidge::default())
      .with_hybrid(HybridBoot::gpt());
  ```

- **Results and entries** expose getters only. Construction is crate-private.
- **Plain value types** that are complete by definition (`Guid`, `DateTime`, `BlockIndex`, `NodeId`) may be exhaustive, but fields stay private with `const fn` constructors and accessors.

### R3. No `cfg` on variants, fields or trait bounds of public types

A feature may add items (types, functions, modules, impls). It never changes
the shape of an existing item. `ErrorKind::NoSpace` exists in every build even
though only a `write` build produces it.

Previews use a separate module, for example `hadris_fat::unstable::exfat`,
never cfg-gated variants. Anything outside `unstable` is covered by semver.

### R4. On-disk layouts live in `raw`

`#[repr(C)]` structs, bitflags that mirror disk bytes and specification
constants live under `crate::raw`. They are public, exhaustive and documented
as "mirrors the specification; may gain items, existing items follow the
spec". The crate root never glob re-exports them.

### R5. No glob re-exports at crate roots

Crate roots re-export a curated list, and the root never changes meaning with
features. Today `hadris_iso::read::IsoImage` silently means the sync one.
`pub use sync::*` at the root goes away. Users write
`hadris_iso::sync::IsoImage` or `hadris_iso::r#async::IsoImage`.

### R6. No panics on disk data or user input

Library code returns errors. `expect` and `unwrap` are allowed only on
invariants the same function established. No feature turns an error into a
panic, so `dirty-file-panic` is removed. Only fallible forms exist; there are
no `try_*` twins.

### R7. Iterators fuse after an error

Any iterator yielding `Result` yields at most one `Err` and then `None`. A
shared `FuseOnError` adapter in `hadris-fs` implements this, and each iterator
has a test for it.

### R8. Sizes and offsets are `u64`

File sizes, offsets and byte counts that can exceed 4 GiB are `u64` in every
crate, including 32-bit targets. On-disk narrowing uses checked conversion and
returns `ErrorKind::LimitExceeded`.

### R9. No bool parameters in public functions

Use an enum, a flags type or an options struct. Public functions with more
than four parameters take a struct.

### R10. Traits can grow

Public traits that users implement (`FileSystem`, `FileSystemMut`,
`BlockDevice`, `ByteSource`, `Clock`, `Lock`) are not sealed. Methods added in
3.x must have a default body. For filesystem operations the default returns
`ErrorKind::Unsupported`, and `Capabilities` gains a matching flag that
defaults to off. Traits that only Hadris implements are sealed.

### R11. CI enforces it

- `cargo semver-checks` on every PR against the latest 3.x release (after 3.0.0).
- The public-API snapshot runs with all non-`unstable` features on, and a second run with them off must produce a subset. That proves R3.
- A lint script rejects public enums without `#[non_exhaustive]` outside `raw`.
- A sync/async parity check diffs the public item lists of the two modules.

---

## 3. Layers

```
hadris-io        embedded-io traits, StdIo adapter, ByteSource, io::Error
hadris-storage   BlockDevice, device adapters, Slice, Cache
hadris-fs        shared vocabulary, FileSystem traits, path layer, handles,
                 Shared<F, L>, Tree/Content for writers
format crates    native API with full fidelity + FileSystem impls
hadris-block,    detection and dispatch; OpenVolume / OpenOpticalImage
hadris-optical   implement FileSystem by delegation
hadris-vfs       object-safe DynFileSystem, host helpers, FUSE (std only)
hadris           umbrella with flat paths
```

Two ideas carry the design.

**One common trait, not one common API.** `FileSystem` and `FileSystemMut`
cover what every filesystem can express: look up a name, read a directory,
read and write bytes at an offset, change metadata. Generic code, the
conformance suite, `hadris-vfs` and kernel integrations target these traits.
Format crates keep a native API for what the trait does not model: formatting,
fsck, FAT attributes and cluster chains, ISO namespaces and boot catalogs,
NTFS streams. The native API and the trait impl share one implementation, so
the trait never falls behind.

The trait is the wrong abstraction for three jobs, and V3 does not force it on
them:

- Build-once writers (ISO, UDF, CD, cpio, GPT) take a whole `Tree` and produce an image. They share `Tree`, `Content` and a report shape, not a trait. Generic code over "any image writer" has little use because the options differ per format.
- Streaming archives (cpio, later tar) are forward-only. They get an entry reader, not random access.
- Tools (fsck, analysis, the ISO verifier) use `raw` and native types.

**The library holds no locks.** Volumes take `&mut self` and contain no mutex.
Callers choose how to share through `Shared<F, L>`, which is generic over the
lock. [Section 4.4](#44-sharing-and-locking) explains why.

---

## 4. Cross-cutting design

### 4.1 `hadris-io`

**Traits.** Keep `hadris_io::{Read, Write, Seek}` and their async
counterparts, rebased on `embedded-io`:

- `impl<T: embedded_io::Read + ?Sized> Read for T` is the only blanket impl, and it exists in every build. This addresses #16 and #18.
- `std` adds `hadris_io::StdIo<T>`, a newtype that implements the Hadris traits for `T: std::io::Read/Write/Seek`. Hadris handles implement `std::io` traits under `std`. Enabling `std` only adds items.
- `impl<T: Read + ?Sized> Read for &mut T` in sync mode too. The `Borrowed` wrapper goes away.
- `ReadWrite` and `ReadWriteSeek` exist in both modes.

```rust
let file = std::fs::File::open("disk.img")?;
let dev = hadris_storage::sync::StreamDevice::new(StdIo::new(file), BlockSize::B512)?;
let vol = hadris_fat::sync::FatVolume::open(dev, VolumeOptions::default())?;
```

**Error.** `hadris_io::Error` keeps the original error:

```rust
#[non_exhaustive]
pub struct Error {
    kind: ErrorKind,
    #[cfg(feature = "alloc")]
    source: Option<Box<dyn core::error::Error + Send + Sync>>,
}
```

The field is private, so the `alloc` gate does not change the public shape. With
`alloc`, `source()` returns the device error. Without `alloc`, only the kind
survives, which matches V2. `erase` is removed. `core::error::Error` is used
everywhere, so error traits need no `std` gate.

A generic `Error<E>` was the other option. It is lossless without `alloc`, but
it puts a type parameter on every signature and makes errors from two devices
different types. Kernels without `alloc` get the kind, which is what they map
to errno anyway.

**Byte sources.** One source type for every writer input replaces the two
`FileSource` copies:

```rust
pub trait ByteSource {
    fn len(&self) -> u64;
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize>;
}
```

`ByteSource` is positional so writers can read a file twice (checksum pass,
data pass) without a seek contract. The async module has the same trait with
`async fn read_at`. `hadris-fs::Content` wraps it (4.7).

### 4.2 `hadris-storage`

Filesystems read from a `BlockDevice`, not a byte stream. That fixes three V2
problems: the cache can sit under every format instead of inside FAT, 4Kn
devices and 2048-byte optical media stop being special cases, and a partition
is just another device.

```rust
pub trait BlockDevice {
    fn block_size(&self) -> BlockSize;
    fn block_count(&self) -> u64;
    fn access(&self) -> Access;                                   // ReadOnly | ReadWrite
    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()>;
    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> { unsupported }
    async fn flush(&mut self) -> Result<()> { Ok(()) }
}
```

`async fn` here means "written once, generated for both modes" (4.8).

Provided devices and adapters:

| Type | Purpose |
|---|---|
| `impl BlockDevice for &mut D` | Borrow a device instead of moving it in. |
| `StreamDevice<T>` | Any `Read + Seek` (optionally `Write`) byte stream, with a block size the caller picks. The migration path for every V2 user. |
| `MemDevice<B>` | `&mut [u8]`, `Vec<u8>` with `alloc`. For tests and in-memory images. |
| `Slice<D>` | A block range of `D`. `D` can be owned or `&mut`. Replaces `PartitionView`. |
| `Cache<D>` | Write-back LRU over whole blocks. Explicit `flush`. Capacity set at construction. Works in both modes. Kernels skip it. |
| `ByteView` (crate-internal helper used by format crates) | Byte-granular read and read-modify-write on top of a `BlockDevice`, for records that straddle blocks. |

A filesystem sector can be larger than the device block (FAT 4096-byte sectors
on a 512-byte image) but not smaller unless the device is a `StreamDevice`,
which accepts any block size. Formats check this at open and return
`ErrorKind::Unsupported` otherwise.

cpio stays on `Read`/`Write` streams, since it must work on pipes.

`hadris-storage` stays a separate crate. The V2 idea of folding it into
`hadris-block` creates a cycle: format crates need `BlockDevice`, and
`hadris-block` depends on the format crates.

### 4.3 `hadris-fs`: shared vocabulary and traits

New crate. It takes the useful parts of `hadris-common` and `hadris-path`.

**Vocabulary.**

| Type | Notes |
|---|---|
| `NodeId` | Opaque `u64`. Stable for as long as the node is pinned (4.5). Maps directly to FUSE `ino` and kernel inode numbers. |
| `FileType` | `File`, `Dir`, `Symlink`, `CharDevice`, `BlockDevice`, `Fifo`, `Socket`. Non-exhaustive. One definition for every crate. |
| `Name` / `NameBuf<N>` | Names are bytes. `Name::to_str()` returns `Result`. `NameBuf` is a fixed-capacity buffer so no-alloc callers can read directories. |
| `Metadata` | `file_type`, `len: u64`, `times: FileTimes`, `permissions: Option<Mode>`, `owner: Option<(u32, u32)>`, `nlink`, plus `attributes: Attributes` (DOS-style flags) and an `extra()` hook for per-format types. Getters only. |
| `DateTime` | Private fields: seconds since 1970, nanoseconds, optional UTC offset in minutes. Every format converts to and from its own encoding in `raw`. |
| `FileTimes` | `created`, `modified`, `accessed`, `changed`, each `Option<DateTime>`, with `with_*` setters. Used for both reads and `set_metadata`. |
| `Clock` | `fn now(&self) -> DateTime`. `NoClock` (fixed documented epoch) is the default, `SystemClock` with `std`. |
| `Capabilities` | Flags and limits: writable, symlinks, hard links, case sensitivity (`Sensitive`, `InsensitivePreserving`, `Insensitive`), max name length, name charset, supports permissions, supports owners, timestamp resolution. |
| `FsStats` | Total, free and used blocks, block size, file count if known. |
| `ErrorKind` | The shared kind set (4.6). |
| `VPath` | The `hadris-path` type, moved here. Path parsing is convenience only; the traits take names. |

**The read trait.**

```rust
pub trait FileSystem {
    fn capabilities(&self) -> Capabilities;
    fn root(&self) -> NodeId;

    async fn lookup(&mut self, dir: NodeId, name: &Name) -> Result<NodeId>;
    async fn metadata(&mut self, node: NodeId) -> Result<Metadata>;
    async fn read_dir(&mut self, dir: NodeId, cursor: &mut DirCursor, name: &mut NameBuf)
        -> Result<Option<DirEntry>>;
    async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> Result<usize>;
    async fn read_link(&mut self, node: NodeId, target: &mut NameBuf) -> Result<()> { unsupported }
    async fn stats(&mut self) -> Result<FsStats>;
    fn forget(&mut self, node: NodeId) {}
}
```

- `lookup` pins the node it returns. `forget` unpins it. This is the FUSE `lookup`/`forget` contract. ISO, UDF and NTFS have naturally stable IDs, so `forget` does nothing for them.
- `read_dir` is a resumable cursor. `DirCursor` is a `Copy` value that the caller can store and reuse, which FUSE `readdir(offset)` and kernel `getdents` need. It returns one entry per call into a caller buffer, so it works without `alloc`. `DirEntry` carries the name length, `FileType`, and the entry's `NodeId`. Entries from `read_dir` are not pinned; a caller that wants to keep one calls `lookup`.
- Everything is positional. There is no cursor inside a file node.
- `.` and `..` never appear in `read_dir` output and `lookup` rejects them. The path layer handles `..` itself by tracking the parent chain.

**The write trait.**

```rust
pub trait FileSystemMut: FileSystem {
    async fn create(&mut self, dir: NodeId, name: &Name, kind: NewNode<'_>, meta: &SetMetadata)
        -> Result<NodeId>;                                  // File | Dir | Symlink(target) | Device(..)
    async fn remove(&mut self, dir: NodeId, name: &Name) -> Result<()>;
    async fn rename(&mut self, from_dir: NodeId, from: &Name, to_dir: NodeId, to: &Name,
        flags: RenameFlags) -> Result<()>;                  // NoReplace | Exchange later via R10
    async fn write_at(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> Result<usize>;
    async fn set_len(&mut self, node: NodeId, len: u64) -> Result<()>;   // grow or shrink
    async fn set_metadata(&mut self, node: NodeId, changes: &SetMetadata) -> Result<()>;
    async fn sync_node(&mut self, node: NodeId) -> Result<()>;
    async fn sync(&mut self) -> Result<()>;
}
```

- `rename` keeps the `NodeId` of the moved node. That is the point of the open-node table (4.5).
- A failed operation leaves the volume unchanged. The conformance suite already tests this ("rejection" scenarios), and the trait docs make it part of the contract.
- `sync` writes every piece of cached metadata (FSInfo, dirty FAT sectors, directory entries) and flushes the device. There is one durability call, not V2's `sync` plus `flush`.

**Volume-specific operations stay native.** Labels, formatting, fsck, FAT
attribute bits beyond `Attributes`, cluster chains, ISO namespaces and NTFS
streams are inherent methods on the format types.

**The path and handle layer.** An extension trait with a blanket impl gives
every `FileSystem` a std-like API. It lives in `hadris-fs`, so it is written
once:

```rust
use hadris_fs::sync::prelude::*;

let mut vol = FatVolume::open(dev, VolumeOptions::default())?;
let mut f = vol.open("/EFI/BOOT/BOOTX64.EFI", OpenOptions::read())?;   // File<'_, _>: Read + Seek
let mut log = vol.open("/log.txt", OpenOptions::write().create().append())?;
log.write_all(b"hello")?;
log.close()?;                                   // returns errors; Drop is best effort
vol.create_dir_all("/a/b/c")?;
vol.rename("/a/b", "/a/renamed")?;
vol.remove_dir_all("/a")?;
let meta = vol.metadata("/EFI")?;
for entry in vol.read_dir("/EFI")? { let entry = entry?; }       // fuses on error
vol.sync()?;
```

- `File<'a, F>` and `Dir<'a, F>` hold a `NodeId`, a position and a borrow of `F`. They implement `hadris_io::{Read, Write, Seek}`, `std::io` traits with `std`, and `Iterator` for directories in sync builds.
- `OpenOptions { read, write, append, truncate, create, create_new }` makes overwrite semantics explicit (#90, #91).
- `close()` returns `Result`. `Drop` does a best-effort `forget`, never flushes and never panics. `#[must_use]` on handles.
- Helpers: `exists`, `read_to_vec`, `write_all_to`, `create_dir_all`, `remove_dir_all`, `copy_tree` (between any two `FileSystem`s), and with `std`, `extract_to_host` and `import_from_host`. The host helpers reject absolute names and `..` components so archives and images cannot escape the target directory.
- `F` can be the volume itself (one handle at a time, borrow-checked) or `&Shared<V, L>` (many handles, 4.4).

The conformance suite's FAT adapter becomes one generic impl over
`FileSystemMut`. The rust-fatfs and mtools peers can keep their own adapters,
or implement the trait themselves.

### 4.4 Sharing and locking

Volumes take `&mut self` and hold no lock. Sharing is a wrapper:

```rust
pub struct Shared<F, L: Lock> { /* L wraps F */ }

impl<F: FileSystem, L: Lock> FileSystem for &Shared<F, L> { .. }
impl<F: FileSystemMut, L: Lock> FileSystemMut for &Shared<F, L> { .. }
```

Because `&Shared` implements the traits, every helper in 4.3 works on it, and
many `File` handles can be open at once. With `alloc`, `Arc<Shared<F, L>>`
gives owned handles (`OwnedFile<F, L>`) that can move between tasks or live in
a kernel file table.

`Lock` is generated for both modes:

| Mode | Lock implementations |
|---|---|
| sync | Any `lock_api::RawMutex` (spin, parking_lot, critical-section based mutexes). `SingleThread` wraps `RefCell` for single-threaded no-alloc users. `StdMutex` with `std`. |
| async | A small `AsyncLock` trait. Impls behind features for `embassy-sync` and `async-lock`. Users can implement it for tokio's mutex in a few lines. |

The lock is held for one trait call. A `read_at` on one file and a `lookup` on
another serialize, which V2 already does through its internal mutex.

The other option was `&self` methods with a lock inside every format crate.
It allows concurrent readers in principle, but each crate has to pick a lock,
async builds need an async lock inside the format crate, and the V2 bug class
(guard held across `.await`) stays possible. With the lock outside, the format
crates cannot make that mistake because they never see a lock. Real read
concurrency needs a device that supports concurrent positional reads plus
finer locks around the node table and cache. If a kernel user needs that,
`Shared` can grow an `RwLock` mode in 3.x without changing the traits. See
[Q1](#7-open-questions).

### 4.5 Node identity: the open-node table

FAT and exFAT have no inodes. The natural identity of a file is the location
of its directory entry, and that changes on rename. V2's `FileEntry` snapshot
model produced `StaleEntry`, #90 and #24.

V3 keeps an open-node table inside `FatVolume` (and `ExFatVolume`):

- A `NodeId` is an index into the table. The entry records the directory-entry location, the first cluster, the size, the pin count and whether a writer holds it.
- `lookup` finds or creates the table entry and increments the pin count. `forget` decrements it, and the entry is freed at zero.
- `rename` updates the location in place, so the `NodeId` stays valid. `set_len` and `write_at` update the size in the table, so every handle sees the same size and the directory entry is written on `sync_node` or `sync`.
- `remove` on a pinned node deletes the directory entry but keeps the clusters until the last pin goes away, matching POSIX unlink. See [Q3](#7-open-questions).
- IDs of unpinned entries from `read_dir` are valid until the next mutation of that directory. The docs state this, and `lookup` is the way to keep one.

The table needs `alloc`, and FAT write already needs `alloc`. A no-alloc,
read-only `FatVolume` uses the directory-entry location as the `NodeId`,
which is stable because nothing moves.

ISO (directory record location), UDF (ICB location and partition) and NTFS
(MFT reference with sequence number) have stable IDs that fit in a `u64`, so
they need no table.

### 4.6 Errors

Each crate has exactly one error type, `hadris_<crate>::Error`:

```rust
#[non_exhaustive]
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    context: Context,          // private: sector, cluster, node, field name
    source: Option<hadris_io::Error>,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Io,
    NotFound,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    NoSpace,
    ReadOnly,
    InvalidInput,        // bad options, names or arguments from the caller
    Corrupt,             // disk data violates the specification
    Unsupported,         // valid but not implemented, or not in Capabilities
    LimitExceeded,       // a value does not fit the on-disk field
    InvalidHandle,       // unknown or forgotten NodeId
    Busy,                // e.g. a second writer, or removing a mounted root
}
```

- `hadris-fs` defines `ErrorKind` once. Crates re-export it, and the traits use `hadris_fs::Error`, which every crate error converts into without losing kind or source.
- Callers match on `err.kind()`. New failure modes add context, not kinds.
- Crate-specific detail comes through typed accessors (`err.sector()`, `err.cluster()`) and a `#[non_exhaustive] enum Detail` where matching is useful.
- No `String` payloads without `alloc`. No foreign types (`bytemuck::PodCastError`, `PathBuf`) in the public API.
- Wrapper crates (`hadris-cd`, `hadris-optical`, `hadris-block`) hold the inner error as `source`, keep its kind, and add context.
- Module-level `Error`/`Result` aliases (`write::Error`, `modify::Error`) are removed.

### 4.7 Shared input tree for writers

ISO, UDF, CD and cpio share one tree in `hadris-fs::tree` (behind `alloc`):

```rust
let mut tree = Tree::new();
tree.add_file("boot/grub/grub.cfg", Content::bytes(cfg))?;
tree.add_file("install.wim", Content::path("/data/install.wim"))?;   // std, opened lazily
tree.add_file("big.bin", Content::source(my_byte_source))?;
tree.add_dir("empty")?;
tree.add_symlink("latest", "releases/3.0")?;
tree.add_device("dev/console", DeviceKind::Char, DeviceNumber::new(5, 1))?;
tree.add_hard_link("bin/sh", "bin/busybox")?;
tree.set_metadata("install.wim", SetMetadata::default().with_mode(Mode::new(0o644)))?;

#[cfg(feature = "std")]
let tree = Tree::from_fs("/path/to/root", FromFsOptions::default())?;   // lazy, detects hard links
```

- `Content` is opaque (bytes, boxed `ByteSource`, lazy path), so new kinds of content never break callers. It does not derive `PartialEq`.
- Streaming is the normal path. `unstable-streaming` goes away.
- Paths use `/`. `PathSeparator` is removed from the writer API.
- Nodes carry `SetMetadata`. Formats ignore what they cannot store and report it through the writer's `Report::warnings()`.
- No layout state on the tree. Writers keep their planning structures `pub(crate)`.
- `Tree::from_fs` reports unreadable entries as errors or warnings (`FromFsOptions::on_error`) instead of dropping them.
- `Tree::from_filesystem(&mut impl FileSystem)` builds a tree from any mounted volume, which gives FAT-to-ISO conversion and image round-trips for free.

Every writer returns a report:

```rust
let report = hadris_iso::sync::write(&mut out, &tree, &opts)?;
report.total_blocks(); report.size_bytes(); report.warnings();
report.extent_of("install.wim");                   // Option<Extent>, used by hadris-cd
```

### 4.8 Sync and async

Keep the `strip_async!` code generation and use it everywhere:

- `hadris-io`, `hadris-storage` and `hadris-block` move from hand-copied files to the same generator. Every async fix is written once.
- Mode-independent types (raw layouts, names, options, metadata, errors, trees, `NodeId`) are defined once, outside the generated modules.
- Only types that do I/O live in `sync` and `async`.
- Every crate exposes the same public items in both modes. The parity check in R11 enforces it. V2 gaps to close: ISO async write and modify, exFAT async, CD async, UDF async writer, FAT cache on the async path (solved by `Cache<D>`), fsck in async.
- `hadris-macros` gains span-preserving errors so contributors see the right line.
- Traits in the async module use `async fn`. See [Q2](#7-open-questions) for the `Send` question.

### 4.9 Feature flags

| Feature | Meaning |
|---|---|
| `alloc` | Heap-backed conveniences: owned names, trees, boxed sources, error sources, the FAT node table, FAT write. |
| `std` | Implies `alloc`. `StdIo`, `std::io` impls on handles, `Content::path`, `SystemClock`, host helpers. |
| `sync` | Sync API. On by default. |
| `async` | Async API. |
| `write` | Writers, formatters, modifiers. Enables every correctness dependency (CRC and so on). |
| `unstable-*` | Enables an `unstable` module. Never changes stable items. |

- `read` is dropped; reading is always available.
- `crc` and `rand` stop being user-facing features. CRC is always compiled with `write`. Random GUIDs come from the caller (4.10).
- `cache` stops being a feature. `Cache<D>` is always available and costs nothing unless constructed.
- The umbrella forwards the same axes plus one feature per format.

A no-alloc FAT write tier (fixed-capacity node table through a const generic)
is possible later without changing any shape, so it is not in 3.0.

### 4.10 Partition GUIDs and randomness

No hidden RNG. `Gpt::new(disk_guid: Guid, ..)` and `GptEntry::new(unique_guid:
Guid, ..)` take GUIDs. `Guid::random()` exists with `std`. Formatters that need
a volume serial take one in options, defaulting to one derived from the clock.

### 4.11 Crate layout

| V2 crate | V3 |
|---|---|
| `hadris-io` | Kept. Traits, `StdIo`, `Error`, `ByteSource`. |
| `hadris-storage` | Kept and adopted by every filesystem. `BlockDevice`, adapters, `Slice`, `Cache`. |
| `hadris-fs` | New. Vocabulary, traits, path and handle layer, `Shared`, `Tree`/`Content`, `FuseOnError`, `VPath`. |
| `hadris-common` | Internal. Endianness and fixed-size string types, merged with `hadris-fixed` into one set. Documented as not for direct use. |
| `hadris-fixed` | Merged into `hadris-common`. |
| `hadris-path` | Merged into `hadris-fs`. |
| `hadris-macros` | Kept, internal. |
| `hadris-archive` | Removed. The umbrella re-exports `hadris-cpio` directly. |
| `hadris-block`, `hadris-optical` | Detection and dispatch. `OpenVolume` and `OpenOpticalImage` implement `FileSystem` by delegation. |
| `hadris-vfs` | New, `std` only, can ship in 3.x. `DynFileSystem` (object-safe, boxed futures in async), a mount table for composing volumes, and a `fuser` adapter. |
| Format crates | Kept. |
| `hadris` | Re-exports `io`, `storage` and `fs` so no_std users need one dependency. Flat paths: `hadris::fat`, `hadris::iso`. |

---

## 5. Per-crate changes

Each section lists the API changes, then the V3 feature work. Items marked
"3.x" may land after 3.0.0 because their shape ships in 3.0.

### 5.1 `hadris-fat`

**One shape for FAT12/16/32 and exFAT.**

```rust
let mut vol = FatVolume::open(dev, VolumeOptions::default().with_clock(SystemClock))?;
vol.kind();                                      // FatKind::{Fat12, Fat16, Fat32, ExFat}
vol.label()?; vol.set_label(&VolumeLabel::new("BOOT")?)?;
vol.fat_attributes(node)?; vol.set_fat_attributes(node, FatAttributes::HIDDEN)?;
vol.cluster_chain(node)?;                        // native, for tools
// Everything else goes through FileSystem / FileSystemMut and the path layer.

let vol = hadris_fat::sync::format(dev, &FormatOptions::default().with_kind(FatKind::Fat32))?;
let report = hadris_fat::sync::check(&mut vol)?;           // fsck, both modes
```

- `FatVolume` implements `FileSystem` and `FileSystemMut`. Path methods come from the `hadris-fs` path layer. `FatVolumeReadExt` and `FatVolumeWriteExt` are removed.
- The node table (4.5) replaces `FileEntry` snapshots. `StaleEntry` and `WriterConflict` go away as errors; a second writer on the same node shares the node and its size.
- Clock and code page are generic parameters with zero-sized defaults: `FatVolume<D, C: Clock = NoClock, P: CodePage = Ascii>`. No `'static` borrows, no `Sync` supertraits (#83).
- The FAT sector cache is gone as a separate thing. Users wrap the device in `Cache<D>`. `FatSectorCache`, `CachedFat`, `with_cached_fat` and `fat_cache` are removed (#27). Chain caching becomes an internal detail of the node table.
- `FormatOptions` (non_exhaustive, `with_*`) covers FAT and exFAT with a `FatKind` selection. It replaces `FatVolumeFormatter`, `FatFormatOptions`, `ExFatFormatOptions`, `format_exfat` and `ExFatLayoutParams`. Formatting uses the device's block count, so pre-sizing and a separate size argument go away, and formatting a `Slice` inside a disk works.
- `tool::` becomes `check` (fsck) and `analysis` in both modes, with getters on reports. A `repair` pass is 3.x.
- `expect("Fixed root info required ...")` sites return `ErrorKind::Corrupt`.
- exFAT internals (`allocate_cluster`, `sync_bitmap`, `name_hash`, `parse_entry_set`) become private.

**Feature work.**

- Random-access writes, read-write handles, grow through `set_len`.
- Create the root label entry when absent, and write the BPB label.
- Exact free space on FAT12/16 by scanning, cached after first use.
- `remove` of a pinned node defers freeing (4.5).
- Close audit items C2 (cancellation safety of compound async operations) and B3 to B7, or confirm they are fixed.
- exFAT: async, rename, attributes and times, label, directory growth, fragmented bitmap and upcase table, entry sets that cross clusters, fsck. exFAT stays in `hadris_fat::unstable::exfat` until it passes the conformance suite, then moves to the stable `FatVolume` in a 3.x minor. See [Q5](#7-open-questions).
- TexFAT and fsck repair: 3.x.

### 5.2 `hadris-iso`

**Reading.** One reader, `IsoImage<D>`, in each mode:

```rust
let mut iso = IsoImage::open(dev)?;
let ns = iso.namespaces();                         // what the image has
let mut view = iso.view(Namespace::Preferred)?;    // IsoView<'_, D>: FileSystem
let entry = view.metadata_path("/boot/grub/grub.cfg")?;
let rr = view.rock_ridge(node)?;                   // Option<RockRidgeInfo>, native
let boot = iso.boot_catalog()?;                    // Option<BootCatalog>, native
```

- `IsoReader` (no-alloc) and `IsoImage` (alloc) merge. The no-alloc core is the implementation, and `alloc` adds convenience on the same types.
- An ISO has up to four trees (primary, Joliet, Rock Ridge over primary, enhanced). `IsoView` picks one and implements `FileSystem` over it. `Namespace::Preferred` keeps V2's preference order, but Rock Ridge no longer hides Joliet names; the caller can pick.
- `DirectoryRef`, `LogicalSector` and raw records are not needed to walk a tree. They stay under `raw` and via `view.raw_record(node)` for the CLI verifier.
- `BootCatalog` is public and readable. The CLI stops parsing boot records by hand.
- `IsoStr::as_str` returns `Result`. Panicking `best_choice` and `primary` are removed.
- Non-2048 logical block sizes work in the unified reader, because the device layer already handles block size.

**Writing.**

```rust
#[non_exhaustive]
pub struct IsoOptions { /* private */ }
// with_volume(VolumeIdentifiers), with_level(IsoLevel), with_joliet(JolietLevel),
// with_rock_ridge(RockRidge), with_el_torito(ElTorito), with_hybrid(HybridBoot),
// with_charset(Charset::Strict | Relaxed), with_min_blocks(u64), with_clock(C)

let report = hadris_iso::sync::write(&mut out, &tree, &opts)?;
```

- `IsoLevel::{L1, L2, L3}` replaces `BaseIsoLevel` and `EntryType`. Lowercase handling is `NameCase`. Rock Ridge is present if and only if `with_rock_ridge` is set. `supports_rrip`, `RripOptions.enabled` and `long_filenames` are removed. The enhanced tree becomes `with_enhanced_tree(..)`, so "Level 3" means one thing.
- `sector_size` is removed.
- `create_with_allocation_floor` becomes `with_min_blocks`.
- `ElTorito` holds `Vec<BootEntry>`, each with platform, emulation and an image given as a tree path. `BootSectionOptions` and the tuple list are removed.
- `RockRidge::with_relocation(Relocation::Directory(name) | Reject)` covers #123 and #124.
- The writer runs in both modes and without `std` (clock injected, `Content` without paths).
- The output needs `BlockDevice`. A two-pass `write_stream` that needs only `Write` is 3.x; it computes the layout first and then emits blocks in order.

**Sessions and modification.** `IsoModifier` is replaced by a session API that
reads the full existing image (all namespaces, Rock Ridge, multi-extent,
boot catalog) into a `Tree` with `Content` pointing back at existing extents:

```rust
let mut session = hadris_iso::sync::Session::open(&mut dev)?;
session.tree_mut().add_file("new.txt", Content::bytes(b"hi"))?;
session.tree_mut().remove("old.txt")?;
let report = session.write(&opts, SessionMode::Append)?;   // new session after the last one
// SessionMode::Rewrite rebuilds in place for rewritable media and images
```

`Append` writes a real new session: data and descriptors after the previous
session, previous extents reused. `Rewrite` replaces V2's in-place behaviour,
keeps hybrid boot data and updates the backup GPT.

**Feature work.** Joliet beyond the BMP, zisofs read and write, RRIP SF and
RR, Apple Partition Map in hybrid images. All four fit existing shapes and can
be 3.x.

**Visibility.** `PendingRecords`, `FileTreeWalker`, `WrittenFiles`,
`DirectoryId`, `MovedDirectory`, `RripBuilder`, `SystemUseBuilder`,
`ElToritoWriter`, `IsoCursor`, `convert_l1/l2/l3` become `pub(crate)`.

### 5.3 `hadris-udf`

- `UdfVolume` implements `FileSystem`: path lookup, streaming `read_at`, and metadata with times, permissions and owners. V2 has only `read_file -> Vec` and no path lookup.
- A no-alloc read path. V2 needs `alloc` for everything.
- The writer takes the shared `Tree` and `UdfOptions` (non_exhaustive, `with_*`) and returns `UdfReport`. `SimpleFile` and `SimpleDir` are removed.
- Low-level descriptor writers (`write_lvid(location, close: bool)`, `write_fids`) become `pub(crate)` or move to `raw::write` with enums instead of bools.
- No trait bounds on the `UdfVolume` struct definition.
- `write` no longer needs `std`.
- The dead `modify.rs` is deleted. UDF on random-access media is a real read-write filesystem, so modification means `UdfVolume` implementing `FileSystemMut` for Type 1 partitions, not a second modifier API. The shape ships in 3.0 with `Capabilities::writable` false; the implementation can be 3.x.
- Reader coverage: UDF 1.50 and 2.01 reading, prevailing-descriptor selection, allocation-extent chaining, extended allocation descriptors, stream directories.
- 3.x: VAT, sparing tables, metadata partitions (2.50+), which all live behind the same `UdfVolume`.

### 5.4 `hadris-cd`

- One entry point: `hadris_cd::sync::write(target, &tree, &CdOptions)`. `CdOptions` holds `IsoOptions` and `UdfOptions` and re-exports them. The `new(..).finish(..)` and `create(..)` pair with swapped arguments goes away.
- It uses `report.extent_of(..)` from the ISO writer instead of reopening the output.
- Metadata, symlinks and streaming inputs come from the shared `Tree`.
- `LayoutManager` becomes private.
- Async support, since both underlying writers have it.

### 5.5 `hadris-ntfs`

- `NtfsVolume` (renamed from `NtfsFs`) implements `FileSystem`, with metadata including times and security descriptors through `extra()`.
- `NtfsError` becomes `Error`. `raw::*` is no longer glob re-exported. `attr` types with raw `u8`/`u32` codes move to `raw`; the public API uses enums.
- Native API for streams: `vol.streams(node)` lists named data streams, and `vol.read_stream_at(node, name, offset, buf)` reads one.
- `open` seeks to the boot sector instead of reading from the current position (falls out of `BlockDevice`).
- Reachable from `hadris-block` detection, `OpenVolume` and the umbrella.

**Feature work.** `$ATTRIBUTE_LIST`, `$MFTMirr` fallback, compressed streams,
reparse points (exposed as symlinks where they are symlinks or junctions),
keyed B-tree lookup, filtering DOS 8.3 duplicates from listings. Encrypted
streams stay `Unsupported`. Write support is 3.x behind `FileSystemMut`, and
`$LogFile` replay comes with it. NTFS stays in `unstable` until its read side
passes a conformance slice. See [Q5](#7-open-questions).

### 5.6 `hadris-part`

```rust
let disk = Disk::read(&mut dev)?;                  // block size from the device
match disk.table() {                               // #[non_exhaustive]
    PartitionTable::Mbr(mbr) => ..,
    PartitionTable::Gpt(gpt) => ..,
    PartitionTable::Hybrid(h) => ..,
    _ => ..,
}
for p in disk.partitions() {
    p.start(); p.len(); p.size_bytes();            // uses the disk's block size
    p.kind();                                      // PartitionKind::{Mbr(MbrType), Gpt(Guid)}
    p.name(); p.unique_guid();                     // GPT fields no longer dropped
    let fs_dev = disk.open(&mut dev, p)?;          // Slice<&mut D>
}
```

- `MbrType(u8)` with associated constants replaces `MbrPartitionType` and the 256-variant `MbrPartitionTypeFull`.
- GPT type GUIDs move to `gpt::types`. `Guid` implements `FromStr`; the inherent `from_str` becomes `Guid::parse_const`.
- `Gpt` fields are private. Edits go through methods that keep CRCs correct. Writing always computes CRCs.
- `HybridMbr` has private fields and `add_mirrored(..)`, identical with and without `alloc`.
- The six `*ReadExt`/`*WriteExt` traits and `PartitionInfoTrait` collapse into `Disk::read`, `Disk::write` and `raw` functions.
- Bool parameters (`bootable`) become `PartitionFlags`.

**Layout builder.** One flow for "disk image with partitions":

```rust
let layout = DiskLayout::gpt(disk_guid)
    .with_alignment(Alignment::MiB1)
    .partition(PartitionSpec::new(gpt::types::EFI_SYSTEM, Size::MiB(100)).with_name("EFI"))
    .partition(PartitionSpec::new(gpt::types::LINUX_FS, Size::Remaining))
    .build(dev.block_count(), dev.block_size())?;
let disk = layout.write(&mut dev)?;
let esp = disk.open(&mut dev, disk.partition(0))?;
hadris_fat::sync::format(esp, &FormatOptions::default())?;
```

**Feature work.** Extended and logical MBR partitions (EBR chains), falling
back to the backup GPT when the primary is corrupt, UTF-16 partition names,
remove and resize, overlap checks in every edit.

### 5.7 `hadris-cpio`

- The reader yields entries that borrow the reader and implement `Read`, so data has to be read or skipped. Dropping an entry skips the rest of its data.
- `skip_entry_data_owned`, `next_entry_alloc` and similar twins are removed.
- Sizes are `u64` at the API boundary, with `LimitExceeded` when a newc field overflows.
- An archive that ends at an aligned entry boundary without a trailer stays valid, as the Linux initramfs format allows (`LINUX-INITRAMFS-NEWC:archive#optional-trailer`). `ReaderOptions::with_strict_trailer()` turns that into `ErrorKind::Corrupt`, and a trailer cut off mid-entry is always `Corrupt`. The unused `Error::MissingTrailer` goes away. Concatenated archives (microcode plus main initramfs) are read with `reader.continue_after_trailer()`.
- The writer streams. `writer.append(path, &SetMetadata, Content)` per entry, `writer.finish()` returns the inner writer. `write_tree(&Tree)` is a helper on top.
- `CpioOptions` uses `Format::{Newc, NewcCrc, Odc}` instead of `crc(bool)`. Odc read and write are new; binary cpio is read-only.
- Hard links follow GNU cpio: data on the last link, metadata copied from the target.
- `RawNewcHeader::build` takes a `NewcFields` struct.
- `Error` variants always exist.
- The CLI supports `-` for stdin and stdout.

### 5.8 `hadris-block` and `hadris-optical`

- `detect` takes a `BlockDevice`.
- `OpenVolume` is `#[non_exhaustive]` and covers FAT, exFAT and NTFS. `OpenOpticalImage` covers ISO views and UDF. Both implement `FileSystem`, so "open whatever this is and list it" is one generic function.
- `hadris-optical` holds `hadris_iso::Error` instead of `hadris_io::Error` for ISO, and drops the remaining `expect` in `image_sync.rs`.
- `Error` exists in every feature combination.

---

## 6. Migration plan

Each step is one PR against a `v3` integration branch, so `main` keeps
shipping 2.x fixes. Bugs found during the survey are fixed on 2.x first, on `fix/survey-bugs`:
path traversal in the cpio and UDF CLI extract commands, and GPT CRCs of 0
without the `crc` feature. Locks held across `.await` cannot be fixed without
the V3 changes in 4.2 and 4.4 (the device lives inside the mutex and every
await is I/O on it), so 2.x documents async volumes as single-task.

1. **CI guardrails.** semver-checks against the branch point, the `non_exhaustive` lint, the all-features vs no-features API subset check, the sync/async parity check. Report-only at first.
2. **`hadris-io`.** embedded-io base, `StdIo`, `&mut T`, lossless `Error`, `ByteSource`, moved onto `strip_async!`. Update every crate to compile.
3. **`hadris-storage`.** `BlockDevice` in both modes from one source, `StreamDevice`, `MemDevice`, `Slice`, `Cache`, `ByteView`.
4. **`hadris-fs`.** Vocabulary, `ErrorKind`, `DateTime`/`Clock`, traits, path and handle layer, `Shared`/`Lock`, `FuseOnError`. Merge `hadris-path`. Slim `hadris-common` and merge `hadris-fixed` into it. Delete `hadris-archive`.
5. **`hadris-fat` as the reference implementation.** `BlockDevice` input, node table, `FileSystem`/`FileSystemMut`, `FormatOptions`, `check`, clock and code page generics. Port the conformance adapter to the generic `FileSystemMut` adapter in the same PR. This step tests the trait design, and the trait can still change here.
6. **Freeze the traits.** Review `hadris-fs` against FAT, the conformance adapter and a prototype FUSE adapter before any other format ports.
7. **Errors and the R1/R2/R4/R5 pass, crate by crate.**
8. **`hadris-part`.** `Disk`, `DiskLayout`, `MbrType`, GUIDs, CRC always on, EBR.
9. **`hadris-iso`.** Unified reader, `IsoView`, `IsoOptions`, `Tree` input, report, sessions, async writer.
10. **`hadris-udf`, `hadris-cd`, `hadris-cpio`.** Shared `Tree`, streaming readers and writers, UDF `FileSystem`.
11. **`hadris-ntfs`, `hadris-block`, `hadris-optical`, umbrella.**
12. **exFAT feature work and async parity** until exFAT passes the conformance suite.
13. **CLIs, examples, fuzz targets, docs.** Rewrite on the public API only. Anything the CLI verifier still needs goes in `raw`, which tests that `raw` is enough. `fs_dump` becomes one generic walk over `FileSystem`.
14. **3.0.0-rc.1.** CI guardrails become blocking. Migration guide (`docs/hadris-3.0.0-migration.md`) with a V2 to V3 symbol table.

`hadris-vfs` and the 3.x feature items follow 3.0.0.

---

## 7. Open questions

**Q1. Lock placement.** The proposal keeps locks out of format crates and
puts them in `Shared<F, L>` (4.4). The user wants flexible end usage and has
not settled this. The case for it: callers pick any lock, including async and
single-threaded ones, and the lock-across-await bug cannot happen inside a
format crate. The cost: operations on one volume serialize, as they do in V2.
If a kernel needs concurrent readers, the upgrade path is an `RwLock` mode in
`Shared` plus a device that supports concurrent positional reads. That needs
read methods on `&self`, which would be a trait change, so it has to be
decided before step 6.

**Q2. `Send` futures.** `async fn` in traits does not let a generic caller
require `Send` futures. Tokio users spawning tasks over a generic
`F: FileSystem` hit this. Options: ship a `Send` variant generated with
`trait_variant`, wait for return type notation, or document that the concrete
volume types (whose futures are `Send` when the device is) are what to spawn
over. Recommendation: generate both variants in the async module.

**Q3. Removing an open file.** POSIX semantics (entry gone, clusters freed at
the last `forget`) need orphan tracking, and a crash leaves lost clusters that
fsck has to reclaim. The alternative is `ErrorKind::Busy`. Recommendation:
POSIX semantics, since kernels expect them, with `check` reclaiming orphans.

**Q4. MSRV.** `core::error::Error` needs 1.81 and `async fn` in traits 1.75, so
the current 1.88 works. Raise it only if `dyn`-compatible async traits
stabilise before 3.0.

**Q5. exFAT and NTFS stability.** Promote exFAT to stable in 3.0 if it passes
the conformance suite by then, otherwise in 3.1. NTFS stays in `unstable`
through 3.0 either way.

**Q6. Name encoding in `Capabilities`.** FAT short names depend on the OEM code
page and long names are UTF-16. ISO primary names are d-characters, Joliet is
UCS-2. The trait takes bytes. Does `Capabilities` describe the charset
precisely enough for a VFS to translate names, or does each format also need
a `NameCodec`? Still open: `hadris-fs` ships `Capabilities::name_charset()`
returning a non-exhaustive `NameCharset` (`Bytes`, `Utf8`, `Ucs2`, `Utf16`,
`DCharacters`, `OemCodePage`) as the interim answer.
