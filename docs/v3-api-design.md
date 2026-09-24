# Hadris V3 API design

Status: draft for discussion. Sections 3, 4.1 to 4.4, 4.6, 4.12 and 4.13
were revised after the lock-placement prototype
(`experiments/lock-placement`, variant D, scenarios S1 to S16). Sections
4.3 to 4.6, 4.11 and R10 were revised after the step 6 trait review
([`v3-trait-review.md`](v3-trait-review.md)), which froze the `hadris-fs`
traits.

V3 is the release where the public shapes stop moving. V2 kept breaking semver
inside minor releases (#83, #93, #94) or hid new work behind `unstable-*` flags
that change the shape of public types (#111, #115). V3 fixes the extension
points first. After 3.0.0, new features land in 3.x minors because the types,
traits and error kinds they need already exist.

That gives a rule for scope. Every feature listed in [section 5](#5-per-crate-changes)
is V3 work. Every one of them must have its public shape in 3.0.0. The
implementation of a feature can follow in 3.x only if it fits a shape that
already shipped. NTFS write is the obvious example: the write methods of `FsDriver` exist in
3.0 with `ReadOnly` defaults, NTFS reports itself read-only through
`Capabilities`, and write support arrives later without a break.

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

- **Options** either are `#[non_exhaustive]` or have private fields (which already rule out struct literals, so they need no attribute), implement `Default`, and take consuming `with_*` setters. Presets are associated functions that return a configured value. They compose because they are starting points:

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

Previews use a separate module behind an `unstable-*` feature, for example
`hadris_block::ntfs` behind `unstable-ntfs`, never cfg-gated variants.
Anything outside such a module is covered by semver.

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
has a test for it. The adapter also ends after the inner iterator's first
`None` and implements `FusedIterator`. `Dir` fuses itself, since it also has an
async `next_entry` that no `Iterator` adapter covers.

### R8. Sizes and offsets are `u64`

File sizes, offsets and byte counts that can exceed 4 GiB are `u64` in every
crate, including 32-bit targets. On-disk narrowing uses checked conversion and
returns `ErrorKind::LimitExceeded`.

### R9. No bool parameters in public functions

Use an enum, a flags type or an options struct. Public functions with more
than four parameters take a struct.

### R10. Traits can grow

Public traits that users implement (`FsDriver`, `FileSystem`, `Resolver`,
`BlockDevice`, `ByteSource`, `Clock`, `LockKind`, `NodeTable`) are not
sealed. Methods added in 3.x must have a default body. For filesystem
operations the default returns `ErrorKind::Unsupported` (`ReadOnly` for
writes) or composes existing methods, and `Capabilities` gains a matching
flag that defaults to off. Traits that only Hadris implements are sealed.

Growing `FsDriver` and `FileSystem` has two more rules, both stated in the
trait docs:

- `impl_fs_driver!` forwards a new method only when a format names it in
  `also = [..]`. Adding it to the macro's write list would require every
  third-party format to have an inherent method of that name.
- A method is added to both traits and forwarded by every Hadris wrapper
  (`&mut F`, `Box<F>`, `&F`, `Arc<F>`, `Rc<F>`, `AsDriver`, `Volume`,
  `WithResolver`). `tests/sync_forwarding.rs` in `hadris-fs` reads the method
  lists from the trait definitions and fails until each wrapper forwards the
  new one. User types that wrap a driver keep the default until they forward
  it themselves; the docs say so.

### R11. CI enforces it

- `cargo semver-checks` on every PR against the latest 3.x release (after 3.0.0).
- The public-API snapshot runs with all non-`unstable` features on, and a second run with them off must produce a subset. That proves R3.
- A lint script rejects public enums without `#[non_exhaustive]` outside `raw`.
- A sync/async parity check diffs the public item lists of the two modules.

---

## 3. Layers

```
hadris-io        Read/Write/Seek with an associated error (ErrorType), ExactError,
                 FromEmbedded (embedded-io feature), StdIo, ByteSource
hadris-storage   BlockDevice, WriteError, MemDevice, StreamDevice, Slice, Cache,
                 std::fs::File as a device
hadris-fs        vocabulary, Error<E>, FsDriver and FileSystem, Volume<F, K>,
                 Resolver (Lexical, Posix), handles, Tree/Content for writers
format crates    native API with full fidelity; FsDriver through inherent methods
hadris-block,    detection and dispatch; OpenVolume / OpenOpticalImage
hadris-optical   implement FsDriver by delegation
hadris-vfs       object-safe DynFileSystem, host helpers, FUSE (std only)
hadris           umbrella with flat paths
```

Two ideas carry the design.

**One common trait, not one common API.** `FsDriver` covers what every
filesystem can express: look up a name, read a directory, read and write
bytes at an offset, change metadata. Generic code, the conformance suite,
`hadris-vfs` and kernel integrations target it, or its shared twin
`FileSystem`. Format crates keep a native API for what the trait does not
model: formatting, fsck, FAT attributes and cluster chains, ISO namespaces
and boot catalogs, NTFS streams. The trait impl is generated from the
inherent methods, so the trait never falls behind.

The trait is the wrong abstraction for three jobs, and V3 does not force it on
them:

- Build-once writers (ISO, UDF, CD, cpio, GPT) take a whole `Tree` and produce an image. They share `Tree`, `Content` and a report shape, not a trait. Generic code over "any image writer" has little use because the options differ per format.
- Streaming archives (cpio, later tar) are forward-only. They get an entry reader, not random access.
- Tools (fsck, analysis, the ISO verifier) use `raw` and native types.

**Every layer above the driver is opt-in.** A driver takes `&mut self`,
owns its device and holds no lock, no `Arc` and no allocation beyond its own
tables. Users who need several open files wrap it in `Volume<F, K>`, which
picks its lock by type. Users who need owned handles put that in an `Arc` or
`Rc`. Each layer adds only its own cost, and every layer can do every job
(4.3). [Section 4.4](#44-sharing-and-locking) explains the locking.

---

## 4. Cross-cutting design

### 4.1 `hadris-io`

**Traits.** `Read`, `Write` and `Seek` follow the embedded-io shape: each
reports its own error through a shared `ErrorType` supertrait. Sync and async
come from one source file, so they cannot drift.

```rust
pub trait ErrorType {
    type Error: core::error::Error + Send + Sync + 'static;
}

pub trait Read: ErrorType {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ExactError<Self::Error>> { .. }
}
pub trait Write: ErrorType { /* write, flush, write_all -> ExactError */ }
pub trait Seek: ErrorType { /* seek */ }

pub enum ExactError<E> { UnexpectedEof, WriteZero, Io(E) }
```

- The bound is the whole error contract. A kernel writes its own enum with `Display` and an empty `impl core::error::Error`. std devices use `std::io::Error`. embedded-io errors pass through unchanged. There is no Hadris error trait to implement.
- `Send + Sync` lets code erase any device error into `AnyError` or `std::io::Error` (4.6) with no extra where-clauses. It rules out errors that hold an `Rc` or a raw pointer; such a device wraps them.
- `&mut T` implements each trait when `T` does, and so does `Box<T>` with `alloc`.
- `FromEmbedded<T>` adapts an `embedded-io` or `embedded-io-async` stream. Its error is `T::Error`, unwrapped. It is the only item that names embedded-io, so the dependency sits behind an `embedded-io` feature.
- With `std`, `into_std_error(e)` converts any device error to `std::io::Error`, returning an `io::Error` as itself. `ExactError<E>` converts with `?`.
- With `std`, `StdIo<T>` adapts any `std::io` stream (a `Cursor`, a pipe) and reports `std::io::Error`. A host image file does not need it: `std::fs::File` is a `BlockDevice` directly (4.2), and file handles implement `std::io` directly (4.3).

A blanket `impl<T: embedded_io::Read> Read for T` was the first plan. It fails
on coherence. A `&mut T` impl overlaps it, and without that impl, generic code
holding `R: Read` cannot pass `&mut R` on, because `&mut R` is not an
`embedded_io::Read`. Explicit adapters cost one wrapper at the edge and remove
the whole class of problem. This addresses #16 and #18.

```rust
let file = std::fs::File::options().read(true).write(true).open("disk.img")?;
let fs = hadris_fat::sync::FatFs::open(file)?;           // no adapter, errors are io::Error
```

**Errors are the device's own.** The earlier draft erased every device error
into one `hadris_io::Error`. That lost the device error without `alloc`,
which is exactly the kernel case, and it made a std user unwrap a Hadris
error to get the `io::Error` back. With an associated error the device error
survives unchanged in every build, and 4.6 describes how code that mixes
devices erases it on purpose. Step 2 of the migration built the erased error;
step 4 replaces it.

**Byte sources.** One source type for every writer input replaces the two
`FileSource` copies:

```rust
pub trait ByteSource: ErrorType {
    fn len(&self) -> u64;
    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error>;
}
```

`ByteSource` is positional so writers can read a file twice (checksum pass,
data pass) without a seek contract. `&[u8]`, `Vec<u8>` and `&mut S`
implement it, and `SeekSource<T>` adapts any `Read + Seek`. `hadris-fs::Content`
wraps it (4.7).

### 4.2 `hadris-storage`

Filesystems read from a `BlockDevice`, not a byte stream. That fixes three V2
problems: the cache can sit under every format instead of inside FAT, 4Kn
devices and 2048-byte optical media stop being special cases, and a partition
is just another device.

```rust
pub trait BlockDevice: ErrorType {
    fn block_size(&self) -> BlockSize;
    fn block_count(&self) -> u64;
    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error>;
    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8])
        -> Result<(), WriteError<Self::Error>> { Err(WriteError::ReadOnly) }
    async fn flush(&mut self) -> Result<(), WriteError<Self::Error>> { Ok(()) }
}

#[non_exhaustive]
pub enum WriteError<E> { ReadOnly, Device(E) }
```

`flush` returns `WriteError` too, because a write-back device writes there.

`async fn` here means "written once, generated for both modes" (4.8). A
read-only device implements three methods and no write method.

**No `writable()` query.** A static flag is wrong too often: an SD card's lock
switch moves while mounted, `std::fs::File` cannot tell whether it was opened
for writing, and a USB stick can be write-protected at any time. Probing with
a test write is worse: it writes, it wears flash, and write-once media keep
it. Instead each write answers for itself:

- A device that refuses writes returns `WriteError::ReadOnly`. `Error<E>` converts it to `ErrorKind::ReadOnly` with no device error attached.
- The driver remembers the first refusal and from then on rejects writes before touching the device or its own tables (`is_read_only()`).
- Users who know up front mount read-only (`FatFs::open_with(dev, MountOptions::new().with_read_only())`), which never calls `write_blocks`. That also covers media where even an attempted write is unwelcome.
- Format crates must leave their in-memory state unchanged when a write is refused. That is the rule 4.3 already sets for every failed operation, so it adds no new contract.

Provided devices and adapters:

| Type | Purpose |
|---|---|
| `impl BlockDevice for &mut D`, `Box<D>` | Borrow or box a device instead of moving it in. |
| `std::fs::File` | With `std`, a host image file is a device with 512-byte blocks and `std::io::Error`. A file opened read-only fails writes with the OS error, which reaches the caller unchanged. |
| `StreamDevice<T>` | Any `Read + Seek + Write` byte stream, with a block size the caller picks. Error `StorageError<T::Error>`. `StreamDevice<ReadOnly<T>>` needs only `Read + Seek` and answers writes with `WriteError::ReadOnly`. The migration path for every V2 user. |
| `MemDevice<B>` | `&[u8]` (answers writes with `ReadOnly`), `&mut [u8]`, `[u8; N]`, and `Vec<u8>` or `Box<[u8]>` with `alloc`, through the `MemBuffer` trait. Error `OutOfRange`. For tests and in-memory images. |
| `Slice<D>` | A block range of `D`. `D` can be owned or `&mut`. Error `StorageError<D::Error>`: requests past the end of the slice never reach `D`. Replaces `PartitionView`. |
| `Cache<D>` | Write-back LRU over whole blocks, `alloc` only. Error `D::Error`. Explicit `flush`, or `finish` to flush and return the device. Capacity set at construction. Works in both modes. Kernels skip it. The first write goes straight to the device, so a read-only device refuses at once instead of at a later flush; out-of-range requests bypass the cache and fail with the device's error. Requests of at least `capacity` blocks also bypass it (reads still see dirty cached blocks), and a flush writes each run of consecutive dirty blocks in one call. |
| `ByteView<D>` | Byte-granular `read_at`/`write_at` with read-modify-write for partial blocks, and a bounded `Read + Write + Seek` stream. Error `StorageError<D::Error>`. Used by format crates for records that straddle blocks. Without `alloc` its scratch buffer caps the block size at 4096. |

Adapters that can refuse a request themselves report
`#[non_exhaustive] StorageError<E> { OutOfRange, UnexpectedEof, WriteZero,
ReadOnly, BlockTooLarge, Device(E) }`. Adapters that only pass requests on
(`&mut D`, `Box<D>`, `Cache<D>`) keep `D::Error`. Stacking two refusing
adapters nests the type (`StorageError<StorageError<E>>`), and converting that
to `std::io::Error` keeps only the outer level unchanged; the common stacks
(`Slice<File>`, `Cache<Slice<File>>`) have one level.

`StreamDevice` picks its write support through a small `StreamWrite` trait,
implemented for every `Write` and for `ReadOnly<T>`, whose write returns
`WriteError::ReadOnly`. A second `impl BlockDevice` for read-only streams
would overlap the first, so the marker type carries the choice.

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
| `NodeId` | Opaque `u64`, never 0. Stable for as long as the node is pinned (4.5). Maps directly to FUSE `ino` and kernel inode numbers; the root may have any value, and a FUSE layer maps it to 1. |
| `FileType` | `File`, `Dir`, `Symlink`, `CharDevice`, `BlockDevice`, `Fifo`, `Socket`. Non-exhaustive. One definition for every crate. |
| `Name` / `NameBuf<N>` | Names are bytes. `Name::to_str()` returns `Result`. `NameBuf` is a fixed-capacity buffer so no-alloc callers can read directories. The default capacity is 1024 bytes, enough for a 255-unit UTF-16 long name in UTF-8, so `dyn`-friendly signatures that name the default `NameBuf` work for every format. |
| `NodeTable` | Per-node driver state with pin counts (4.5). `FixedTable<N>` (no alloc), `HeapTable` (`alloc`), or a user type. |
| `Metadata` | `file_type`, `len: u64`, `times: FileTimes`, `permissions: Option<Mode>`, `owner: Option<(u32, u32)>`, `nlink` (1 on a directory means not counted), plus `attributes: Attributes` (DOS-style flags). Getters only, `Copy`. Per-format metadata (FAT attribute bits, NTFS security descriptors) stays native, reached through `vol.lock()`; there is no `extra()` hook, since one holding owned data could not be `Copy`. Plain fields can be added in 3.x (section 4.14). |
| `DateTime` | Private fields: seconds since 1970, nanoseconds, optional UTC offset in minutes. Every format converts to and from its own encoding in `raw`. |
| `FileTimes` | `created`, `modified`, `accessed`, `changed`, each `Option<DateTime>`, with `with_*` setters. Used for both reads and `set_metadata`. |
| `Clock` | `fn now(&self) -> DateTime`. `NoClock` (fixed documented epoch) is the default, `SystemClock` with `std`. |
| `Capabilities` | Flags and limits: writable, symlinks, hard links, case sensitivity (`Sensitive`, `InsensitivePreserving`, `Insensitive`), max name length, name charset, supports permissions, supports owners, timestamp resolution. |
| `FsStats` | Total, free and used blocks, block size, file count if known. |
| `ErrorKind` | The shared kind set (4.6). |
| `DirCursor` | A `Copy` position in a listing. Raw value 0 is the start; drivers return raw values up to `DirCursor::MAX_RAW` (`2^63 - 16`), so a cursor fits `off_t` with room for the `.` and `..` a FUSE layer adds. |
| `RemoveKind` | `File`, `Dir`, `Any`: what `remove` expects to find. Non-exhaustive. |
| `VPath` | The `hadris-path` type, moved here. Path parsing is convenience only; the traits take names. |

**The driver traits.** One node API, in two receivers:

```rust
pub trait FsDriver {                               // format crates; &mut self, no locks
    type DeviceError: core::error::Error + Send + Sync + 'static;

    fn capabilities(&self) -> Capabilities;
    fn root(&self) -> NodeId;
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError>;
    async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError>;
    async fn read_dir_entry(&mut self, dir: NodeId, cursor: &mut DirCursor, name: &mut NameBuf)
        -> FsResult<Option<DirEntry>, Self::DeviceError>;
    async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, ..>;
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, ..> { unsupported }
    async fn read_link(&mut self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, ..> { unsupported }
    async fn stats(&mut self) -> FsResult<FsStats, ..>;
    fn forget(&mut self, node: NodeId);
    async fn open_node(&mut self, node: NodeId) -> FsResult<(), ..> { Ok(()) }
    fn close_node(&mut self, node: NodeId) {}
    async fn resolve(&mut self, path: &str) -> FsResult<NodeId, ..> { Lexical.resolve(self, path) }

    // Write methods default to ErrorKind::ReadOnly.
    async fn create(&mut self, dir: NodeId, name: &Name, kind: NewNode<'_>, meta: &SetMetadata)
        -> FsResult<NodeId, ..>;                            // File | Dir | Symlink(target) | Device(..)
    async fn remove(&mut self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), ..>;
    async fn rename(&mut self, from_dir: NodeId, from: &Name, to_dir: NodeId, to: &Name,
        flags: RenameFlags) -> FsResult<(), ..>;            // NoReplace | Exchange later via R10
    async fn write_at(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, ..>;
    async fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), ..>;
    async fn set_metadata(&mut self, node: NodeId, changes: &SetMetadata) -> FsResult<(), ..>;
    async fn sync_node(&mut self, node: NodeId) -> FsResult<(), ..>;     // durable, like fsync
    async fn publish_node(&mut self, node: NodeId) -> FsResult<(), ..> { self.sync_node(node) }
    async fn sync(&mut self) -> FsResult<(), ..>;
}

pub trait FileSystem { /* the same methods on &self */ }   // shared users; Volume implements it
impl<F: FileSystem + ?Sized> FsDriver for &F { .. }         // so the path layer is written once
```

- A format crate writes each method once, as an inherent method on its driver, and `impl_fs_driver!` generates the `FsDriver` impl from them. Raw users call the inherent methods with no trait import. A `read_only` form of the macro leaves the write methods at their defaults. Optional methods (`parent`, `read_link`, `resolve`, `open_node`, `close_node`, `publish_node`, and every method added after 3.0) are forwarded only when named, `impl_fs_driver!(sync, impl[D: BlockDevice] IsoFs<D>, error = D::Error, read_only; also = [parent, read_link])`. The first argument is the mode (`sync`, `async`, `async_send`), which a format crate's per-mode module passes, so a driver never forwards a method it does not have. The exported macro uses `$crate::` paths.
- `FileSystem` is what shared code programs against. `Volume` (4.4) implements it for every driver, and a format that wants finer locking can implement it directly. Because `&F` is a driver whenever `F` is a `FileSystem`, every helper below is written once over `FsDriver` and serves both.
- `node_metadata` and `read_dir_entry` carry the `node_`/`_entry` in their names so they never clash with the path helpers `metadata(path)` and `read_dir(path)`; both traits can be in scope at once.
- `lookup` pins the node it returns. `forget` unpins it. This is the FUSE `lookup`/`forget` contract. A pin never blocks a removal: a pinned node that is removed keeps its id until its last `forget`, and every other method answers `NotFound` for it. `open_node` marks a pinned node as open and `close_node` ends that; `remove` and a replacing `rename` fail with `Busy` only for the last name of an open node (Q3). `File` and `OpenFile` open their node. The split exists because a FUSE kernel holds a lookup on every cached name: with pins blocking removal, every `rm` failed (review F1). `close_node`, like `forget`, never fails or blocks. ISO, UDF and NTFS have naturally stable IDs, so `forget` does nothing for them. Without a node table, a directory's id is the location of its own `.` record, not of the record in its parent: `parent` is then one read, and a relocated or symlinked directory has one id (the ISO driver in experiment E3). The names of an ISO Rock Ridge hard link share the id of the first record in path table order with the same `PX` serial number (or data extent), found by a scan when a file with more than one link is listed; UDF hard links share their ICB. Rejecting a forged id with `InvalidHandle` is best effort for such formats: the driver checks what it can (ISO compares the both-endian fields), and a forged id that still decodes reads garbage but is never undefined behaviour.
- `read_dir_entry` is a resumable cursor. `DirCursor` is a `Copy` value that the caller can store and reuse, which FUSE `readdir(offset)` and kernel `getdents` need. It returns one entry per call into a caller buffer, so it works without `alloc`. `DirEntry` carries the name length, `FileType`, and the entry's `NodeId`. Entries are not pinned; a caller that wants to keep one calls `lookup`. Until the directory changes, and unless a node is forgotten in between, a listed id is the id a `lookup` of that name returns, so `ls -i` and `stat` agree.
- Everything is positional. There is no cursor inside a file node.
- `.` and `..` never appear in `read_dir_entry` output and `lookup` rejects them. Paths resolve through a `Resolver` (4.12): the default never asks the driver about `..`, and `Posix` calls `parent`.
- `rename` keeps the `NodeId` of the moved node. That is the point of the open-node table (4.5).
- A failed operation leaves the volume unchanged, in memory and on disk. The conformance suite already tests this ("rejection" scenarios), and the trait docs make it part of the contract.
- `sync` writes every piece of cached metadata (FSInfo, dirty FAT sectors, directory entries) and flushes the device. There is one durability call per scope, not V2's `sync` plus `flush`: `sync_node` makes one node durable (`fsync`), `sync` the volume. `publish_node` writes a node's pending metadata without flushing the device, which is what closing a file needs; its default calls `sync_node`.
- `remove(dir, name, kind)` checks the node's type against `RemoveKind`, so `unlink` and `rmdir` need no lookup first.
- Reads never change times. `node_metadata` shows a pending size at once; other fields a driver keeps in memory may lag until `publish_node`, `sync_node` or `sync`. In the async modes a call whose future is dropped before it completes leaves no pin.
- The `contract` feature adds `contract::check(&mut fs)` in each mode: the format-independent rules of this contract as a test kit, which every format port runs. The test driver and `FatFs` pass it.

The earlier draft split reads and writes into `FileSystem` and
`FileSystemMut`. The prototype kept one trait. A compile-time split doubles
the traits, the forwarding impls and the bounds on every helper, and it still
cannot describe the common case: a driver that becomes read-only at runtime
when its device refuses a write (4.2). `Capabilities::writable` and
`ErrorKind::ReadOnly` cover it.

**Volume-specific operations stay native.** Labels, formatting, fsck, FAT
attribute bits beyond `Attributes`, cluster chains, ISO namespaces and NTFS
streams are inherent methods on the format types. On a shared volume they are
reached through `vol.lock()`.

**Tiers.** Every tier can do every job; the tiers differ in how much the user
wraps, not in what they can reach. The test S15 runs the same operations on
all three.

| Tier | Build it with | Costs | Paths and handles |
|---|---|---|---|
| Raw | `FatFs::open(dev)?` | Nothing | Node API as inherent methods. `DriverExt` adds path helpers on `&mut self`. `File<&mut FatFs<_>>` is one open file borrowing the driver. `OpenFile` is a `Copy` cursor, so a kernel file table holds many beside one `&mut` driver. |
| Shared | `Volume::new(fs)`, `Volume::spin(fs)`, `Volume::local(fs)` | One lock | `PathExt` on `&self`, any number of `File<&Volume<..>>`. |
| Owned | `Arc::new(Volume::new(fs))` or `Rc` | One allocation | The shared API unchanged; `File<Arc<Volume<..>>>` can move to another thread or task. |

**One canonical pattern per tier.** Each tier's module docs open with one
pattern, and the rustdoc examples use only that pattern. Alternatives exist,
but they sit behind an extension trait that the user imports, so they do not
show up until asked for.

```rust
// Raw tier: a kernel VFS or a format tool. Node ids, no paths, no lock.
let mut fs = FatFs::open(dev)?;
let boot = fs.lookup(fs.root(), Name::new("boot")?)?;
let n = fs.read_at(boot, 0, &mut buf)?;
fs.forget(boot);

// Shared tier: applications, embedded, scripts. Paths and std-like handles.
let vol = Volume::new(FatFs::open(file)?);         // Volume::spin / Volume::local on no_std
let mut log = vol.open("/log.txt", OpenOptions::write().create().append())?;
writeln!(log, "hello")?;
log.close()?;                                      // returns errors; Drop is best effort
for entry in vol.read_dir("/EFI")? { let entry = entry?; }       // fuses on error

// Owned tier: handles that outlive a borrow.
let vol = Arc::new(Volume::new(FatFs::open(file)?));
let file = File::open(Arc::clone(&vol), "/big.bin", OpenOptions::read())?;
std::thread::spawn(move || read_all(file));
```

| Job | Raw | Shared or owned |
|---|---|---|
| Open by path | `DriverExt::open`, or `OpenFile::open` for a file table | `PathExt::open`, `File::open` for `Arc` |
| Several files at once | `OpenFile` values beside `&mut fs` | Any number of `File`s |
| `std::io` on a file | `File<&mut FatFs<_>>` | `File<&Volume<..>>`, `File<Arc<..>>` |
| List a directory | `DriverExt::read_dir`, or `read_dir_entry` with a cursor | `PathExt::read_dir` |
| Format-specific calls | Inherent methods | `vol.lock()` |
| Other calls while a file is open | `file.driver()` | `vol` directly |
| Path semantics | `fs.with_resolver(Posix::new())` or `Posix::new().resolve(&mut fs, path)` | `Volume::new(fs.with_resolver(Posix::new()))` |
| Get the device back | `fs.into_inner()` | `vol.into_inner().into_inner()`, or `Volume::local(&mut fs)` for a scope |

**Handles.** One `File<A>` and one `Dir<A>` serve all tiers. `A` is how the
handle reaches the filesystem, through a small `Access` trait implemented for
`&mut D`, `&F`, `Rc<F>`, `Arc<F>` and `Volume` by value. Users name `Access`
only in generic code over handles.

```rust
pub trait Access {
    type DeviceError: ..;
    type Driver: FsDriver<DeviceError = Self::DeviceError>;
    fn into_driver(self) -> Self::Driver;
}
// &mut D -> &mut D, &F -> &F, Arc<F> / Rc<F> / Volume -> AsDriver<Self>
pub struct AsDriver<F>(F);      // FsDriver for a FileSystem held by value; Deref<Target = F>
```

The handle stores `A::Driver`, and `file.driver()` returns `&mut A::Driver`.
The prototype's first `Access` had a lifetime GAT (`type Driver<'a>`), and a
handle holding that projection across an `.await` could not be spawned, even
for a concrete `File<&Volume<..>>` (E2: "implementation of `Access` is not
general enough"). Implementing `FsDriver` directly on `Arc<F>` and `Volume`
instead makes `vol.root()` ambiguous, since both traits have it, hence
`AsDriver`.

- `File` implements `hadris_io::{Read, Write, Seek}` and, with `std`, the `std::io` traits for any device error (4.6). `Dir` is an `Iterator` in sync builds and has `next_entry` in async builds.
- `OpenOptions { read, write, append, truncate, create, create_new }` makes overwrite semantics explicit (#90, #91). Opening for writing fails with `ReadOnly` at open time when `capabilities().writable()` is false, before any truncation, instead of at the first write.
- Opening a symlink node as a file (possible with `Lexical`, which never follows links) fails with `ErrorKind::Symlink`, as POSIX `O_NOFOLLOW` fails with `ELOOP`.
- `close()` returns `Result`. It publishes the node's metadata; `sync_all()` makes the file durable first. `Drop` does a best-effort `close_node` and `forget` and never flushes the device. In the blocking API it also publishes a written file's metadata, ignoring errors, so a dropped log file keeps its size across a power cut; the async APIs cannot await in `Drop`, so there the metadata stays pending in the driver until the next `publish_node`, `sync_node` or `sync`. `#[must_use]` on handles. `OpenFile` is plain data and does not close or forget on drop; its owner calls `close`.
- Helpers on both `DriverExt` and `PathExt`: `exists`, `metadata`, `open`, `read_dir`, `read_to_vec`, `write_file`, `create_dir_all`, `remove_file`, `remove_dir`, `remove_dir_all`, `rename_path`. `rename_path` is not `rename` because the node method of that name is on the driver traits and both can be in scope. `remove_dir_all` needs no allocation and fails with `LimitExceeded` below 64 levels. Free functions: `copy_tree` (between any two filesystems, with `alloc`), and with `std` in the sync API, `extract_to_host` and `import_from_host`. Each side is any `Access`, so `&mut fs`, `&vol` and an `Arc` all work. The host helpers reject absolute names, separators, drive prefixes and `..` components, and never write through an existing host symlink, so archives and images cannot escape the target directory.

```rust
pub async fn copy_tree<S: Access, T: Access>(src: S, from: &str, dst: T, to: &str) -> Result<(), AnyError>;
pub fn extract_to_host<S: Access>(src: S, from: &str, host: impl AsRef<Path>) -> std::io::Result<()>;
pub fn import_from_host<T: Access>(host: impl AsRef<Path>, dst: T, to: &str) -> std::io::Result<()>;
```

`copy_tree` needs `alloc`, unlike `remove_dir_all`: two devices mean two
error types, and `AnyError` is the only common one. With `alloc` required
anyway, it walks with a `Vec` instead of a fixed stack, so it has no depth
limit. It merges into existing directories, overwrites files, and fails with
`AlreadyExists` on a type clash or an existing symlink. The host helpers exist
only in the sync API, because the host side is blocking `std::fs`, and an
async version would block the executor, the problem the survey found in the
V2 `FileSource`. The sync/async parity check (R11) reports them as sync-only,
as it reports the mode-specific locks.

The conformance suite's FAT adapter becomes one generic impl over
`FileSystem`. The rust-fatfs and mtools peers can keep their own adapters,
or implement the trait themselves.

### 4.4 Sharing and locking

Drivers take `&mut self` and hold no lock. Sharing is a wrapper the user
chooses:

```rust
pub struct Volume<F, K: LockKind> { .. }
impl<F: FsDriver, K: LockKind> FileSystem for Volume<F, K> { .. }

Volume::new(fs)               // sync: std mutex (std); async: async mutex
Volume::spin(fs)              // spin mutex, no_std
Volume::local(fs)             // one thread: a RefCell flag, no_std, no alloc
Volume::with_lock::<K>(fs)    // any LockKind: critical-section, embassy-sync, tokio
```

- There is one constructor per lock and no default `K`. A default that depended on the `std` feature would change the type of every `Volume` in the build when any crate enabled `std`, which R3 forbids. Constructors infer `K`, so it appears only in stored types, and there the user writes one alias: `type Disk = Volume<FatFs<File>, StdMutex>;`. Hadris ships no aliases, because each would be a second spelling of the same type.
- The lock is held for one driver call. Path resolution runs under one lock hold, so a path costs one lock, not one per component.
- `forget` and `close_node` never block, so handle `Drop` works in both modes. If the lock is taken, the call goes on a queue that the next lock drains, publishes before closes before forgets. A blocking `publish_node` for a node whose close is queued joins the queue instead of waiting, so a written `File` can be dropped under `vol.lock()`. With `alloc` the queue grows; without it, it holds 16 distinct nodes and a 17th panics rather than waiting for a lock this thread may hold.
- `vol.lock()` returns a guard to the driver for format-specific calls. Calling a `FileSystem` method on the same volume while holding the guard deadlocks, or panics with `Local`.
- `Volume::local(&mut fs)` borrows a driver for one scope: a kernel keeps ownership, opens several handles, and gets the driver back afterwards.
- `Arc<Volume>` and `Rc<Volume>` give owned handles that move between tasks or live in a kernel file table.

`LockKind` is generated for both modes:

| Mode | Lock kinds |
|---|---|
| sync | `StdMutex` with `std`, `Spin` (on targets with atomic compare-and-swap), `Local` (no alloc, not `Sync`), and any `lock_api::RawMutex`. |
| async | `AsyncMutex` (async-lock) and impls behind features for `embassy-sync`. Users can add tokio's mutex in a few lines. |

Operations on one volume serialize, as they do in V2. A format that needs
concurrent readers implements `FileSystem` itself with finer locks around its
node table and cache; `FileSystem` takes `&self`, so callers do not change.

The alternative was `&self` methods with a lock inside every format crate.
The prototype built it (variant B). Each crate has to pick a lock, async
builds need an async lock inside the format crate, the V2 bug class (guard
held across `.await`) stays possible, and a kernel that wants no lock pays for
one anyway. With the lock outside, format crates never see a lock. See
[Q1](#7-open-questions).

### 4.5 Node identity: the open-node table

FAT and exFAT have no inodes. The natural identity of a file is the location
of its directory entry, and that changes on rename. V2's `FileEntry` snapshot
model produced `StaleEntry`, #90 and #24.

V3 keeps an open-node table inside `FatFs` (and `ExFatFs`):

- The table is a public `hadris_fs::NodeTable`, shared by FAT, exFAT and any format that caches node state (ISO), and a type parameter of the driver: `FatFs<D, T: NodeTable = FixedTable<64>>`. `FixedTable<N>` is a fixed array and needs no allocator, `HeapTable` grows with `alloc`, and users can supply their own, such as an evicting table. The user names the kind of table; the driver stores `T::With<State>` for its private state type.
- The entry records the directory-entry location, the first cluster, the size, the pin count and whether a writer holds it. The `NodeId` comes from the entry's location when the node is first pinned and stays with the node.
- `lookup` finds or creates the table entry and increments the pin count. `forget` decrements it, and the entry may be freed at zero. A full table gives `ErrorKind::LimitExceeded` (`TableFull`). A driver that must keep state past the last pin (unwritten metadata) holds a pin of its own.
- `rename` updates the location in place, so the `NodeId` stays valid. `set_len` and `write_at` update the size in the table, so every handle sees the same size and the directory entry is written on `sync_node` or `sync`.
- `remove` of a pinned node succeeds; the table entry stays, marked unlinked, until the last `forget`, and answers `NotFound`. The entry also counts opens, and removing an open node fails with `ErrorKind::Busy`. See [Q3](#7-open-questions).
- A FAT id is the slot of the node's short entry plus a tier in the bits above bit 40. The tier counts up only while a pinned node that has moved away holds the lower tiers of that slot, so a listing and a lookup compute the same id.
- IDs of unpinned entries from `read_dir` are valid until the next mutation of that directory. The docs state this, and `lookup` is the way to keep one.

ISO (directory record location), UDF (ICB location and partition) and NTFS
(MFT reference with sequence number) have stable IDs that fit in a `u64`, so
they need no table.

### 4.6 Errors

Every filesystem operation in every crate returns one type, generic over the
device's error:

```rust
pub struct Error<E> {
    kind: ErrorKind,
    context: Context,          // private: sector, cluster, node, field name
    device: Option<E>,
}

impl<E> Error<E> {
    pub fn kind(&self) -> ErrorKind;
    pub fn device_error(&self) -> Option<&E>;
    pub fn into_device_error(self) -> Option<E>;
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> Error<F>;
}

pub type FsResult<T, E> = Result<T, Error<E>>;

pub struct AnyError { .. }     // alloc: kind plus the boxed device error
impl<E: ..> From<Error<E>> for AnyError { .. }
impl<E: ..> From<Error<E>> for std::io::Error { .. }       // std

pub struct MountError<D, E> { .. }  // a failed mount: the Error<E> and the device given back
impl<D, E> From<MountError<D, E>> for Error<E> { .. }      // also into AnyError and std::io::Error

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Io,                  // the device failed; device_error() is Some
    NotFound,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    NoSpace,
    ReadOnly,            // a read-only mount, or the device answered WriteError::ReadOnly
    InvalidInput,        // bad options, names or arguments from the caller
    Corrupt,             // disk data violates the specification
    Unsupported,         // valid but not implemented, or not in Capabilities
    LimitExceeded,       // a value does not fit a field or buffer, a path is too long, a table is full
    Symlink,             // too many symlinks, or a symlink where a file or directory was needed (ELOOP)
    InvalidHandle,       // unknown or forgotten NodeId
    Busy,                // removing the last name of an open node
    NameTooLong,         // ENAMETOOLONG
    FileTooLarge,        // EFBIG: past the format's file size limit
}
```

- **Kernels** keep their own error without `alloc`. The errno mapping is one `match (err.kind(), err.device_error())` (S12). Filesystem failures have no device error.
- **std users** use `?` into `std::io::Error` or `Box<dyn Error>`. A device error that is already an `io::Error` comes back as itself, found by a downcast through `Any` that does not allocate, so `raw_os_error()` survives (S11). Any other device error becomes the source of an `io::Error` with kind `Other` and can be downcast back out. Filesystem failures map their kind (`NotFound` to `NotFound`, `ReadOnly` to `ReadOnlyFilesystem`, and so on).
- **Code that mixes devices** returns `AnyError`, or `io::Error` with `std`. Both accept `?` from any `Error<E>`, so `copy_tree` from a `MemDevice` into an embedded card is one function with no extra bounds (S4).
- **Drivers that take the device by value** fail to mount with `MountError<D, E>`, which gives the device back (`into_device`, `into_parts`). `?` still converts it to `Error<E>`, `AnyError` or `io::Error`, dropping the device.
- **Generic code** names the device error through the trait: `fn install<F: FileSystem>(vol: &F) -> FsResult<(), F::DeviceError>`, or returns `AnyError`.
- Writers return `Error<W::Error>` for their output stream.
- Each kind maps to one errno. `NameTooLong` and `FileTooLarge` were split from `LimitExceeded` in the step 6 review, because one kind covered three errnos.
- `Error<E>` equality compares the kind and the device error only; context added later never takes part, so tests that compare errors keep passing.
- Callers match on `err.kind()`. New failure modes add context, not kinds. Crate-specific detail comes through typed accessors (`err.sector()`, `err.cluster()`) and a `#[non_exhaustive] enum Detail` where matching is useful.
- No `String` payloads without `alloc`. No foreign types (`bytemuck::PodCastError`, `PathBuf`) in the public API.
- Wrapper crates (`hadris-cd`, `hadris-optical`, `hadris-block`) keep the kind and the device error and add context.
- Module-level `Error`/`Result` aliases (`write::Error`, `modify::Error`) are removed.

The earlier draft rejected `Error<E>` for putting a type parameter on every
signature and making errors from two devices different types. In practice the
parameter hides behind `FsResult<T, F::DeviceError>` and the `?` conversions
above, and the erased form it replaced lost the kernel's error entirely.

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
- `TreeExt::from_filesystem(src)` builds a tree from any mounted volume (any `Access`), which gives FAT-to-ISO conversion and image round-trips for free.

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
- Traits in the async module use `async fn`. See [Q2](#7-open-questions) for the `Send` question: the proposal is a third generated mode, `async_send`, whose futures are `Send`.

### 4.9 Feature flags

Revised 2026-09-24 (4.15): FAT and exFAT without `alloc` move to the embedded
API, and the `embassy-sync` feature and `Local` lock kinds are removed.

| Feature | Meaning |
|---|---|
| `alloc` | Heap-backed conveniences: owned names, trees, boxed sources, `AnyError`, `read_to_vec`, `Box`/`Rc`/`Arc` forwarding and `Access` impls. |
| `std` | Implies `alloc`. `std::fs::File` as a `BlockDevice`, `std::io` impls on handles, `From<Error<E>> for std::io::Error`, `StdMutex` and `Volume::new`, `StdIo`, `Content::path`, `SystemClock`, host helpers. |
| `embedded-io` | `FromEmbedded<T>` for embedded-io and embedded-io-async streams. Nothing else names embedded-io. |
| `sync` | Sync API. On by default. |
| `async` | Async API. |
| `write` | Writers, formatters, modifiers. Enables every correctness dependency (CRC and so on). |
| `unstable-*` | Enables a preview module, such as `hadris_fat::exfat`. Never changes stable items. |

- `read` is dropped; reading is always available.
- `crc` and `rand` stop being user-facing features. CRC is always compiled with `write`. Random GUIDs come from the caller (4.10).
- `cache` stops being a feature. `Cache<D>` is always available and costs nothing unless constructed.
- No feature changes behaviour. Path semantics (4.12) and lock choice (4.4) are types, so enabling a feature anywhere in the build only adds items.
- The umbrella forwards the same axes plus one feature per format.

FAT reads and writes without `alloc`: E1 replaced the node table with a fixed
array and the chain extension's `Vec` with two passes over the FAT. The
table's capacity is [Q8](#7-open-questions).

For `FatFs`, `write` gates only `format`; the write methods of `FsDriver`
are always compiled, so the feature adds items and never changes what an
existing call does. `write` implies neither `read` nor `alloc`, so the no-alloc CI tiers build
`format`. The V2 `lfn` and `dirty-file-panic` features switched behaviour
and were removed with the V2 driver: `FatFs` always reads and writes long
names, and it has no writer that can be dropped unfinished.

### 4.10 Partition GUIDs and randomness

No hidden RNG. `Gpt::new(disk_guid: Guid, ..)` and `GptEntry::new(unique_guid:
Guid, ..)` take GUIDs. `Guid::random()` exists with `std`. Formatters that need
a volume serial take one in options, defaulting to one derived from the clock.

### 4.11 Crate layout

| V2 crate | V3 |
|---|---|
| `hadris-io` | Kept. `ErrorType`, `Read`/`Write`/`Seek`, `ExactError`, `FromEmbedded`, `StdIo`, `ByteSource`. |
| `hadris-storage` | Kept and adopted by every filesystem. `BlockDevice`, `WriteError`, adapters, `Slice`, `Cache`. |
| `hadris-fs` | New. Vocabulary, `Error<E>`, `FsDriver`/`FileSystem`, `impl_fs_driver!`, `Volume`/`LockKind`, resolvers, handles, `Tree`/`Content`, `FuseOnError`, `VPath`. |
| `hadris-common` | Internal. Endianness and fixed-size string types, merged with `hadris-fixed` into one set. Documented as not for direct use. |
| `hadris-fixed` | Merged into `hadris-common`. |
| `hadris-path` | Merged into `hadris-fs`. |
| `hadris-macros` | Kept, internal. |
| `hadris-archive` | Removed. The umbrella re-exports `hadris-cpio` directly. |
| `hadris-block`, `hadris-optical` | Detection and dispatch. `OpenVolume` and `OpenOpticalImage` implement `FsDriver` by delegation. |
| `hadris-vfs` | New, `std` only, can ship in 3.x. Type erasure, a mount table for composing volumes, and a `fuser` adapter (prototyped in `experiments/fuse-prototype`, on `FileSystem` and `HeapTable`). The sync `FileSystem` is already dyn-compatible once its error is fixed, so the sync form needs no second trait: an `Erased<F>` wrapper implements `FileSystem<DeviceError = AnyError>` for any `F`, and one `Vec<Box<dyn FileSystem<DeviceError = AnyError> + Send + Sync>>` holds FAT and ISO volumes with different device errors; path helpers, handles and resolvers work on it unchanged. E3's `DynFileSystem`, which repeated every method, would have been a second list to grow under R10. The async forms still need an object-safe trait with boxed futures, since `async fn` is not dyn-compatible. Lost: the typed device error (boxed, error path only), format-specific methods and `lock()`. |
| Format crates | Kept. |
| `hadris` | Re-exports `io`, `storage` and `fs` so no_std users need one dependency. Flat paths: `hadris::fat`, `hadris::iso`. |

### 4.12 Path resolution

Path semantics are a type, `Resolver`, never a cargo feature. Features unify
across the build graph: if `alloc` or a `posix` feature switched how `..`
works, one dependency turning it on would change what paths mean for every
other crate in the build. A type only affects the code that names it, so two
crates, or two volumes in one program, can resolve differently (S14).

```rust
pub trait Resolver {
    async fn resolve<D: FsDriver + ?Sized>(&self, fs: &mut D, path: &str)
        -> FsResult<NodeId, D::DeviceError>;
}

pub struct Lexical;                           // the default everywhere
pub struct Posix<const N: usize = 1024>;      // POSIX, with an N-byte symlink buffer
pub struct WithResolver<D, R> { .. }          // a driver whose paths use R
```

| | `Lexical` (default) | `Posix<N>` |
|---|---|---|
| `..` | Removes the previous component of the text | Goes to the real parent through `FsDriver::parent` |
| `/a/missing/../b` | `/b` | `NotFound` |
| `/file.txt/..` | `/` | `NotADirectory` |
| Trailing `/` on a file | Ignored | `NotADirectory` |
| Symlinks | Not followed; a link mid-path gives `NotADirectory` | Followed, 40 at most (`Symlink`, like `ELOOP`), absolute and relative targets |
| Driver methods used | `lookup` | `lookup`, `node_metadata`, `parent`, `read_link` |
| Cost | One pin at a time, one `lookup` per component | Plus one `node_metadata` per component, and `parent` per `..` (a directory scan on FAT) |
| Memory | Nothing | An `N`-byte buffer in the resolve call, touched only when a symlink is followed. Path plus link text beyond `N` fails with `LimitExceeded`, like `ENAMETOOLONG` |
| `alloc` | Not needed | Not needed |

`Lexical` is the default because it works on every filesystem, including
ones with no `parent` support, and costs the least. It is how Windows and URL
resolution treat `..`. On a filesystem without symlinks it agrees with POSIX
whenever every component exists. `Posix` needs no `alloc`: symlink targets are
spliced into a fixed buffer, so POSIX semantics are available to kernels too.

Choosing a policy, per tier:

- Raw: `Posix::new().resolve(&mut fs, path)` for one call, or `fs.with_resolver(Posix::new())` so every `DriverExt` helper uses it. `Posix::<256>` picks a smaller buffer.
- Shared: `Volume::new(fs.with_resolver(Posix::new()))`. Every handle and helper on that volume uses it, and the whole path resolves under one lock hold.
- Generic code calls `resolve` on `FsDriver` or `FileSystem`. `Volume` forwards to its driver's policy; a custom `FileSystem` gets `Lexical` unless it overrides `resolve`.

Users can write their own `Resolver`. Two obvious ones: a jail that refuses
`..` past a starting directory, and case-insensitive lookup on top of a
case-sensitive format.

Not in 3.0: a no-follow mode for the last component (`lstat`, `O_NOFOLLOW`).
`Posix` always follows a final symlink. The shape allows adding it in 3.x as
another resolver type or an option on `Posix`.

### 4.13 Known costs and limitations

Revised 2026-09-24 (4.15): the limits of the fixed node table, the 16-entry
`Volume` queue and `Local` reentry below are removed with those types.

Each row is a deliberate trade, what it buys, and what a user does about it.

| Cost | Why it stays | What users do |
|---|---|---|
| Device errors must be `core::error::Error + Send + Sync + 'static` | Makes every device error erasable into `AnyError` and `io::Error` with no where-clauses | Wrap an error that holds an `Rc` or raw pointer |
| A non-io device error becomes `io::ErrorKind::Other` in std | std has no kind for "your device's enum" | Downcast `io::Error::into_inner()` to get it back |
| Generic code carries `F::DeviceError` | Keeps the device error without allocation | Use `FsResult<T, F::DeviceError>`, or return `AnyError` |
| Stored `Volume` types name the lock | A default lock would depend on a feature (R3) | One user-written alias |
| Read-only is detected on the first refused write, not up front | A static flag is unreliable and a probe write is harmful (4.2) | `MountOptions::with_read_only` when it is known; `is_read_only()` afterwards |
| The default resolver is lexical, not POSIX | Works on every format, costs least, needs no `parent` | `with_resolver(Posix::new())` |
| `Posix` costs a metadata call per component and a stack buffer | Symlink detection needs the type; no `alloc` | Pick `N`; use `Lexical` on formats without symlinks |
| No no-follow resolution | Not needed for 3.0 users so far | Planned as an additive 3.x resolver |
| Calls on one `Volume` serialize; a path resolves under one lock hold | One lock per call keeps format crates lock-free | A format can implement `FileSystem` with finer locks |
| Holding `vol.lock()` and calling the same volume deadlocks (panics with `Local`) | The guard is a plain lock guard | Drop the guard first; documented on `lock()` |
| Without `alloc`, `Volume`'s queue is 16 `(node, publishes, closes, forgets)` entries; when it is full and the lock is held, `forget` or `close_node` panics | Both run in `Drop` and must neither lose a call nor allocate (E1), and waiting could never end if this thread or task holds the lock | Drop the `vol.lock()` guard before dropping handles to more than 16 distinct nodes, or enable `alloc`, where the queue grows |
| Removing the last name of an open file fails with `Busy` | POSIX unlink of an open file needs orphan tracking, and a crash leaves lost clusters (Q3) | Close first; orphans can come in 3.x without a break |
| Async `Volume::new` needs `alloc` | `async-lock` and `event-listener` link `alloc` unconditionally | The `embassy-sync` feature adds an async `Local` lock with no allocator |
| The default FAT node table is `FixedTable<64>`; a full table gives `LimitExceeded` | No allocation in the driver, and a feature cannot change the table (R3) | Name `HeapTable` or a larger `FixedTable<N>` in the driver type ([Q8](#7-open-questions)) |
| Each device type monomorphizes the driver: about 2.4 KB of FAT per extra device type on Cortex-M, 180 bytes per extra device of the same type | Typed errors and static dispatch | Use one device type per binary where size matters |
| `OpenFile` does not close or forget on drop | It is `Copy` plain data for file tables | Call `close`; `File` handles close and forget on drop |
| `read_exact`/`write_all` return `ExactError<E>` | Short reads and zero-length writes need their own cases, as in embedded-io | `?` converts into `Error<E>` |
| `Access` appears in generic code over handles | One `File` type serves all tiers | `fn f<A: Access>(file: &mut File<A>)` |
| `Dir` is not an `Iterator` in async builds | No async iterator in core | `next_entry().await` |
| Async futures from generic code are not provably `Send` in the `async` mode | `async fn` in traits; `+ Send` would reject every non-`Send` device, including embedded-io-async ones | Use the `async_send` mode ([Q2](#7-open-questions)); spawn concrete types in `async` |

Experiment E1 built the prototype for `thumbv7em-none-eabihf` with no
allocator, with `alloc`, and with std. The raw tier that opens a FAT volume and
reads a file is 6.0 KB of `.text` (`opt-level = "s"`, LTO), `Volume::local`
adds 1.9 KB, `Posix<256>` another 0.8 KB, and async raw is 8.7 KB. 64-bit
division (0.9 KB) and `memcpy` (1 KB) are fixed costs; FAT should shift by the
cluster size instead of dividing. The binaries were linked and measured, not
run on hardware. Dropping a `Volume` applies its queued forgets, so a `Volume::local(&mut fs)`
leaks no pins into `fs`; the prototype discarded them.

Not yet verified: embedded-io behind a feature, error context inside
`Error<E>`, the async `FromEmbedded` adapter, and a tokio file device. The prototype sizes a `std::fs::File` device from
its metadata, which reports 0 for host block devices such as `/dev/sdX`; the
real impl seeks to the end instead.

### 4.14 Compatible 3.x additions to the frozen traits

The step 6 review found these gaps. Each fits R10: a default method or a new
private field with a getter, plus a `Capabilities` flag where callers must
ask first. None needs a break, so none blocks 3.0.

| Addition | Default | Needed by |
|---|---|---|
| `pin(node)` for an id just returned by `read_dir_entry` | `Unsupported` | FUSE `readdirplus`, `getdents` plus `stat` without a second directory scan |
| `forget_n(node, count)` | Calls `forget` `count` times | FUSE `forget` and `batch_forget` |
| `Metadata::allocated()`, `generation()`, `device()` | `None`, 0, `None` | `stat` block counts, FUSE generations for reused ids, device numbers from ISO RRIP and cpio |
| `link(node, dir, name)` | `ReadOnly` | NTFS and UDF write, FUSE `link` |
| `NewNode::Fifo`, `NewNode::Socket` | Formats refuse with `Unsupported` | cpio, ISO RRIP, FUSE `mknod` |
| `RenameFlags::EXCHANGE` | Formats refuse unknown flags with `Unsupported` | `renameat2` |
| Extended attributes and named streams | `Unsupported` | NTFS ADS, UDF named streams, macOS clients |
| A `lookup` that also returns the stored name | Lookup plus a listing scan | Case-insensitive formats, the FAT CLI |
| `FsStats` available blocks and free file slots | Free blocks, unknown | FUSE `statfs` `bavail` and `ffree` |
| Per-field time resolution in `Capabilities` | The modification time's | FAT (created 10 ms, modified 2 s, accessed 1 day) |
| `DateTime` conversions to and from `SystemTime` | None | Every std adapter |
| Orphan tracking: unlink of an open node succeeds | `Busy` | POSIX semantics (Q3) |

### 4.15 Layers per format: raw, shared, host and embedded

The V3 review (2026-09-24) wrote real programs against 3.0 for OS images,
forensics, firmware, servers, FUSE and conversions. Most friction came from
one driver serving firmware and hosted code at once: firmware wants a few
fixed node slots, 512-byte buffers, ASCII folding and two-byte errors, while
hosted code wants unbounded tables, Unicode, paths in errors and no visible
generics. Features may not switch behaviour, so one type cannot choose per
build. A first answer (type-alias profiles over one generic engine) kept the
firmware generics on the shared type. The user chose instead to build every
format on a public low-level layer and to give firmware its own API.

```
hadris-<fmt>-raw      I/O-free codecs (stable) and thin device primitives
                      (raw::io, sync and async); for anyone building their own
<fmt>::{sync,async}   FsDriver/FileSystem drivers, Volume, handles; kernels,
                      host applications, servers, FUSE; requires alloc for FAT
<fmt>::host (std)     open(path), host::Error, std::fs-named path methods
<fmt>::embedded       handle-based firmware API over raw; fixed buffers, no
  ::{sync,asynch}     node table, no Unicode tables unless asked
```

**Raw crates.** Each format gets a `hadris-<fmt>-raw` crate with its own
version, so the low-level API can evolve without breaking the format crate's
major version. Users whose case the higher layers do not cover are pointed
here.

- The I/O-free part is pure functions and small state machines over byte slices: for FAT, boot sector parsing into `Geometry`, FAT entry encode and decode, `ChainGuard` (cycle detection), free-cluster scans, directory slot parsing and encoding, long-name assembly and checksums, short-name generation, dates with a UTC offset, the format layout planner and boot fields (hidden sectors, CHS geometry), and name folding as a function pointer (`fold_ascii` by default; Unicode tables link only when referenced). exFAT adds entry sets (parse, validate, seal), set checksums, name hashes, the up-case decoder, bitmap helpers and times. Most of this exists today as the private `codec` modules (about 2,900 lines).
- `raw::io` is a thin layer of device primitives generic over the device only, borrowing a caller buffer: `read_geometry`, FAT `get`/`set` (every copy), `next`, `runs` (contiguous extents, cycle guarded), `allocate`/`free_chain` with FAT writes batched per sector, directory iteration with a cached position, `write_slots`, `mkfs`, and `check` that runs without mounting. exFAT adds the bitmap, a lazy up-case index and `write_set`, which writes a whole entry set per block, secondaries before the primary. Ordering-sensitive sequences live here once, so both the shared driver and the embedded API inherit the same crash ordering.
- A fully sans-IO design was considered. It works for the codecs, but operations that read, decide and read again (chain walks, directory iteration, allocation) would need a hand-written state machine each on stable Rust. `raw::io` is instead generated per mode from one source.

**Shared drivers.** `FatFs<D>` and `ExFatFs<D>` have one type parameter and
require `alloc`. The clock and code page are runtime options, and the node
table is an internal heap table with an optional cap (`with_max_nodes`).
`NodeTable`, `FixedTable` and `HeapTable` leave the public API. `Volume`
requires `alloc` and its deferred queue is a `Vec`. The `Local` lock kinds
and the `embassy-sync` feature are removed; firmware shares the embedded API
with its own mutex. Drivers without allocation needs (ISO, UDF, NTFS
readers) keep working without `alloc` at this tier.

**Modes.** The shared tier ships `sync` and `async`, where `async` is today's
`async_send` (futures are `Send` when the device is). Non-`Send` async
remains in `hadris-io`, `hadris-storage` and the embedded API. Sync stays
generated from the async source by macro: a measurement on 2026-09-24 of
async code driven by `block_on` over an always-ready device, with fat LTO at
opt-level `s` and `z`, found +40 to 49% code, +70% worst-case stack (26 KB to
45 KB on thumbv7em), 40 or more poll state machines left in the binary and
1.3 to 2.5 times slower host throughput, with identical images.

**Host.** `hadris::<fmt>::host` (with `std`) adds `open(path)`,
`open_device(device)`, `format`, the non-generic `host::Error` (kind,
context, path, boxed source; it replaces `AnyError`) and path methods named
after `std::fs` on the shared `Volume`. The FAT code page defaults to
`Cp437`. `hadris::host::open(path)` detects block, partition, optical and
archive images.

**Embedded.** `hadris_fat::embedded::{sync, asynch}` is a separate,
handle-based API built only on the raw crate:

```rust
pub struct Fat<D, const FILES: usize = 4> { .. }
impl<D: BlockDevice, const N: usize> Fat<D, N> {
    pub fn mount(dev: D) -> Result<Self, MountError<D, D::Error>>;
    pub fn root(&self) -> Dir;
    pub fn open_dir(&mut self, parent: Dir, name: &str) -> Result<Dir, Error<D::Error>>;
    pub fn open(&mut self, dir: Dir, name: &str, mode: Mode) -> Result<File, Error<D::Error>>;
    pub fn read(&mut self, file: &File, buf: &mut [u8]) -> Result<usize, Error<D::Error>>;
    pub fn write(&mut self, file: &File, buf: &[u8]) -> Result<usize, Error<D::Error>>;
    pub fn flush(&mut self, file: &File) -> Result<(), Error<D::Error>>;
    pub fn close(&mut self, file: File) -> Result<(), Error<D::Error>>;
    pub fn list(&mut self, dir: Dir, each: impl FnMut(&Entry) -> ControlFlow<()>) -> Result<(), Error<D::Error>>;
    // create_dir, remove, rename, seek, sync, into_inner
}
```

512-byte blocks, `File` is move-only, errors are `Error<E>`, and the targets
are under 2 KB of RAM and under 2 KB of mount stack, checked in CI on
`thumbv6m`, `thumbv7em` and `riscv32imc`. 3.0 ships FAT12/16/32 read and
write and exFAT read-only; exFAT write follows in 3.x.

**Errors.** One `hadris_fs::Error<E>` for every crate. Its private context is
`Copy` and allocation free: a `&'static str` message, an optional location
(byte, block, cluster) and a detail code, so nothing is dropped at crate
boundaries. Each crate keeps a `Detail` enum with `Detail::of(&err)`, and the
per-crate `Error` wrappers are removed. `ErrorKind::NotRecognized` separates
"not this format" from `Corrupt`. An error either shows its source's text or
returns it from `source()`, never both.

**Other formats.** ISO and UDF readers already work without allocation and
need no embedded API. Their raw crates gain, in 3.x, the record and FID
iterators, the Rock Ridge decoder, CS0 and d-character codecs and descriptor
selection. NTFS exposes its record layer under `unstable-ntfs`. cpio and
partitions need little beyond making existing codecs public.

**Other simplifications.** The `contract` kit moves to a `hadris-fs-contract`
crate outside 3.0 semver. Resolvers and `impl_fs_driver!` stay.

**What this settles from the review.** C1 to C6, D1, D7 and D8, and B5 and B6
as targets of the embedded API. A1, A2, A5, B3 and B4 move into `raw::io`
primitives; A4 disappears with the `Vec` queue. Decisions: Q9.

### 4.16 Review decisions on API shape

Decided by the user on 2026-09-24 from the V3 review (items D2 to D12):

| Item | Decision |
|---|---|
| D2 labels and serials | Text is always `label()`, a numeric id is always `volume_serial()`, in every crate. `FsDriver` gains a defaulted `volume_label()` so generic code and the openers can show it. UDF keeps `logical_volume_id()` as an extra. |
| D3 features | `hadris-fs` defaults to `std` and `sync` like every other crate. `write` keeps meaning "can create images" in every crate, documented per crate. |
| D4 exFAT detection | `BlockFormat::Fat(FatKind)` and a separate `BlockFormat::ExFat`; `FatVariant` is removed. |
| D5 optical detection | `detect` returns `OpticalFormats`; an empty set means nothing was found. |
| D6 `OpenFile` | Move-only: `close` consumes it, so a double close does not compile. |
| D9 directory entries | `DirEntry::metadata()` returns the metadata the directory entry already stores, filled by every current driver. `Dir::driver()` and a node-based, cycle-safe `walk` helper are added. |
| D10 FAT times | `MountOptions::with_utc_offset`. The engine and embedded profiles default to UTC; the host profile defaults to the system's local offset. exFAT reads and writes its UTC offset fields. |
| D11 permissions | `Metadata::permissions()` and `SetMetadata::with_permissions()`; the `mode` spellings leave `hadris-fs`. cpio keeps its raw `mode()`. |
| D12 trait additions | Stay 3.x additions under 4.14 (`link`, `forget_n`, `Metadata::device()` and the rest, orphans). |

### 4.17 Workspace simplifications

Decided by the user on 2026-09-24 after the layering pass (4.15):

| Item | Decision |
|---|---|
| S1 CLIs | One `hadris` binary with subcommands (`fat`, `iso`, `udf`, `cpio`, `detect`) replaces the five CLI crates, with one set of flags, overwrite rules and output handling. The 2.x binary names are not installed. |
| S2 detection | `hadris-block` and `hadris-optical` are removed. Detection and opening move into the umbrella as `hadris::detect` (one format enum for block, partition, optical and archive images) and `hadris::open`, which implements `FsDriver` by delegation. |
| S3 bridge writer | `hadris-cd` is removed; the ISO and UDF bridge writer becomes `hadris_udf::write_bridge`. |
| S5 storage errors | `WriteError`, `StorageError` and `OutOfRange` become one storage error type. |
| S6 path helpers | Path helpers exist on shared volumes and in `host`; the bare-driver tier keeps node-level calls. `DriverExt` and `PathExt` become one trait. |
| Async naming | In the shared tier `r#async` means futures that are `Send` when the device is (the former `async_send`). The embedded API's `r#async` is non-`Send`. `hadris-io` and `hadris-storage` offer `sync`, `r#async` (`Send`) and `local` (non-`Send`) device traits. Features are `sync` and `async`; `async-send` is removed. |
| S4 `write` | Undecided: dropping it leaves writers always compiled and shrinks the feature matrix; keeping it makes it stable for 3.x. Settled with the feature rework. |

### 4.18 Action catalog and API prototype

Decided on 2026-09-24. [docs/v3/actions.md](v3/actions.md) lists every action on FAT, exFAT, ISO 9660, UDF and cpio with a stable ID, the users who need it and a verdict (`3.0`, `3.x` or `no`), plus the non-functional constraints (`NF-*`). An action marked `3.0` that does not work is a bug; each ID gets a conformance test in hadris-tests.

Before more implementation, the API is redesigned against the catalog as a prototype: a standalone crate with `todo!()` bodies and realistic signatures (lifetimes, `Send` bounds, error types), never merged. An item enters the prototype only when a catalog action cannot be written without it, and names the action IDs it serves. Every `3.0` action gets a usage snippet that must compile, and a FUSE-shaped adapter is written against it as a test of the core set. Sections 4.15 to 4.17 are inputs to check against the prototype, not fixed results.

Pass 1 (core mounted API), accepted on 2026-09-24:

- One node trait `FileSystem` on `&mut self` with the catalog's operation names; write methods default to `ReadOnly`, so ISO and UDF implement the same trait. It replaces `FsDriver`, `Access`, `AsDriver` and `OpenFile` (D6 is superseded).
- The bare tier tracks opens per node: `open(node)`, `read`/`write` at an offset, `close(node)`. Positions and append live in the caller or in `File`. The bare tier trusts the caller's open mode.
- `Volume<F>` has no lock parameter (sync uses the std mutex, async a portable async mutex needing only `alloc`); `LockKind` from 4.15 is dropped. A no_std sync user has the node API; `Volume::with_lock` can be added later.
- `truncate` stays separate from `setattr`, since they fail differently.
- UDF read-write arrives in 3.x through a new `UdfFs::mount_writable`; `mount` stays read-only in every version.
- `forget(node, count)`, generation, allocated size and device number are 3.0 (the catalog overrides D12). No-follow resolution is 3.0 (overrides 4.13).

---

## 5. Per-crate changes

Each section lists the API changes, then the V3 feature work. Items marked
"3.x" may land after 3.0.0 because their shape ships in 3.0.

### 5.1 `hadris-fat`

**One shape for FAT12/16/32; exFAT is a sibling driver.**

```rust
let mut vol = FatFs::open(dev)?;                  // NoClock, Ascii, FixedTable<64>
let mut vol = FatFs::open_with(dev, MountOptions::new().with_clock(SystemClock).with_code_page(Cp437))?;
vol.kind();                                      // FatKind::{Fat12, Fat16, Fat32}
vol.label()?; vol.set_label(&VolumeLabel::new("BOOT")?)?;
vol.fat_attributes(node)?; vol.set_fat_attributes(node, FatAttributes::HIDDEN)?;
vol.cluster_chain(node)?;                        // native, for tools
// Everything else goes through the node API (FsDriver) and the path layer.

let vol = hadris_fat::sync::format(dev, FormatOptions::new().with_kind(FatKind::Fat32))?;
let exfat = hadris_fat::exfat::sync::ExFatFs::open(dev)?; // the same node API
let report = hadris_fat::sync::check(&mut vol)?;           // fsck, both modes
```

- `FatFs` implements `FsDriver` through its inherent methods (`impl_fs_driver!`). Path methods come from the `hadris-fs` path layer. `FatVolumeReadExt` and `FatVolumeWriteExt` are removed. See [Q7](#7-open-questions) for the name.
- exFAT is a separate driver, `ExFatFs`, not a `FatKind`. Its directory entry sets, allocation bitmap and up-case table share little with FAT12/16/32, so one type would branch on the kind in every method. Both drivers share the node table, the vocabulary and the codecs that do overlap. `ExFatFs` lives in the `hadris_fat::exfat` module with its own `sync`, `r#async` and `async_send` modes, `MountOptions`, `FormatOptions`, `VolumeLabel`, `CheckReport`, `Finding` and `raw`, because those names differ from FAT's and R5 keeps them out of the crate root. The user decided on 2026-09-24 to promote exFAT in 3.0 because it passes the suite: `ExFatFs` is stable, in every build, and the `unstable-exfat` feature is gone.
- The node table (4.5) replaces `FileEntry` snapshots. `StaleEntry` and `WriterConflict` go away as errors; a second writer on the same node shares the node and its size.
- Clock and code page are generic parameters with zero-sized defaults: `FatFs<D, T: NodeTable = FixedTable<64>, C: Clock = NoClock, P: CodePage = Ascii>`. No `'static` borrows, no `Sync` supertraits (#83). `MountOptions<T, C, P>` chooses them with `with_table`, `with_clock`, `with_code_page` and `with_read_only`; `FatFs::open(dev)` takes the defaults and `FatFs::open_with(dev, options)` the rest, so one constructor pair replaces the `open`/`open_read_only`/`open_with_table` combinations. `Clock` lives in `hadris-fs` because every writable format needs time; `CodePage` (`Ascii`, `Cp437`) lives in `hadris-fat` because only FAT short names use an OEM code page. `Ascii` reads a byte `b` above `0x7F` as the private-use character `U+F700 + b`, so short names stay distinct and look up by the name listed. No chrono clock ships: `SystemClock` covers `std`, and FAT's local-time convention is left to a user `Clock`.
- The FAT sector cache is gone as a separate thing. Users wrap the device in `Cache<D>`. `FatSectorCache`, `CachedFat`, `with_cached_fat` and `fat_cache` are removed (#27). Chain caching becomes an internal detail of the node table.
- `FormatOptions` (non_exhaustive, `with_*`) covers FAT12/16/32 with a `FatKind` selection; exFAT has its own `hadris_fat::exfat::FormatOptions`. It replaces `FatVolumeFormatter`, `FatFormatOptions`, `ExFatFormatOptions`, `format_exfat` and `ExFatLayoutParams`. Formatting uses the device's block count, so pre-sizing and a separate size argument go away, and formatting a `Slice` inside a disk works. `format(dev, options)` takes the options by value, so the clock moves into the returned `FatFs<D, FixedTable<64>, C>` without a `Clone` bound; `into_inner` and `open_with` remount with other options. Fields are private, so the struct needs no `non_exhaustive`. Without `with_kind`, volumes below 16 MiB are FAT12, below 512 MiB FAT16, and larger ones FAT32 (the V2 formatter's 32 MiB and 2 GiB limits failed at 32 MiB and above 512 MiB); the cluster size starts from the V2 tables and doubles or halves until the count fits the variant. The volume id defaults to one derived from the clock, so `NoClock` formats reproducibly. Errors: `NoSpace` (too small), `LimitExceeded` (too large), `InvalidInput` (bad option, checked before any write), `Unsupported` (blocks over 4096 bytes), each returned as a `MountError` that gives the device back, as a failed final mount does.
- `tool::` becomes `check` (fsck) and `analysis` in both modes, with getters on reports. A `repair` pass is 3.x. `check(&mut fs)` returns a `CheckReport` of counts; `check_with(&mut fs, bitmap, on_finding)` also passes each `Finding` (a `non_exhaustive` enum located by byte offsets) to a callback, so no collection needs an allocator. Cross-links and lost clusters need a bit per cluster: the caller's bitmap covers a window of clusters and the tree is walked once per window, so the findings do not depend on its size and `check` uses 4 KiB. The tree is walked without a stack by following `..` entries, so a directory whose `..` does not name its parent is reported and not entered. `analysis` is not ported yet; tools read chains with `cluster_chain(node, visit)`.
- `expect("Fixed root info required ...")` sites return `ErrorKind::Corrupt`.
- exFAT internals (`allocate_cluster`, `sync_bitmap`, `name_hash`, `parse_entry_set`) become private.

**Feature work.**

- Random-access writes, read-write handles, grow through `set_len`.
- Create the root label entry when absent, and write the BPB label.
- Exact free space on FAT12/16 by scanning, cached after first use.
- `remove` of an open node fails with `Busy`; a pinned node is removed and answers `NotFound` (4.5, Q3).
- Close audit items C2 (cancellation safety of compound async operations) and B3 to B7, or confirm they are fixed.
- exFAT: async, rename, attributes and times, label, directory growth, fragmented bitmap and upcase table, entry sets that cross clusters, fsck. Done in step 12; exFAT passed the conformance suite and is stable in 3.0. See [Q5](#7-open-questions).
- TexFAT volumes (two FATs) are mounted, written with both copies kept equal, and formatted. TexFAT transactions and fsck repair: 3.x.

### 5.2 `hadris-iso`

**Reading.** One reader, `IsoImage<D>`, in each mode:

```rust
let mut iso = IsoImage::open(dev)?;
let ns = iso.namespaces();                         // what the image has
let mut view = iso.view(Namespace::Preferred)?;    // IsoView<&mut D>: FsDriver
let entry = view.metadata("/boot/grub/grub.cfg")?; // DriverExt
let rr = view.rock_ridge(node)?;                   // Option<RockRidgeInfo>, native
let boot = iso.boot_catalog()?;                    // Option<BootCatalog>, native
```

- `IsoReader` (no-alloc) and `IsoImage` (alloc) merge. The no-alloc core is the implementation, and `alloc` adds convenience on the same types.
- An ISO has up to four trees (primary, Joliet, Rock Ridge over primary, enhanced). `IsoView` picks one and implements `FsDriver` over it. `Namespace::Preferred` keeps V2's preference order, but Rock Ridge no longer hides Joliet names; the caller can pick.
- `DirectoryRef`, `LogicalSector` and raw records are not needed to walk a tree. They stay under `raw` and via `view.raw_record(node)` for the CLI verifier.
- `BootCatalog` is public and readable. The CLI stops parsing boot records by hand.
- `IsoStr::as_str` returns `Result`. Panicking `best_choice` and `primary` are removed.
- Non-2048 logical block sizes work in the unified reader, because the device layer already handles block size.

**Writing.**

```rust
pub struct IsoOptions<C = NoClock> { /* private */ }
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

- `UdfVolume` implements `FsDriver`: path lookup, streaming `read_at`, and metadata with times, permissions and owners. V2 has only `read_file -> Vec` and no path lookup.
- A no-alloc read path. V2 needs `alloc` for everything.
- The writer takes the shared `Tree` and `UdfOptions` (non_exhaustive, `with_*`) and returns `UdfReport`. `SimpleFile` and `SimpleDir` are removed.
- Low-level descriptor writers (`write_lvid(location, close: bool)`, `write_fids`) become `pub(crate)` or move to `raw::write` with enums instead of bools.
- No trait bounds on the `UdfVolume` struct definition.
- `write` no longer needs `std`.
- The dead `modify.rs` is deleted. UDF on random-access media is a real read-write filesystem, so modification means `UdfVolume` implementing the `FsDriver` write methods for Type 1 partitions, not a second modifier API. The shape ships in 3.0 with `Capabilities::writable` false; the implementation can be 3.x.
- Reader coverage: UDF 1.50 and 2.01 reading, prevailing-descriptor selection, allocation-extent chaining, extended allocation descriptors, stream directories.
- 3.x: VAT, sparing tables, metadata partitions (2.50+), which all live behind the same `UdfVolume`.

### 5.4 `hadris-cd`

- One entry point: `hadris_cd::sync::write(target, &tree, &CdOptions)`. `CdOptions` holds `IsoOptions` and `UdfOptions` and re-exports them. The `new(..).finish(..)` and `create(..)` pair with swapped arguments goes away.
- It uses `report.extent_of(..)` from the ISO writer instead of reopening the output.
- Metadata, symlinks and streaming inputs come from the shared `Tree`.
- `LayoutManager` becomes private.
- Async support, since both underlying writers have it.

### 5.5 `hadris-ntfs`

- `NtfsVolume` (renamed from `NtfsFs`) implements `FsDriver`, with metadata including times. Security descriptors are native (`vol.security_descriptor(node)`), since `Metadata` stays `Copy` (4.3).
- `NtfsError` becomes `Error`. `raw::*` is no longer glob re-exported. `attr` types with raw `u8`/`u32` codes move to `raw`; the public API uses enums.
- Native API for streams: `vol.streams(node)` lists named data streams, and `vol.read_stream_at(node, name, offset, buf)` reads one.
- `open` seeks to the boot sector instead of reading from the current position (falls out of `BlockDevice`).
- Reachable from `hadris-block` detection, `OpenVolume` and the umbrella.

**Feature work.** `$ATTRIBUTE_LIST`, `$MFTMirr` fallback, compressed streams,
reparse points (exposed as symlinks where they are symlinks or junctions),
keyed B-tree lookup, filtering DOS 8.3 duplicates from listings. Encrypted
streams stay `Unsupported`. Write support is 3.x behind the `FsDriver` write methods, and
`$LogFile` replay comes with it. NTFS stays in `unstable` until its read side
passes a conformance slice. See [Q5](#7-open-questions).

### 5.6 `hadris-part`

```rust
let disk = hadris_part::sync::read(&mut dev)?;     // block size from the device
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
    let fs_dev = hadris_part::sync::open(&mut dev, &p)?;   // Slice<&mut D>
}
```

- `MbrType(u8)` with associated constants replaces `MbrPartitionType` and the 256-variant `MbrPartitionTypeFull`.
- GPT type GUIDs move to `gpt::types`. `Guid` implements `FromStr`; the inherent `from_str` becomes `Guid::parse_const`.
- `Gpt` fields are private. Edits go through methods that keep CRCs correct. Writing always computes CRCs.
- `HybridMbr` has private fields and `add_mirrored(..)`, identical with and without `alloc`.
- The six `*ReadExt`/`*WriteExt` traits and `PartitionInfoTrait` collapse into `read`, `write`, `create`, `scan` and `open` in each mode, and the layouts and CRC in `raw`.
- Bool parameters (`bootable`) become `PartitionFlags`.

**Layout builder.** One flow for "disk image with partitions":

```rust
let layout = DiskLayout::gpt(disk_guid)
    .with_alignment(Alignment::MiB1)
    .partition(PartitionSpec::new(gpt::types::EFI_SYSTEM, Size::MiB(100)).with_name("EFI"))
    .partition(PartitionSpec::new(gpt::types::LINUX_FILESYSTEM, Size::Remaining));
let disk = hadris_part::sync::create(&mut dev, &layout)?;   // build(block_count, block_size) + write
let esp = hadris_part::sync::open(&mut dev, &disk.partition(0).unwrap())?;
hadris_fat::sync::format(esp, FormatOptions::new())?;
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
- `OpenVolume::open` and `open_detected` mount once and give the device back in `OpenError` on every failure, using the driver's `MountError`.
- `OpenVolume` is `#[non_exhaustive]` and covers FAT, exFAT and NTFS. `OpenOpticalImage` covers ISO views and UDF. Both implement `FsDriver`, so "open whatever this is and list it" is one generic function.
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
2. **`hadris-io`.** embedded-io base, `StdIo`, `&mut T`, `ByteSource`, moved onto `strip_async!`. Done on `feat/v3-api`, first with an erased error, then reworked to `ErrorType` (4.1). The erased V2 traits live in `hadris_io::legacy`, which every format crate uses until its own port step; the last port deletes the module. `embedded-io` stays a required dependency until then, because `legacy` and the re-exported `SeekFrom` use it; the `embedded-io` feature and a Hadris `SeekFrom` land with that deletion.
3. **`hadris-storage`.** `BlockDevice` in both modes from one source, `WriteError`, `std::fs::File`, `StreamDevice`, `MemDevice`, `Slice`, `Cache`, `ByteView`. Done on `feat/v3-api`.
4. **`hadris-fs`.** Done: vocabulary, `ErrorKind`, `Error<E>`, `AnyError`, `DateTime`/`Clock`, steps 2 and 3 reworked to associated errors, the three modes (`sync`, `async`, `async_send`), `FsDriver`/`FileSystem`, `impl_fs_driver!`, `Volume`/`LockKind`, resolvers, helpers and handles, tested against an in-memory driver (the FS-generic parts of S1 to S16; the FAT-specific ones move to step 5), `copy_tree`, the sync host helpers `extract_to_host` and `import_from_host`, and `FuseOnError`. `hadris-path`, `hadris-fixed` and `hadris-archive` are merged or removed.
5. **`hadris-fat` as the reference implementation.** `BlockDevice` input, node table, `FsDriver` through inherent methods, `parent`, `FormatOptions`, `check`, clock and code page generics. Port the conformance adapter to the generic `FileSystem` adapter in the same PR. This step tests the trait design, and the trait can still change here. Done on `feat/v3-api`: the mode-independent codecs, the `async_send` mode, the public `NodeTable`, and `FatFs` reading and writing (`create`, `remove`, `rename`, `write_at`, `set_len`, `set_metadata`, `sync_node`, `sync`) for FAT12/16/32, checked against the V2 driver and the host `fsck` before it was deleted. Clock and code page generics with `MountOptions` and `FatFs::open_with`; `FormatOptions` with `format` in all three modes, without allocation, checked with `fsck.fat` and `fsck_msdos` and at the FAT12, FAT16 and FAT32 size limits; `check` and `check_with` in all three modes, without allocation, checked against `fsck.fat -n` verdicts and the leftovers of interrupted operations; `FatFs::label` and `FatFs::cluster_chain`. The conformance suite drives Hadris through `tests/src/fat/generic.rs`, one adapter over any `FileSystem`. The V2 driver is deleted with its `read`, `lfn`, `cache`, `tool` and `dirty-file-panic` features and the `chrono` dependency, and every user is ported: `hadris-block` detects on a `BlockDevice` and opens `FatFs` (an early part of step 11), `hadris-fat-cli` runs on `FatFs`, `check_with`, `cluster_chain` and the `hadris-fs` host helpers, and the fuzz targets and examples drive `FatFs`. The exFAT preview keeps its API on `hadris_io::legacy` with its own `exfat::Error` until step 12. Follow-ups: a library `analysis` module (the CLI computes fragmentation from `cluster_chain`), `set_label`, `fat_attributes`, and repair in 3.x.
   Spec compliance pass, done: mode, uid and gid stay ignored, as the `FsDriver::set_metadata` contract says and `copy_tree` and `import_from_host` rely on, with `Capabilities` reporting neither. A rename onto an existing target gives the result the requested name and case, as Windows and mtools do (Linux `vfat` keeps the target's), and reuses the target's slots so a full FAT12/16 root still accepts it. Short-name tails stay `~1` to `~4`, then take the Windows layout, two basis characters, four hashed hex digits and `~1` to `~9` (`LO1A2F~1.TXT`), instead of `LO~1A2F`. Renamed files get the archive bit, and the conformance model sets it on rename and on any change of contents or size. Names with a trailing dot or space stay refused with `InvalidInput`, so a created name is always the listed name; the specification says they are ignored, and Windows, Linux `vfat` (dots only) and mtools strip them.
6. **Freeze the traits.** Review `hadris-fs` against FAT, the conformance adapter and a prototype FUSE adapter before any other format ports. Done: [`v3-trait-review.md`](v3-trait-review.md) records the review, and the traits are frozen; later additions follow R10 and section 4.14. A FUSE prototype (`experiments/fuse-prototype`, `fuser` 0.18) mounted `FatFs` in a Linux container and ran shell workloads, then `fsck.fat`. The changes: pins and opens are separate (`open_node`, `close_node`), and only the last name of an open node is `Busy` (Q3); `remove` takes a `RemoveKind`; `ErrorKind` gains `NameTooLong` and `FileTooLarge`; `publish_node` writes pending metadata without a device flush while `sync_node` is durable, and the host `File` device's flush calls `sync_data`; `NodeId` 0 and cursors above `DirCursor::MAX_RAW` are ruled out; a listed id is the id `lookup` returns (a FAT id is now slot plus tier); the contract gained sentences on times, pending fields, cancellation and zone-less times; R10 gained rules for growing the traits and a forwarding test; `Metadata` stays `Copy` and `extra()` is dropped; sync `hadris-vfs` erases through `dyn FileSystem`; the `contract` feature adds a driver test kit that the test driver and `FatFs` pass; and `MountOptions::with_read_only()` lost its bool (R9).
7. **Errors and the R1/R2/R4/R5 pass, crate by crate.** Done for the crates already on the V3 API; every other crate gets its pass in its own port step. `hadris-io`: the root no longer glob re-exports `sync` (R5), so the traits are always `hadris_io::sync::Read` and its twins. `hadris-storage`: `BlockIndex` and `BlockCount` stay exhaustive plain value types with a private field and `const fn` `new`/`get` (R2), `BlockRange` and `BlockGeometry` get accessors, `StreamWrite` is sealed (R10), and `PartitionView`, the last legacy stream type, is removed in favour of `Slice`. `hadris-fs`: `DateTimeError`, `OpenOptionsError` and `PathError` convert into `Error<E>` like `NameError`, and `ContractViolation` is built only by the kit. `hadris-macros`: `send_async!` passes malformed input through instead of panicking (R6). `hadris-fat`: the struct variants of `Finding` are `#[non_exhaustive]`, and `raw` states the R4 promise; `MountOptions` and `FormatOptions` keep private fields without `#[non_exhaustive]`, as 5.1 says. `hadris-block`: `Error` reports `kind()` and `device_error()` and converts into `hadris_fs::Error` and `std::io::Error`, and `OpenError` gains `kind()` and `device()`. `hadris-common`: `MaybePod`, whose bounds changed with `bytemuck` (R3), goes, with the unused `optical` and `alg` modules and the `chrono`, `crc` and `rand` dependencies. The guardrail scripts learned the rest: `check-v3-api.py parity` compares `async` with `async_send`, no longer reads a method named `sync` as a mode, and lists intended differences with their reason; `check-non-exhaustive.py` reports `unstable-*` preview modules separately. Both are clean for these crates. Deferred: the private `Context` of `Error<E>` (sector, cluster, node, field) and typed accessors, which are additive, until a driver has detail to report; `Error` of `hadris-block` in every feature combination and the `read`, `detect` and `storage` features to step 11; the Hadris `SeekFrom` and the `embedded-io` feature to the deletion of `hadris_io::legacy`; FAT directory entry layouts in `raw` to step 13; the exFAT preview (`ExFatInfo`, `ExFatFileEntry`, `ExFatFormatOptions` and the rest) to step 12; `hadris-part` to step 8; and the `hadris-common` types only the optical crates use (`EndianType`, `extent::{Extent, FileType}`, `layout::{FileLayout, DirectoryLayout}`, the `fixed` types, `BOOT_SECTOR_BIN`, and the `sync` and `async` features that now only forward to `hadris-io`) to steps 9 and 10.
8. **`hadris-part`.** `Disk`, `DiskLayout`, `MbrType`, GUIDs, CRC always on, EBR. Done: `Disk` is a mode-independent value (4.8: it holds a table and does no I/O), so the I/O is free functions in each of the three modes: `read`, `write`, `create(&mut dev, &layout)`, `open(dev, &partition)` returning a `Slice`, and `scan`, which lists partitions through a callback without an allocator; `Disk`, the tables and `DiskLayout` need `alloc`. `PartitionTable` is `Mbr`, `Gpt` or `Hybrid`, `Partition` has `start`, `len`, `size_bytes`, `kind`, `flags`, `name` and `unique_guid`, and `Disk::runs` gives the table as runs of whole blocks, which the still-V2 ISO writer copies into its image unchanged byte for byte. `Mbr` follows EBR chains and writes them; `Gpt` keeps private fields, computes CRCs on every write, falls back to the backup copy (reported by `damaged_copy`, repaired by `write`) and names are UTF-16 `PartitionName`s; every edit (`add`, `add_logical`, `remove`, `resize`, `set_*`) checks bounds and overlap and changes nothing on failure. `MbrType(u8)` with constants, `gpt::types`, `Guid: FromStr` with `Guid::parse_const`, `Guid::random` only with `std`, `HybridMbr` with a fixed three-slot array, and `PartitionFlags`. Errors are `Error<E>` (a `hadris-fs` `ErrorKind`, a non-exhaustive `Detail` and the device error) and `TableError` for edits. Features are `std`, `alloc`, `sync`, `async` and `async-send`; `read`, `write`, `crc` and `rand` are gone. `hadris-block` drops its `partition` module for `part::sync::open`, and the `part_read` fuzz target edits, writes and re-reads what it parses. Not done: moving the backup GPT to the end of a grown disk, and entry sizes above 128 bytes keep only their first 128 bytes on rewrite.
9. **`hadris-iso`.** Unified reader, `IsoView`, `IsoOptions`, `Tree` input, report, sessions, async writer. Done: `hadris-fs::tree` holds `Tree`, `Content` (bytes, blocking and async sources, lazy host paths, stored extents), `ContentReader`, `Warning`, `Tree::from_fs` with `FromFsOptions::with_on_error` and `TreeExt::from_filesystem` over any `Access`; `Extent` is at the `hadris-fs` root and `contract::check_read_only` checks read-only drivers. `hadris-iso` has one allocation-free reader in each mode: `IsoImage::open` (failing with `MountError`), `namespaces`, `view` and `into_view` returning `IsoView<&mut D>` or `IsoView<D>`, which implements `FsDriver` through `impl_fs_driver!` with node ids that are record offsets, so no node table is needed; `rock_ridge`, `raw_record`, `extents`, `descriptor`, `primary_descriptor` and `boot_catalog` are native, the layouts are in `raw`, `IsoStr::as_str` returns a `Result`, and logical blocks from 512 to 2048 bytes work. `write` and `plan` take a `Tree` and `IsoOptions<C = NoClock>` and return a `Report`; the planner does no I/O and the emitter writes blocks in ascending order, so every mode writes to any `BlockDevice` without `std`. Output is byte-identical to V2 for option sets without Rock Ridge. `Session` implements `Append` (descriptors and new data after the old volume, the new descriptors also copied to sector 16 so readers without multisession support see the new tree) and `Rewrite` (new descriptors at 16, new data after the old volume or the partition that holds it, tables regrown and the backup GPT moved). `Error<E>` follows `hadris-part`, the reader is off `hadris_io::legacy`, features are `std`, `alloc`, `sync`, `async` and `async-send`, and `hadris-common` drops `EndianType`, the `fixed` types and `BOOT_SECTOR_BIN`. `hadris-optical` opens ISO images through a transitional `StreamBlocks` adapter, `hadris-cd` writes its ISO part from a `Tree` and the `Report`, and the CLIs, conformance suite, examples and fuzz targets use the V3 API. Decisions: Joliet and enhanced trees keep the real hierarchy when Rock Ridge relocates; without Rock Ridge, symlinks and device nodes are left out with a warning; the relocation directory is only the name given (no `.rr_moved` fallback); files of 4 GiB or more need `IsoLevel::L3`; Rock Ridge serials are per node and shared by hard links; `Session::options` writes at Level 3. Not done: Joliet beyond the BMP, zisofs, RRIP SF and RR, Apple Partition Map, `write_stream`; `hadris-common`'s `extent` and `layout` modules and its `sync` and `async` features wait for step 10, and `StreamBlocks` in `hadris-optical` for step 11.
10. **`hadris-udf`, `hadris-cd`, `hadris-cpio`.** Shared `Tree`, streaming readers and writers, UDF `FileSystem`. Done: `hadris-udf` has one allocation-free reader in each mode, `UdfFs` (the `<Format>Fs` name of Q7, not 5.3's `UdfVolume`), which implements the read-only `FsDriver` through `impl_fs_driver!` with `parent` and `read_link` and passes `contract::check_read_only`; node ids are ICB locations. It probes logical blocks of 512 to 4096 bytes and the three anchor positions, selects prevailing descriptors, falls back to the reserve sequence, follows descriptor pointers, allocation extent descriptors and every allocation descriptor form, reads extended file entries and symlink path components, and refuses type 2 partition maps with `Unsupported`; stream directories, indirect entries, VAT, sparing and metadata partitions wait for 3.x behind the same type, and the `FsDriver` write methods report `ReadOnly`. `write` and `plan` take a `Tree` and `UdfOptions<C = NoClock>` and return a `Report` (5.3 said `UdfReport`; the name follows `hadris_iso::Report`), writing blocks once in ascending order; `Bridge` points a volume at `Content::stored` extents of an ISO 9660 image on the same device. Output is byte-identical to 2.4 with the clock pinned, apart from fixes (descriptor version 3 and FID unique ids for UDF 2.00 and later, integrity descriptor counts, directory link counts, the root as its own parent, FID tag locations), and is read by the Linux kernel, macOS, `udfinfo` and 7-Zip (which refuses symlinks); `mkudffs` volumes read back. `hadris-cd` is `write(dev, &tree, &CdOptions)` and `plan` in all three modes: it plans the UDF metadata, writes ISO 9660 after it with `min_blocks`, and writes the UDF bridge over the extents of the ISO `Report`; images match 2.4 apart from the UDF fixes and a correct next unique id. `hadris-cpio` reads newc, newc-crc, odc and old binary archives with an allocation-free `CpioReader` whose entries implement `Read` and are skipped when dropped, and writes newc, newc-crc and odc through a streaming `CpioWriter` and `write(out, &tree, &opts)`, in all three modes; output without hard links is byte-identical to 2.4, and hard link groups follow GNU cpio. Every crate has `Error<E>` with a `Detail`, `raw` layouts, and the features `std`, `alloc` (implied for `hadris-cd`), `sync`, `async` and `async-send`. `hadris-common` keeps only the endian integers FAT and NTFS use: `extent`, `layout` and the forwarding features are gone. `hadris-optical` opens UDF through `UdfFs` over `StreamBlocks`, and the CLIs, fuzz targets and example use the V3 APIs. Decisions: device nodes, creation times and DOS attributes are left out of UDF with a warning; the ISO-only and UDF-only switches of the 2.x CD writer are gone in favour of the format writers; cpio's `Format` has a read-only `Binary` variant. Not done: `hadris_io::legacy`, still used by the exFAT preview, NTFS, `hadris-block` detection and `hadris-optical`, waits for steps 11 and 12.
11. **`hadris-ntfs`, `hadris-block`, `hadris-optical`, umbrella.** Done: `hadris-ntfs` has one allocation-free reader in each mode, `NtfsFs` (the `<Format>Fs` name of Q7, not 5.5's `NtfsVolume`), which implements the read-only `FsDriver` through `impl_fs_driver!` with `parent` and passes `contract::check_read_only` on crafted volumes and on volumes made by `mkntfs` and written through an `ntfs-3g` mount; node ids are file references (record number, sequence number in the top 16 bits). Records, index blocks and a page cache of `$UpCase` are fixed buffers, so MFT and index records and device blocks are limited to 4096 bytes. It follows `$ATTRIBUTE_LIST` into extension records for streams, names and index roots (ntfs-3g moves the root's `$INDEX_ROOT` out once the root record fills) and for a `$MFT` of up to 32 extents, applies update sequences in 512-byte strides, filters DOS aliases from listings (part of 5.5's feature work), and has 5.5's native `streams` and `read_stream_at`, plus `label`. `Error<E>` has a `Detail`, the boot sector is `raw::BootSector` with the codes beside it, and the parsers are private. `hadris-block` detects NTFS and its `OpenVolume` opens FAT12/16/32 and NTFS; `hadris-optical` detects and opens on a `BlockDevice` in all three modes, and `StreamBlocks` is gone. Both openers are structs over a private driver enum that implement `FsDriver` by delegation and give the device back in `OpenError`, and both facades' `Error<E>` (kind, `Detail`, device error) exists in every feature combination. Their features are the platform and mode axes plus `write` and `part` (block), `cd` (optical) and `unstable-ntfs` (block); `read`, `detect`, `storage`, `fat`, `open`, `iso` and `udf` are gone. The umbrella always re-exports `io`, `storage` and `fs` and each format crate at a flat path behind a feature of its name (`hadris::fat`, `hadris::iso`, `hadris::block`, `hadris::ntfs` behind `unstable-ntfs`). The fuzz target and `fs_dump` walk NTFS through the node API, and the example and website use the V3 facades. After this step the exFAT preview in `hadris-fat` is the only user of `hadris_io::legacy`, so the module, the Hadris `SeekFrom` and the `embedded-io` feature wait for step 12. Decisions: `hadris-ntfs` reads without an allocator, like the other V3 readers, so `OpenVolume` needs none either; `OpenVolume` always opens NTFS, because a feature that decided whether it does would switch behaviour, and `unstable-ntfs` only adds the re-export and the `as_ntfs` accessors, keeping the preview's native API out of the stable surface (Q5); the openers are structs rather than `#[non_exhaustive]` enums for the same reason; listings hide the metadata files (records below 16) as ntfs-3g does, while `lookup` finds them and prefers an exact listed name over a case-folded one; names with unpaired surrogates show U+FFFD, as in FAT; `hadris-ntfs` stays out of the public API snapshots as a preview. Not done: compressed streams, reparse points as symlinks, `$MFTMirr` fallback, the `$Mft` bitmap, keyed B-tree lookup, security descriptors (`security_descriptor(node)` of 5.5) and `$LogFile`, all additive later; a mount failure keeps only the kind of an NTFS, ISO or UDF error, since `MountError` holds a `hadris_fs::Error`.
12. **exFAT feature work and async parity** until exFAT passes the conformance suite. Done: `ExFatFs<D, T: NodeTable = FixedTable<64>, C: Clock = NoClock>` replaces the preview, a sibling of `FatFs` generated for `sync`, `r#async` and `async_send` in `hadris_fat::exfat`, with `check`, `check_with` and `format` beside it; it implements the writable `FsDriver` through `impl_fs_driver!` with `parent`, `open_node`, `close_node` and `publish_node`, needs no allocator (one device block of at most 4096 bytes and a fixed up-case page index), and passes the contract kit in all three modes. It reads contiguous and chained allocations, fragmented allocation bitmaps and up-case tables, entry sets that cross clusters and benign secondary entries, which it keeps across renames; it creates, removes, renames (with `NO_REPLACE`), writes, sets lengths, attributes and the four times, grows directories, sets and removes the label, and keeps `VolumeDirty` and `PercentInUse`. Mounts check the boot checksum and fall back to a valid backup boot region, read-only. On TexFAT volumes it follows `ActiveFat` and writes both FATs and both bitmaps, and `FormatOptions::with_fat_count(2)` formats them, the V3 form of the preview's `fat_count`. `FormatOptions` and `MountOptions` have private fields and `with_*` setters; the boot sector, entry layouts and constants are in `exfat::raw`; errors are `Error<E>` and `exfat::Error` is gone, with the preview's `ExFatVolume`, `ExFatInfo`, `ExFatFileEntry`, readers, writers and `format_exfat`. `check_with` reports boot region, bitmap, chain, entry set, name, up-case and tree findings with a windowed bitmap and a fixed stack. The conformance suite has an independent exFAT oracle and runs the FAT operation model, seeded traces, rejection scenarios, the data region, long extent and directory growth limits, and native qualification (exfatprogs `mkfs.exfat`/`fsck.exfat`, macOS `newfs_exfat`/`fsck_exfat` and a macOS kernel mount) on 512-byte, 4 KiB, 32 KiB and TexFAT cases. `hadris_io::legacy` is deleted, `hadris_io::SeekFrom` is a Hadris type with `resolve`, and `embedded-io` is an optional feature. `hadris-block`'s `OpenVolume` opens exFAT with `as_exfat`, `as_exfat_mut` and `into_exfat`; the fuzz targets `exfat_read` and `exfat_ops` drive `ExFatFs`. Decisions: exFAT is promoted to stable in 3.0 (Q5); writes allocate FAT chains only, and a contiguous allocation becomes a chain when it grows; node ids are the File entry's byte offset divided by 32; `set_len` keeps `ValidDataLength` and reads past it return zeros; a volume mounted from its backup boot region, or whose up-case table fails its checksum, is read-only, and `check` reports both; names ending in a dot or a space are refused; removal clears bitmap bits and leaves FAT entries; the `SeekFrom` enum is `#[non_exhaustive]`. Not done: TexFAT transactions, repair, and the `analysis` module, all additive in 3.x; exfatprogs 1.2.9 reads bitmaps and up-case tables as contiguous and refuses TexFAT, so those images are qualified with macOS `fsck_exfat` only.
13. **CLIs, examples, fuzz targets, docs.** Rewrite on the public API only. Anything the CLI verifier still needs goes in `raw`, which tests that `raw` is enough. `fs_dump` becomes one generic walk over `FileSystem`. Done: the five CLIs use only the public V3 API, and their host work goes through `extract_to_host`, `import_from_host` and `Tree::from_fs`. `hadris-fat` supports exFAT in every command (`info`, `ls`, `tree`, `cat`, `extract`, `verify`, `create --fat-type exfat`) by dispatching over `FatFs` and `ExFatFs`; its verifier and `info` read `raw`, which gained `RawDirEntry`, `RawLfnEntry` and the directory entry constants, and the codec now decodes through them; `hadris-common`'s endian integers gained inherent `new`, `get` and `set`, so `raw` fields are usable without importing the internal `Endian` trait. Every CLI accepts `list` for `ls` and `check` for `verify`, `extract -o` defaults to the current directory, `cat` streams, and the ISO, UDF, cpio and FAT CLIs have command tests. `fs_dump` is one generic walk over `FileSystem`, reaching each driver through `Volume::new` (ISO through `into_view`); every format has a read target and FAT and exFAT have ops targets, with small seeds added for cpio, exFAT and UDF. The `volume-list` example opens any image through `hadris_block::sync::OpenVolume`, and the READMEs, rustdoc, CONTRIBUTING, `tests/README.md` and the website describe the V3 API. Fixed on the way: the Joliet and enhanced-tree escape sequence field is zero padded (ECMA-119 8.5.6), which bsdtar needs to read hybrid images, and cpio extraction skips the `.` entry GNU and BSD cpio write. Decisions: the FAT CLI detects exFAT through `exfat::raw::BootSector` rather than depending on `hadris-block`; FAT `info` reads the raw BPB rather than growing `FatFs` accessors; `extract PATH` writes `<output>/<name>` in every CLI, and ISO device nodes now fail extraction as `extract_to_host` refuses them; `verify` exits non-zero on findings; `list` and `check` are aliases, not renames; the presentation helpers stay in each CLI rather than in a shared crate that would need publishing; UDF `ls -a` shows `.` and `..`; the escape padding fix changes Joliet and enhanced-tree output against 2.4, so step 9's byte identity holds only for option sets without them; UDF seeds are empty `mkudffs` volumes and larger Hadris images stay generated by `gen-seeds`; the read targets keep format-specific walks beside the generic `fs_dump`. Not done: the `analysis` module (the FAT CLI computes fragmentation itself), a lookup that returns the stored name (4.14; the CLIs scan the parent directory), `FatFs::volume_id` to match exFAT, TexFAT from the CLI, bsdtar and 7-Zip interop in CI, and hidden `# ` lines in README snippets; version strings stay 2.4.0 until step 14, and website links point at `next` and crate README links at the stable site until the merge.
14. **3.0.0-rc.1.** CI guardrails become blocking. Migration guide (`docs/hadris-3.0.0-migration.md`) with a V2 to V3 symbol table.

`hadris-vfs` and the 3.x feature items follow 3.0.0.

---

## 7. Open questions

**Q1. Lock placement.** Resolved. Four variants were prototyped on
`feat/v3-lock-prototype` under `experiments/lock-placement`: an external
wrapper over `&mut self` (A), a lock inside each format (B), a `&mut` driver
trait plus a `&self` trait with a shared `Volume` (C), and C with opt-in tiers
and device-typed errors (D). V3 takes D: 4.1 to 4.4, 4.6, 4.12 and 4.13
describe it, and scenarios S1 to S16 are its acceptance tests.

**Q2 revised 2026-09-24:** the shared tier keeps two modes, `sync` and `async`, where `async` is the `Send` mode described below; non-`Send` async stays in `hadris-io`, `hadris-storage` and the embedded API (4.15).

**Q2. `Send` futures.** Resolved: V3 ships the `async_send` mode below. `async fn` in traits does not let a generic caller
require `Send` futures, so tokio code that spawns over a generic
`F: FileSystem` does not compile (E2 baseline: 25 errors, "`<F as
FileSystem>::sync` is an `async fn` in trait, which does not automatically
imply that its future is `Send`"). E2 tried every option:

| Option | Result |
|---|---|
| `trait_variant` 0.1.3 | Does not compile: default bodies are not wrapped in `async move`, and its blanket impl conflicts with the `&`, `&mut`, `Box` and `&F` forwarding impls |
| The same pattern by hand | A format crate can implement the `Send` variant or the local one for `FatFs<D>`, not both, so "`Send` when the device is" cannot be written |
| `+ Send` in the async traits | Works, but rejects every non-`Send` device: an `Rc` device, a `RefCell` volume, any embedded-io-async adapter |
| Return type notation | Unstable (E0658 on 1.98.1). On nightly a blanket `SendFileSystem` alias works with no trait changes |
| Spawn concrete types only | Works after the `Access` fix (4.3); a generic function that calls `spawn` cannot compile |
| **A third mode, `async_send`** | Works. The same source is generated a third time by a `send_async!` macro that turns each trait `async fn` into `fn -> impl Future + Send` and adds `Send`/`Sync` supertraits. Users write `F: FileSystem + 'static` with no `Send` bounds; format crates write nothing |

The mode is `async_send` in each crate, behind an additive `async-send`
feature. Costs: the code compiles a third time, `send_async!` is
about 240 lines in `hadris-macros`, the mode needs its own lock trait
(`LockKind::Lock<T: Send>`, an opaque guard) and a `MaybeSend` marker
(`Volume<F: MaybeSend, K>`), `Rc` impls are left out of it, and R11 parity
covers three modes. When return type notation is stable, `SendFileSystem`
aliases over the `async` traits replace the mode as a deprecation, not a
break, because the `async` traits never change. Decided: add the mode.

**Q3. Removing an open file.** Resolved, revised in step 6. POSIX semantics
(entry gone, clusters freed at the last close) need orphan tracking, and a
crash leaves lost clusters that fsck has to reclaim, so removing the last
name of an open node fails with `ErrorKind::Busy`. The first answer made any
pin block removal; the FUSE prototype showed that a kernel pins every cached
name, so every `rm` failed. Now a node is open between `open_node` and
`close_node` (as `File` and `OpenFile` hold it), a pinned node that is only
looked up is removed, and its id answers `NotFound` until its last `forget`.
Orphans can come in 3.x without a break, since they only turn `Busy` into a
success.

**Q4. MSRV.** `core::error::Error` needs 1.81 and `async fn` in traits 1.75, so
the current 1.88 works. Raise it only if `dyn`-compatible async traits
stabilise before 3.0.

**Q5. exFAT and NTFS stability.** Resolved for exFAT: the user decided on
2026-09-24 to promote exFAT in 3.0 because it passes the suite. `ExFatFs` is
stable in `hadris_fat::exfat` and `unstable-exfat` is removed. NTFS stays in
`unstable` through 3.0.

**Q6. Name encoding in `Capabilities`.** FAT short names depend on the OEM code
page and long names are UTF-16. ISO primary names are d-characters, Joliet is
UCS-2. The trait takes bytes. Does `Capabilities` describe the charset
precisely enough for a VFS to translate names, or does each format also need
a `NameCodec`? Still open: `hadris-fs` ships `Capabilities::name_charset()`
returning a non-exhaustive `NameCharset` (`Bytes`, `Utf8`, `Ucs2`, `Utf16`,
`DCharacters`, `OemCodePage`) as the interim answer.

**Q8 revised 2026-09-24:** the node table leaves the public API; `FatFs<D>` uses an internal heap table and firmware uses the embedded API (4.15).

**Q8. FAT node table capacity.** Resolved: a `NodeTable` type parameter. Without `alloc` the table is a fixed array
(E1): `FatFs<D, const N: usize = 64>`, full table gives `LimitExceeded`, and
`FsDriver`, `Volume` and handles never see `N`. A default const parameter is
not used for inference, so other sizes need `FatFs::<_, 8>::open_sized(dev)`.
Because no feature may switch the table, std users get the same 64-node limit,
which a FUSE mount (the kernel holds inodes until it forgets them) can exceed.
Options: keep the const generic and raise the default; make the table a type
parameter with a fixed default and a growable `HeapTable` under `alloc`
(`FatFs<D, T: NodeTable = FixedTable<64>>`); or evict unpinned entries so
the capacity only bounds open nodes. Decided: the type parameter, with
`hadris-vfs` and the FUSE adapter naming `HeapTable`. `NodeTable`,
`FixedTable` and `HeapTable` are public in `hadris-fs`, so FAT, exFAT and ISO
share them and users can supply their own, such as an evicting table. The
trait's contract lets a table keep or drop an entry once its last pin goes,
which is what eviction needs.

**Q7. Driver type names.** Resolved: `<Format>Fs`. `Volume<F, K>` is now the sharing wrapper, which
makes `FatVolume`, `UdfVolume` and `NtfsVolume` read as if they were already
shared. The prototype named the FAT driver `FatFs`. Options: rename every
driver to `<Format>Fs`, or rename the wrapper (`Shared<F, K>`). Decided:
`<Format>Fs` for drivers (`FatFs`, `ExFatFs`, `IsoFs`, `UdfFs`, `NtfsFs`),
since `Volume` is what most users type.

**Q9. Layer boundaries.** Resolved 2026-09-24 by the user, in two rounds.
First: host volumes wrap `Volume` with `StdMutex`, `host::Error` replaces
`AnyError`, the host FAT code page is `Cp437`, and profiles are modules. Then,
after an architecture pass: raw layers are separate `hadris-<fmt>-raw` crates;
`FatFs` and `ExFatFs` collapse to one type parameter and require `alloc`;
the shared tier ships `sync` and `Send` `async`; firmware gets a separate
embedded API with exFAT read-only in 3.0. Async-only with a `block_on` sync
wrapper was measured and rejected (4.15).
