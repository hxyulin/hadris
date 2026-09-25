# Hadris V3 API design

Status: accepted design, 2026-09-24. Sections 2 to 5 describe the API
accepted through the action catalog ([`v3/actions.md`](v3/actions.md)) and
the API prototype, and are the specification. Sections 4.15 to 4.18 record
how those decisions were reached: the layering pass, the review decisions,
the workspace simplifications and the four prototype passes with the
post-pass decisions A, B and C. Earlier rounds, the lock-placement prototype
(`experiments/lock-placement`) and the step 6 trait review
([`v3-trait-review.md`](v3-trait-review.md)), are recorded in sections 6
and 7.

V3 is the release where the public shapes stop moving. V2 kept breaking semver
inside minor releases (#83, #93, #94) or hid new work behind `unstable-*` flags
that change the shape of public types (#111, #115). V3 fixes the extension
points first. After 3.0.0, new features land in 3.x minors because the types,
traits and error kinds they need already exist.

That gives a rule for scope. Every feature listed in [section 5](#5-per-crate-changes)
is V3 work. Every one of them must have its public shape in 3.0.0. The
implementation of a feature can follow in 3.x only if it fits a shape that
already shipped. NTFS and UDF write are the obvious examples: the write
methods of `FileSystem` exist in 3.0 with `ReadOnly` defaults, both drivers
report themselves read-only through `Capabilities`, and write support arrives
later without a break.

V3 serves three kinds of users, and none of them is optional:

- OS kernels, VFS layers and FUSE (#18, the stlankes PRs #83 #84 #89) need
  `Send` drivers, stable node IDs, positional I/O, resumable directory reads,
  and a cache they can turn off.
- Bootloaders and firmware need no-alloc reads and writes, small code and
  stack, and async on embedded executors.
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
  let opts = IsoOptions::new()
      .with_level(IsoLevel::L3)
      .with_joliet()
      .with_rock_ridge()
      .with_hybrid(Hybrid::gpt());
  ```

- **Results and entries** expose getters only. Construction is crate-private.
- **Plain value types** that are complete by definition (`Guid`, `DateTime`, `BlockIndex`, `NodeId`) may be exhaustive, but fields stay private with `const fn` constructors and accessors.

### R3. No `cfg` on variants, fields or trait bounds of public types

A feature may add items (types, functions, modules, impls). It never changes
the shape of an existing item. `ErrorKind::NoSpace` exists in every build even
though only a `write` build produces it.

Previews use a separate module behind an `unstable-*` feature, for example
`hadris::ntfs` behind `unstable-ntfs`, never cfg-gated variants.
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
`hadris_iso::sync::IsoFs` or `hadris_iso::r#async::IsoFs`.

### R6. No panics on disk data or user input

Library code returns errors. `expect` and `unwrap` are allowed only on
invariants the same function established. No feature turns an error into a
panic, so `dirty-file-panic` is removed. Only fallible forms exist; there are
no `try_*` twins.

### R7. Iterators fuse after an error

Any iterator yielding `Result` yields at most one `Err` and then `None`, and
ends after its first `None` (`FusedIterator`). The same holds for the
`next`-style methods that stand in for iterators: async `ReadDir::next_entry`,
`Walk::next` and the cpio reader's `next_entry`. A shared internal adapter
implements it for iterators, and each iterator and method has a test for it.

### R8. Sizes and offsets are `u64`

File sizes, offsets and byte counts that can exceed 4 GiB are `u64` in every
crate, including 32-bit targets. On-disk narrowing uses checked conversion and
returns `ErrorKind::LimitExceeded`.

### R9. No bool parameters in public functions

Use an enum, a flags type or an options struct. Public functions with more
than four parameters take a struct.

### R10. Traits can grow

Public traits that users implement (`FileSystem`, `BlockDevice` in each mode,
`Read`, `Write`, `Seek`, `Clock`, `CodePage`) are not sealed. Methods added in
3.x must have a default body. For filesystem operations the default returns
`ErrorKind::Unsupported` (`ReadOnly` for writes) or composes existing
methods, and `Capabilities` gains a matching flag that defaults to off.
Traits that only Hadris implements are sealed.

Growing `FileSystem` has two more rules, both stated in the trait docs:

- There is one filesystem trait, so a new method is written once. The only
  Hadris type that implements it by delegation is `AnyFs`, and a test reads
  the method list from the trait definition and fails until `AnyFs` forwards
  a new method. User types that wrap a driver keep the default until they
  forward it themselves; the docs say so.
- There is no macro that generates the impl. `impl_fs_driver!` is dropped
  (post-pass decision A): the write methods have defaults, so a read-only
  format implements the read half and there is nothing left to generate.

### R11. CI enforces it

- `cargo semver-checks` on every PR against the latest 3.x release (after 3.0.0).
- The public-API snapshot runs with all non-`unstable` features on, and a second run with them off must produce a subset. That proves R3.
- A lint script rejects public enums without `#[non_exhaustive]` outside `raw`.
- A sync/async parity check diffs the public item lists of the two modules and lists the intended differences (4.8).

### R12. Crates version independently after 3.0.0

Every crate ships 3.0.0 together. After that each crate has its own
version and bumps its major only for its own breaking changes; there is no
lockstep release.

- A crate exposes another Hadris crate's types only where that crate is a
  deliberate public dependency. `hadris-io`, `hadris-storage` and
  `hadris-fs` are the stable base that every format crate exposes. Any
  other public dependency is named in section 5 (the ISO 9660 and UDF
  bridge writer takes `hadris-iso` options), and a major version of that
  dependency is a major version of the crate that exposes it.
- The umbrella `hadris` re-exports the format crates, so it bumps its
  major whenever a re-exported crate does.
- Raw crates (`hadris-<fmt>-raw`) version separately and are never
  re-exported wholesale. A format crate re-exports only the raw items its
  own signatures use ([Q13](#7-open-questions)).
- No new wholesale re-export of one crate by another is added.

Per-crate tags and the release workflow change in step 14.

---

## 3. Layers

```
hadris-io          byte streams: Read and Write in sync, r#async (Send) and local
                   (non-Send); StdIo with std; FromEmbedded with embedded-io
hadris-storage     BlockDevice in sync, r#async and local; Partition, Vec<u8> and
                   the other devices and adapters
hadris-fs          vocabulary, Error<E> and PathError, MountOptions, the FileSystem
                   trait, Volume<F> with File and ReadDir, Walk, Tree and Report,
                   copy_tree and read_tree
hadris-<fmt>-raw   I/O-free codecs and raw::io device primitives, versioned apart
                   from the format crate
format crates      drivers (FatFs, ExFatFs, IsoFs, UdfFs, NtfsFs) that implement
                   FileSystem, inherent extras, format, check, plan and write; FAT
                   and exFAT also have an embedded API
hadris             umbrella: flat re-exports, detect, open and AnyFs, and the host
                   module
hadris-cli         the hadris binary, versioned apart from the library
hadris-vfs         3.x, std only: type erasure, a mount table, FUSE
```

Two ideas carry the design.

**One core trait, native extras.** `FileSystem` (4.3) is the closed set of
operations every mounted filesystem has, taken from the catalog's operation
list (section 2 of [the catalog](v3/actions.md)): look up a name, list a
directory, read and write at an offset, change metadata, create and remove.
Generic code, the conformance suite, FUSE and kernel integrations target it.
Everything else stays with the format as inherent methods with the same name
on every driver (`info`, `extents`, `records`, `read_raw`) or as free
functions in the format module (`format`, `check`, `plan`, `write`). The
trait never grows a method only one format can answer.

The trait is the wrong abstraction for three jobs, and V3 does not force it on
them:

- Image writers (ISO, UDF, the ISO and UDF bridge, cpio, and FAT and exFAT through format plus `copy_tree`) take a whole `Tree` and produce an image. They share `Tree`, `plan`, `write` and `Report` (4.7), not a trait.
- Streaming archives (cpio, later tar) are forward-only. They get an entry reader, not random access.
- Tools (fsck, analysis, the ISO verifier) use `check` and the raw crates.

**Every layer above the driver is opt-in, and every tier does every job.** A
driver takes `&mut self`, owns its device and holds no lock. `Volume<F>` adds
one lock, paths named after `std::fs` and any number of handles. The `host`
module adds the host's clock, time zone and files. Firmware without an
allocator uses the embedded API, which is a separate set of types built on
the raw layer. Each tier adds only its own cost, and none of them is required
to reach a feature: the bare driver can open, list, walk and copy, the shared
tier can reach format extras through `vol.lock()`.

One driver type serving firmware and hosted code was tried first. Firmware
wants fixed node slots, 512-byte buffers, ASCII case folding and two-byte
errors; hosted code wants unbounded tables, Unicode, paths in errors and no
visible generics. Features may not switch behaviour (R3), so one type cannot
choose per build, and type-alias profiles over one generic engine left the
firmware generics on the shared type. Section 4.15 records the change.

---

## 4. Cross-cutting design

### 4.1 `hadris-io`

**Traits.** `Read` and `Write` follow the embedded-io shape: each reports its
own error through an associated type. Sync and async come from one source
file, so they cannot drift. Each trait exists in `sync`, `r#async` (futures
are `Send`) and `local` (futures need not be `Send`, for single-threaded
executors such as embassy).

```rust
pub trait ErrorType {
    type Error: core::error::Error + Send + Sync + 'static;
}

pub trait Read: ErrorType {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ExactError<Self::Error>> { .. }
}
pub trait Write: ErrorType { /* write, write_all, flush */ }
pub trait Seek: ErrorType { /* seek */ }

pub enum ExactError<E> { UnexpectedEof, WriteZero, Io(E) }
```

- The bound is the whole error contract. A kernel writes its own enum with `Display` and an empty `impl core::error::Error`. std devices use `std::io::Error`. embedded-io errors pass through unchanged. There is no Hadris error trait to implement. The `local` traits drop `Send + Sync` from the bound.
- `Send + Sync` lets code erase any device error into `PathError` or `std::io::Error` (4.6) with no extra where-clauses. It rules out errors that hold an `Rc` or a raw pointer; such a device wraps them, or uses the `local` traits.
- `&mut T` implements each trait when `T` does, and so does `Box<T>` with `alloc`. `&[u8]` is a reader and `Vec<u8>` a writer.
- `FromEmbedded<T>` adapts an `embedded-io` or `embedded-io-async` stream. Its error is `T::Error`, unwrapped. It is the only item that names embedded-io, so the dependency sits behind an `embedded-io` feature.
- With `std`, `into_std_error(e)` converts any device error to `std::io::Error`, returning an `io::Error` as itself. `ExactError<E>` converts with `?`.
- With `std`, `StdIo<T>` adapts any `std::io` stream (a pipe, stdin, a decompressor) for the cpio reader and writers. A host image file does not need it: `host::FileDevice` is a `BlockDevice` (4.2), and sync `File` handles implement `std::io` directly (4.3).

A blanket `impl<T: embedded_io::Read> Read for T` was the first plan. It fails
on coherence. A `&mut T` impl overlaps it, and without that impl, generic code
holding `R: Read` cannot pass `&mut R` on, because `&mut R` is not an
`embedded_io::Read`. Explicit adapters cost one wrapper at the edge and remove
the whole class of problem. This addresses #16 and #18.

```rust
let file = std::fs::File::options().read(true).write(true).open("disk.img")?;
let fs = FatFs::mount(host::FileDevice::new(file)?, host::mount_options())?;
```

**The Hadris error lives here** ([Q10](#7-open-questions)). `Error<E>`,
`ErrorKind`, `Location`, `DetailCode`, `Errno` and `FsResult` (4.6) are
defined in `hadris-io`, the lowest crate, because block devices return
`Error<E>` (4.2) and `hadris-fs` depends on `hadris-storage`. `hadris-fs`
and the `hadris` root re-export them, so users write `hadris_fs::Error` or
`hadris::Error`.

**Errors are the device's own.** The earlier draft erased every device error
into one `hadris_io::Error`. That lost the device error without `alloc`,
which is exactly the kernel case, and it made a std user unwrap a Hadris
error to get the `io::Error` back. With an associated error the device error
survives unchanged in every build, and 4.6 describes how code that mixes
devices erases it on purpose. Step 2 of the migration built the erased error;
step 4 replaced it.

**No public byte source.** Writer input is `Content` (4.7): bytes, a host
file, or content read lazily from a mounted volume or an existing ISO session.
The lazy kinds are private. The earlier `ByteSource` trait is not public in
3.0, because no 3.0 action needs content supplied by the user; `Content` is
opaque, so a `Content::source` constructor can be added in 3.x.

### 4.2 `hadris-storage`

Filesystems read from a `BlockDevice`, not a byte stream. That fixes three V2
problems: the cache can sit under every format instead of inside FAT, 4Kn
devices and 2048-byte optical media stop being special cases, and a partition
is just another device.

```rust
pub trait BlockDevice {
    type Error: core::error::Error + Send + Sync + 'static;
    fn block_size(&self) -> u32;
    fn block_count(&self) -> u64;
    async fn read_blocks(&mut self, first: u64, buf: &mut [u8]) -> Result<(), Error<Self::Error>>;

    // Defaulted, so a read-only device implements only the lines above.
    fn max_block_count(&self) -> u64 { self.block_count() }
    fn disk_offset(&self) -> u64 { 0 }
    fn writable(&self) -> bool { false }
    async fn write_blocks(&mut self, first: u64, buf: &[u8]) -> Result<(), Error<Self::Error>> { .. }
    async fn flush(&mut self) -> Result<(), Error<Self::Error>> { .. }
}
```

`async fn` here means "written once, generated for every mode" (4.8). The
`r#async` trait has a `Send` supertrait and `Send` futures; the `local` trait
has neither and is what the embedded async API takes.

- One trait with defaulted writes, not separate read and write device traits. A read-only device must still mount (IO-RO-01), and in 3.x `UdfFs<D: BlockDevice>` gains writes with no new bound (NF-STABLE-02).
- `read_blocks`, `write_blocks` and `flush` return the crate-wide `Error<E>` (4.6), not a separate `WriteError<E>` (post-pass decision B, S5; [Q11](#7-open-questions) for reads). A device that refuses a write returns kind `ReadOnly`; a device failure is `Error::device(e, message)`. `flush` returns `Error<E>` too, because a write-back device writes there.
- `max_block_count` lets a writer grow its output: `Vec<u8>` and `host::FileDevice` report more than `block_count` and grow when written past the end (IO-GROW-01). Writers check the planned size against it before writing anything.
- `disk_offset` is the byte offset of block 0 within the disk the device is a window of. `Partition` adds its start. Formatting records it as the FAT hidden sectors or the exFAT partition offset (VOL-FORMAT-03).

**Read-only devices.** `writable()` says whether the device accepts writes at
all; a device that returns false gets a read-only mount, and opening a file
for writing then fails with `ReadOnly` at open time. A device that says true
may still refuse later: an SD card's lock switch moves while mounted, a USB
stick can be write-protected at any time, and a write blocker refuses
silently. Probing with a test write is worse: it writes, it wears flash, and
write-once media keep it. So each write also answers for itself:

- A refused write returns kind `ReadOnly`, and the mount turns read-only and stays consistent (IO-RO-02). `capabilities().writable()` is false from then on, and the driver rejects writes before touching the device or its own tables.
- Users who know up front mount read-only (`MountOptions::new().read_only()`), which never calls `write_blocks`, not even for dirty flags or FSInfo.
- Format crates leave their in-memory state unchanged when a write is refused. That is the rule 4.3 already sets for every failed operation, so it adds no new contract.

Provided devices and adapters:

| Type | Purpose |
|---|---|
| `impl BlockDevice for &mut D`, `Box<D>` | Borrow or box a device instead of moving it in. |
| `Vec<u8>` | With `alloc`, an in-memory image with 512-byte blocks that grows when written past its end. Error `Infallible`. |
| `Partition<D>` | A byte window of `D` (an MBR or GPT partition, or a hybrid ISO's partition). Offset and length are multiples of the device block size; mount checks them. Reports its start through `disk_offset`. Replaces `Slice` and V2's `PartitionView`. |
| `host::FileDevice` | A host image file or block device (4.15). `open(path)` is read-only; `new(file)` takes a `File` opened by the caller, measures its size by seeking to the end, and can fail, since host block devices report a size of 0 in their metadata. It tracks whether the file was opened for writing. `std::fs::File` itself is not a device, because `block_count` cannot fail. |
| `StreamDevice<T>` | Any `Read + Seek + Write` byte stream, with a block size the caller picks. `StreamDevice<ReadOnly<T>>` needs only `Read + Seek`. The migration path for every V2 user. |
| `MemDevice<B>` | `&[u8]` (read-only), `&mut [u8]`, `[u8; N]`, and `Box<[u8]>` with `alloc`. For tests and fixed in-memory images. |
| `Cache<D>` | Write-back LRU over whole blocks, `alloc` only. Explicit `flush`, or `finish` to flush and return the device. Capacity set at construction. Works in every mode. Kernels skip it. The first write goes straight to the device, so a read-only device refuses at once instead of at a later flush. Requests of at least `capacity` blocks bypass it (reads still see dirty cached blocks), and a flush writes each run of consecutive dirty blocks in one call. |
| `ByteView<D>` | Byte-granular `read_at` and `write_at` with read-modify-write for partial blocks. Used by format crates for records that straddle blocks. Without `alloc` its scratch buffer caps the block size at 4096. |

Adapters keep `D::Error` as their error type. `StorageError`, `OutOfRange` and
`WriteError` are gone (S5): a request an adapter refuses itself, such as a
read or write past the end of a `Partition` or a `MemDevice`, is an
`Error<E>` of kind `InvalidInput` with the block as its `Location`, and
never reaches the device (Q11). `MemDevice` and `Vec<u8>` have the device
error `Infallible`.

A filesystem sector can be larger than the device block (FAT 4096-byte
sectors on a 512-byte image) but not smaller unless the device is a
`StreamDevice`, which accepts any block size. The shared tier accepts device
blocks of 512 to 4096 bytes (ISO: 512 to 2048) and refuses others with
`ErrorKind::Unsupported`; the embedded API takes 512-byte blocks only.

cpio stays on `Read` and `Write` streams, since it must work on pipes.

`hadris-storage` stays a separate crate below `hadris-fs`, so a device
implementation depends on devices only. Because the block methods return
`Error<E>`, that type is defined in `hadris-io`, below this crate
([Q10](#7-open-questions)).

### 4.3 `hadris-fs`: shared vocabulary and traits

New crate. It takes the useful parts of `hadris-common` and `hadris-path`.
Everything here is mode-independent and needs no `alloc` unless stated; the
`FileSystem` trait, `Volume` and its handles are generated per mode.

**Vocabulary.**

| Type | Notes |
|---|---|
| `NodeId` | Non-zero `u64` (`new` returns `Option`, `get`). Stable while the node is pinned (4.5), the same for every hard-link name. Maps directly to FUSE `ino` and kernel inode numbers; the root may have any value, and a FUSE layer maps it to 1. |
| `Name` | Unsized bytes, `Name::new(&s)`, `as_bytes`, fallible `to_str`. Names need not be UTF-8. Drivers reject empty names, `.`, `..`, and names containing `/` or NUL with `InvalidInput`. Paths are `/`-separated bytes passed as `impl AsRef<[u8]>`; there is no public path type. |
| `FileType` | `File`, `Dir`, `Symlink`, `CharDevice`, `BlockDevice`, `Fifo`, `Socket`. Non-exhaustive. One definition for every crate. |
| `Metadata` | Getters only, `Copy`: `file_type`, `len`, `allocated`, `created`, `modified`, `accessed`, `changed` (each `Option<DateTime>`), `permissions`, `owner` (`Option<Owner>`), `attributes`, `nlink` (1 on a directory means not counted), `generation`, `device` (`Option<DeviceNumber>`). A symlink's `len` is its target length. Drivers build it with `Metadata::new(type, permissions)` and `with_*`. Per-format metadata (FAT attribute bits beyond `Attributes`, NTFS security descriptors) stays native. |
| `SetAttr` | Optional changes: `with_accessed`, `with_modified`, `with_created`, `with_permissions`, `with_owner`, `with_attributes`. Also the initial attributes of `create` and `mkdir`, and of a tree node (4.7). |
| `DateTime` | Private fields: seconds since 1970, nanoseconds, optional UTC offset in minutes (`from_unix`, `with_utc_offset`). Every format converts to and from its own encoding in its raw crate. |
| `Permissions`, `Owner`, `Attributes`, `DeviceNumber` | Plain values. `Permissions` holds POSIX mode bits (D11). `Attributes` is DOS-style flags with `NONE`, `READ_ONLY`, `HIDDEN`, `SYSTEM`, `ARCHIVE`, `contains`, `union` and `without`. |
| `DirCursor` | A `Copy` position in a listing. `START` is 0; drivers return raw values up to `DirCursor::MAX_RAW` (`2^63 - 16`), so a cursor fits `off_t` with room for the `.` and `..` a FUSE layer adds. |
| `DirEntry` | One listing entry: `name`, `node`, `metadata` and `next_cursor`. It owns its name inline (768 bytes), so node and path listings share one type with no allocation (D9). |
| `Capabilities` | `writable`, `symlinks`, `hard_links`, `case` (`CaseRule`: `Sensitive`, `InsensitivePreserving`, `Insensitive`), `charset` (`Charset`: `Bytes`, `Unicode`), `max_name_bytes` in UTF-8 bytes (FUSE `namemax`), and `stores(Field) -> Stored` (`No`, `Partial`, `Yes`) for each time, permissions, owner, attributes and device. |
| `FsStats` | Block size, total, free and used blocks. |
| `OpenMode`, `OpenOptions` | `OpenMode::{Read, Write}` for the node-level `open`; `OpenOptions` (`new`, `read`, `write`, `append`, `truncate`, `create`, `create_new`) for paths, which makes overwrite semantics explicit (#90, #91). |
| `RenameMode`, `Resolve`, `SeekFrom` | `RenameMode::{Replace, NoReplace}`, the path policy of 4.12, and a Hadris `SeekFrom`. All non-exhaustive. |
| `MountOptions` | One options type for every format (4.15): `read_only`, `with_clock`, `with_utc_offset`, `with_code_page`, `with_node_limit`, `backup_boot`. Formats ignore what does not apply. `MountOptions::new()` is read-write where possible, UTC, CP437 and a fixed clock at the FAT epoch, the same on every target; `host::mount_options()` is the host default (post-pass decision C). |
| `Clock`, `SystemClock` | `fn now(&self) -> DateTime`, `Send + Sync`, stored as `&'static dyn Clock`. `SystemClock` with `std`. |
| `CodePage`, `Ascii`, `Cp437` | The OEM code page of FAT 8.3 names, stored as `&'static dyn CodePage`. `Cp437` is the default in every tier. `Ascii` reads a byte `b` above `0x7F` as `U+F700 + b`, so short names stay distinct and look up by the name listed. |
| `Extent` | A byte range on the device (`offset`, `len`), its offset within the file (`file_offset`) and an `unwritten` flag. Used by writer reports, file maps (`extents`) and record locations (`records`). |
| `WalkFrame` | One level of a walk stack that the caller lends to `Walk::with_stack`. |

`Error<E>`, `ErrorKind`, `Location`, `Errno`, `MountError` and `PathError`
are in 4.6; `Tree`, `Node`, `Content`, `Report` and `Warning` in 4.7;
`Finding`, `Severity` and `CheckReport` in 4.15.

The constructors a driver needs (`Error::new`, `Error::device`,
`Metadata::new`, `DirEntry::new`, `Capabilities::new`, `FsStats::new`,
`DeviceNumber::new`) are public, since the trait is not sealed and
third-party and test adapters build these values too.

**The trait.** One node trait on `&mut self`, generated for `sync` and
`r#async`. The sync form, with `R<T>` standing for
`FsResult<T, Self::DeviceError>`:

```rust
pub trait FileSystem {
    type DeviceError: core::error::Error + Send + Sync + 'static;

    // volume
    fn capabilities(&self) -> Capabilities;
    fn root(&self) -> NodeId;
    fn statfs(&mut self) -> R<FsStats>;
    fn label<'b>(&mut self, buf: &'b mut [u8]) -> R<Option<&'b str>>;   // 384 bytes suffice

    // names and nodes
    fn lookup(&mut self, dir: NodeId, name: &Name) -> R<NodeId>;
    fn forget(&mut self, node: NodeId, count: u64);
    fn parent(&mut self, dir: NodeId) -> R<NodeId>;
    fn resolve(&mut self, path: &[u8], how: Resolve) -> R<NodeId> { .. }
    fn stat(&mut self, node: NodeId) -> R<Metadata>;
    fn readdir(&mut self, dir: NodeId, from: DirCursor) -> R<Option<DirEntry>>;
    fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> R<&'b [u8]>;

    // data
    fn open(&mut self, node: NodeId, mode: OpenMode) -> R<()>;
    fn close(&mut self, node: NodeId) -> R<()>;
    fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> R<usize>;

    // write half; every method defaults to ErrorKind::ReadOnly
    fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> R<()>;
    fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> R<usize>;
    fn truncate(&mut self, node: NodeId, len: u64) -> R<()>;
    fn fsync(&mut self, node: NodeId) -> R<()>;
    fn create(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> R<NodeId>;
    fn mkdir(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> R<NodeId>;
    fn unlink(&mut self, dir: NodeId, name: &Name) -> R<()>;
    fn rmdir(&mut self, dir: NodeId, name: &Name) -> R<()>;
    fn rename(&mut self, from_dir: NodeId, from: &Name, to_dir: NodeId, to: &Name,
        mode: RenameMode) -> R<()>;
    fn sync(&mut self) -> R<()>;
}
```

The async form returns `impl Future<Output = ..> + Send` from each method and
has a `Send` supertrait; the sync form is generated from it (4.8). The sync
trait is dyn-compatible. `FatFs<D>`, `ExFatFs<D>`, `IsoFs<D>`, `UdfFs<D>`,
`NtfsFs<D>` and `AnyFs<D>` implement it. Each driver has
`mount(dev, MountOptions) -> Result<Self, MountError<D, D::Error>>` and
`unmount(self) -> Result<D, MountError<..>>`, which syncs first and gives the
device back either way.

The contract, stated in the trait docs:

- `lookup`, `parent`, `resolve`, `create` and `mkdir` pin the node they return; `forget(node, count)` drops `count` pins (FUSE `forget(nlookup)`). The root is always pinned. `forget` never fails, never blocks and does no I/O.
- A pin never blocks a removal. A removed pinned node keeps its id until its last `forget`, and every other method answers `NotFound` for it. `open` and `close` bracket reads and writes; removing the last name of an open node fails with `Busy` (Q3). The split exists because a FUSE kernel pins every cached name, so pins that blocked removal made every `rm` fail (step 6).
- `open(node, mode)` fails with `IsADirectory`, with `Symlink` for a symlink, and with `ReadOnly` for `OpenMode::Write` on a read-only mount. The bare tier trusts the caller's open mode after that; positions and append live in the caller or in `File`.
- `close` publishes the node's size and times. `fsync` makes one node durable and flushes the device. `sync` writes every piece of cached metadata to every copy (FSInfo, dirty FAT sectors, directory entries, both FATs on TexFAT) and flushes the device. One call per job, not V2's `sync` plus `flush`.
- `readdir` returns the entry at or after `from`, or `None` at the end; the caller continues from `next_cursor`, which FUSE `readdir(offset)` and kernel `getdents` need. `.` and `..` are never listed. Entries are not pinned, but a listed id equals what `lookup` of that name returns until the directory changes, so `ls -i` and `stat` agree. ISO lists only the highest version of a name, without `;N`; `lookup` accepts an explicit `;N`.
- `lookup` follows the format's rules: case folding, 8.3 aliases, ISO versions. Shared FAT folds Unicode case as Windows does; shared exFAT compares through the volume's up-case table.
- A directory has one `NodeId` however it is reached, so a walk detects directory loops by id (FAT derives directory ids from the first cluster).
- `create` and `mkdir` take the initial `SetAttr`; fields the format cannot store are ignored, as `open(2)` ignores mode bits a filesystem lacks. `AlreadyExists` if any name matches, including case-only and short-alias matches.
- `setattr` succeeds when the value the format would report after storing it equals what was asked, after rounding to the field's resolution and deriving dependent bits; otherwise `touch -d` with odd seconds and `chmod 644` would fail on FAT. A field the format does not store fails with `Unsupported` (META-TIME-02, META-PERM-02).
- `truncate` is separate from `setattr` because it fails differently (`NoSpace`, `FileTooLarge`). The cost is that FUSE `setattr` with size and mode is two calls, not atomic.
- `read` is short at end of file and returns zeros for unwritten ranges. `write` past the end fills the gap with zeros. `rename` keeps the moved node's `NodeId`.
- Reads never change times. A failed call changes nothing, in memory or on disk; the conformance suite tests this ("rejection" scenarios). In the async mode a call whose future is dropped before it completes leaves no pin.
- Write methods default to `ReadOnly`, so a read-only format implements only the read half and gains writes later by overriding them.

The `hadris-fs-contract` crate (outside 3.0 semver) checks the
format-independent rules of this contract as a test kit, which every format
port runs.

What each choice buys, and what it replaced:

| Choice | Rejected | Because |
|---|---|---|
| One node trait on `&mut self`, names from the catalog's operation list | `FsDriver` plus a `FileSystem` twin on `&self` with forwarding impls, `Access`, `AsDriver` | FUSE and kernels hold `&mut` state (FILE-OPEN-02); sharing is `Volume`'s job (HOST-CONC-01). One trait halves the forwarding work each time the trait grows (R10). |
| Handles only at the shared tier; the bare tier is `open`, `read`, `write` and `close` on node ids at an offset | `File<A>` and `Dir<A>` over every tier, `OpenFile` | FUSE needs nothing more. Positions, append and drop behaviour need a volume to publish into, which only the shared tier has. D6 is superseded. |
| `create`, `mkdir`, `unlink` and `rmdir` as separate methods; `create` and `mkdir` take an initial `SetAttr` | `NewNode`, `RemoveKind`; create then setattr | 3.x `symlink`, `link` and `mknod` land as new defaulted methods with no enum to reserve now (NF-STABLE-02). UDF write and FUSE need mode and owner at creation. |
| `forget(node, count)`; `generation`, `allocated` and `device` in `Metadata` in 3.0 | `forget` plus `forget_n` later (D12) | Catalog decision 3 (META-INO-02, META-ALLOC-01, META-DEV-01) and FUSE `forget(nlookup)`. |
| `DirEntry` owns its name and carries metadata | A `NameBuf<N>` out-parameter and a small entry | One type for node and path listings, no allocation (DIR-LIST-01), no second lookup for `ls -l` (D9). The copy is small next to a directory read. |
| `close` publishes, `fsync` makes durable | `publish_node` plus `sync_node` | FILE-CLOSE-01, FILE-SYNC-01; one call per job. |
| `ErrorKind::Unsupported` maps to `EOPNOTSUPP` | `ENOSYS` | FUSE treats `ENOSYS` as "never call this again" (HOST-ERRNO-01). |
| Extras are inherent methods with the same names on every driver | Defaulted trait methods (`extents`, `records`), an `Inspect` trait | The shared trait stays the closed core set; format-specific things stay with the format. |

The earlier draft split reads and writes into `FileSystem` and
`FileSystemMut`. A compile-time split doubles the traits and the bounds on
every helper, and it still cannot describe the common case: a driver that
becomes read-only at runtime when its device refuses a write (4.2).
`Capabilities::writable` and `ErrorKind::ReadOnly` cover it.

**Volume-specific operations stay native.** Formatting and checking are free
functions (`fat::sync::format`, `fat::sync::check`). Mounted extras are
inherent methods: `info()` returns the parsed descriptor (the serial is
`info().volume_serial()`, D2), `extents(node, from, &mut [Extent])` maps a
file, `records(node, &mut [Extent])` locates its on-disk records, and
`read_raw(offset, buf)` reads through the driver's cache. FAT and exFAT add
`was_dirty`, `set_label` and `set_volume_serial`; ISO adds
`boot_catalog(&mut buf)` and `boot_image(&entry)`; UDF adds `was_dirty`. On a
shared volume they are reached through `vol.lock()`, on `AnyFs` by `match`.

**Tiers.** Every tier can do every job; the tiers differ in what the user
wraps, not in what they can reach.

| Tier | Build it with | Needs | Gives |
|---|---|---|---|
| Bare | `FatFs::mount(dev, options)?` | `alloc` for `FatFs` and `ExFatFs`; nothing for `IsoFs`, `UdfFs`, `NtfsFs` | The node API, format extras, `Walk`, `copy_tree` |
| Shared | `Volume::new(fs)` | sync: `std`; async: `alloc` | Path methods named after `std::fs`, any number of `File` and `ReadDir` handles, cheap clones that move between threads and tasks |
| Host | `host::open(path)`, `host::mount_options()` | `std`, sync | Host files as devices, host trees, the host clock and time zone (4.15) |
| Embedded | `Fat::mount_with(dev, options)` | nothing | The handle-based firmware API (4.15) |

**One canonical pattern per tier.** Each tier's module docs open with one
pattern, and the rustdoc examples use only that pattern.

```rust
// Bare tier: a kernel VFS, FUSE or a format tool. Node ids, no paths, no lock.
let mut fs = FatFs::mount(dev, MountOptions::new())?;
let node = fs.lookup(fs.root(), Name::new("kernel.bin"))?;
fs.open(node, OpenMode::Read)?;
let n = fs.read(node, 0, &mut buf)?;
fs.close(node)?;
fs.forget(node, 1);

// Shared tier: applications and servers. Paths and std-like handles.
let vol = Volume::new(FatFs::mount(dev, host::mount_options())?);
let mut log = vol.open("/log.txt", OpenOptions::new().write().create().append())?;
log.write(b"hello\n")?;
log.close()?;                              // consumes the File; Drop is best effort
for entry in vol.read_dir("/EFI")? { let entry = entry?; }       // fuses on error
let worker = vol.clone();                  // clones share the volume
std::thread::spawn(move || worker.metadata("/big.bin"));
```

| Job | Bare | Shared |
|---|---|---|
| Open by path | `resolve(path, Resolve::Lexical)`, then `open(node, mode)` | `vol.open(path, options)` |
| Several files at once | Open several nodes; the caller keeps positions | Any number of `File`s |
| `std::io` on a file | Not provided; the caller owns positions | Sync `File` implements `std::io::{Read, Write, Seek}` with `std` |
| List a directory | `readdir` with a cursor, or `Walk` | `vol.read_dir(path)` |
| Format-specific calls | Inherent methods | `vol.lock()` |
| Path semantics | `resolve(path, Resolve::Follow)` | `Volume::with_resolve(fs, Resolve::Follow)` |
| Copy a tree in or out | `copy_tree(&tree, &mut fs, dir)` | `read_tree(&vol, path)`, then any writer |
| Get the device back | `fs.unmount()` | `vol.into_inner()`, then `unmount` |

**Handles.** `File<F>` and `ReadDir<F>` exist only on `Volume<F>`.

- `File` has `node`, `read`, `write`, `seek`, `set_len`, `metadata`, `sync_all` and `close(self)`. `#[must_use]`. Several handles on one node see each other's writes and size at once, and a handle keeps working after its file is renamed or moved.
- `close(self)` returns `Result` and publishes the node's metadata; a double close does not compile. `sync_all()` makes the file durable first. In the sync mode `Drop` publishes a written file's size best effort, so a dropped log file keeps its size across a power cut; async `Drop` cannot await, so there the size is published by the next call on the volume (FILE-CLOSE-01).
- A write open fails with `ReadOnly` before anything is truncated. A symlink as the last component fails with `Symlink` unless the volume follows links, as POSIX `O_NOFOLLOW` fails with `ELOOP`.
- `ReadDir` is an `Iterator` in the sync mode and has `next_entry` in the async mode. It fuses after an error (R7).
- In the sync mode `File` implements the `std::io` traits with `std`. The `Iterator` versus `next_entry` split and the `std::io` impls are the documented mode differences; the parity check (R11) lists them.

**Path methods.** `Volume` has inherent methods named after `std::fs`:
`open`, `metadata`, `symlink_metadata`, `read_dir`, `read_link`,
`create_dir`, `create_dir_all`, `remove_file`, `remove_dir`,
`remove_dir_all`, `rename` (replaces an existing target, as
`std::fs::rename` does) and `set_attr`. There is no `DriverExt` or `PathExt`
and no extension trait in their place (S6). `Volume` does not implement
`FileSystem`: path `open` and `rename` would share names with the node
methods on one type.

**Trees in and out.** `copy_tree(&Tree, &mut F, dir)` copies a tree into any
mounted filesystem, and `read_tree(&Volume<F>, path)` turns a mounted subtree
into a `Tree` whose file content is read lazily (4.7). The host side is
`host::read_tree` and `host::write_tree`. This replaces the earlier
`copy_tree(src, dst)` between two drivers and the host helpers
`extract_to_host` and `import_from_host`.

The conformance suite's adapters become one generic impl over `FileSystem`.
The rust-fatfs and mtools peers keep their own adapters, or implement the
trait themselves.

### 4.4 Sharing and locking

Drivers take `&mut self` and hold no lock. Sharing is a wrapper the user
chooses:

```rust
pub struct Volume<F> { .. }                  // Arc inside; Clone shares the volume

impl<F: FileSystem> Volume<F> {
    pub fn new(fs: F) -> Self;                             // paths resolve lexically
    pub fn with_resolve(fs: F, resolve: Resolve) -> Self;
    pub fn lock(&self) -> VolumeGuard<'_, F>;              // Deref and DerefMut to F
    pub fn into_inner(self) -> Result<F, Self>;            // Err while clones or handles exist
    // path methods (4.3)
}
```

- `Volume<F>` has no lock type parameter. The sync `Volume` uses the std mutex and needs `std`; the async one uses a portable async mutex and needs only `alloc`. Stored types never name a lock.
- The lock is held for one call. A path resolves under one lock hold, so a path costs one lock, not one per component.
- `vol.lock()` returns a guard to the driver for node calls and format-specific calls. Dropping a `File` or `ReadDir` of the volume while holding the guard queues its close and does not deadlock; with `alloc` the queue grows, so it has no fixed limit. Calling a path method on the same volume while holding the guard deadlocks, as with any mutex; `lock()` documents it.
- Clones are cheap and share the volume, so a handle or a clone can move to another thread or task. In the async mode the futures of volume, handle and node calls are `Send` for any `Send` device, and a task that holds the guard across `.await` can be spawned.
- Operations on one volume serialize, as they do in V2.

A `no_std` sync user with `alloc` has the bare driver and no `Volume`. Such
users are kernels, which hold the driver under their own lock.
`Volume::with_lock(fs, raw_lock)` can be added in 3.x if one asks.

Rejected:

- `Volume<F, K: LockKind>` with `Volume::spin`, `Volume::local`, `Volume::with_lock::<K>` and an `embassy-sync` feature. It put a type parameter in every stored type for a choice only `no_std` sync users face, and they are kernels, which lock the driver themselves. Firmware without an allocator uses the embedded API.
- A lock inside each format crate (variant B of the lock-placement prototype, [Q1](#7-open-questions)). Each crate had to pick a lock, async builds needed an async lock inside the format crate, the V2 bug class (a guard held across `.await`) stayed possible, and a kernel that wants no lock paid for one anyway. With the lock outside, format crates never see a lock.
- `Volume` implementing `FileSystem` with a lock per call. Path `open` and `rename` and node `open` and `rename` would share names on one type, and a path resolves under one lock hold either way.

### 4.5 Node identity: the open-node table

FAT and exFAT have no inodes. The natural identity of a file is the location
of its directory entry, and that changes on rename. V2's `FileEntry` snapshot
model produced `StaleEntry`, #90 and #24.

`FatFs<D>` and `ExFatFs<D>` keep a private open-node table on the heap, which
is why they need `alloc`:

- An entry records the directory-entry location, the first cluster, the size, the pin count and the open count. `lookup` finds or creates the entry and adds a pin; `forget` drops pins, and the entry may go at zero. A driver that must keep state past the last pin (unwritten metadata) holds a pin of its own.
- The table grows without a limit. `MountOptions::with_node_limit(n)` caps pinned and open nodes; past the cap `lookup`, `create` and `mkdir` fail with `LimitExceeded`.
- A directory's id is derived from its first cluster, so it has one id however it is reached. A file's id comes from its entry location when first pinned and stays with the node. On FAT a tier in the bits above bit 40 counts up while a pinned node that moved away holds the lower tiers of that slot, so a listing and a lookup compute the same id. exFAT file ids are the File entry's byte offset divided by 32.
- `rename` updates the location in place, so the `NodeId` stays valid. `truncate` and `write` update the size in the table, so every handle sees the same size, and the directory entry is written on `close`, `fsync` or `sync`.
- Removing a pinned node succeeds; its entry stays, marked unlinked, until the last `forget`, and answers `NotFound`. Removing the last name of an open node fails with `Busy` ([Q3](#7-open-questions)).
- Ids of unpinned entries from `readdir` are valid until the next change to that directory. A pinned id is stable across rename (META-INO-01 "while pinned"); an unpinned file id from a listing changes when its entry moves. `lookup` is the way to keep one.

ISO (the location of a directory's own `.` record, or of a file's record),
UDF (ICB location and partition) and NTFS (MFT reference with sequence number)
have stable ids that fit in a `u64`, so they need no table and no allocator.
The names of an ISO Rock Ridge hard link share the id of the first record in
path table order with the same `PX` serial number; UDF hard links share their
ICB. Rejecting a forged id with `InvalidHandle` is best effort for these
formats: a forged id that still decodes reads garbage but is never undefined
behaviour.

The embedded API has no node table. An embedded entry's `node()` is its
entry position, valid until the directory changes (FILE-OPEN-02 for `emb`).

Rejected: a public `NodeTable` trait as a driver type parameter
(`FatFs<D, T: NodeTable = FixedTable<64>>`) with `FixedTable<N>` and
`HeapTable`. It served firmware and FUSE with one type, but the fixed default
ran out under a FUSE mount and every hosted user saw the generic. Firmware now
has the embedded API, and hosted code gets an unbounded table with an optional
cap ([Q8](#7-open-questions)).

### 4.6 Errors

Every operation in every crate returns one type, generic over the device's
error:

Defined in `hadris-io` and re-exported by `hadris-fs` and `hadris` (Q10):

```rust
#[derive(Clone, Copy)]                 // when E is
pub struct Error<E> { .. }             // kind, message, location, detail code, device error

impl<E> Error<E> {
    pub const fn new(kind: ErrorKind, message: &'static str) -> Self;
    pub const fn device(error: E, message: &'static str) -> Self;   // kind Io
    pub fn with_location(self, location: Location) -> Self;
    pub fn with_detail(self, detail: DetailCode) -> Self;
    pub fn kind(&self) -> ErrorKind;
    pub fn message(&self) -> &'static str;
    pub fn location(&self) -> Option<Location>;
    pub fn detail(&self) -> Option<DetailCode>;
    pub fn device_error(&self) -> Option<&E>;
    pub fn into_device_error(self) -> Option<E>;
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> Error<F>;
    pub fn without_device<F>(&self) -> Error<F>;
}

pub type FsResult<T, E> = Result<T, Error<E>>;

#[non_exhaustive]
pub enum Location { Byte(u64), Block(u64), Cluster(u64), NameByte(u32) }

pub struct DetailCode { .. }           // a u16 within a static domain
impl DetailCode {
    pub const fn new(domain: &'static str, code: u16) -> Self;
    pub fn domain(self) -> &'static str;
    pub fn code(self) -> u16;
    pub fn code_in(self, domain: &str) -> Option<u16>;
}

pub struct MountError<D, E> { .. }     // the Error<E> and the device given back
impl<D, E> MountError<D, E> {
    pub fn error(&self) -> &Error<E>;
    pub fn into_parts(self) -> (Error<E>, D);
}
impl<D, E> From<MountError<D, E>> for Error<E> { .. }
impl<E: ..> From<Error<E>> for std::io::Error { .. }          // std

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Io,                  // the device failed; device_error() is Some
    NotRecognized,       // the bytes are not this format at all
    Corrupt,             // this format, but it violates the specification
    NotFound,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    NoSpace,
    ReadOnly,            // a read-only mount or device, or a mount that turned read-only
    InvalidInput,        // bad options, names or arguments from the caller
    Unsupported,         // the format cannot store or do this
    LimitExceeded,       // a value does not fit a field, a buffer or a depth limit
    NameTooLong,         // ENAMETOOLONG
    FileTooLarge,        // EFBIG: past the format's file size limit
    Symlink,             // too many symlinks, or a symlink where a file or directory was needed
    InvalidHandle,       // unknown or forgotten NodeId, or a stale embedded handle
    Busy,                // removing the last name of an open node
}
```

- The context is `Copy` and needs no allocation: a static message, an optional location and a detail code, so nothing is dropped at crate boundaries. The detail code is a `u16` within a static domain (the crate name), so `Detail::of` of one crate never misreads another crate's code. Each crate has a `#[non_exhaustive] enum Detail` read back with `Detail::of(&err)`. Two errors are equal when their kinds and device errors are; the context never takes part. Detail codes are shared with `check` findings (4.15), so the finding for a bad boot sector carries the code mount fails with. The per-crate `Error` wrappers are removed.
- `NotRecognized` separates "not this format" from `Corrupt`, so `detect` and `open` can tell a foreign device from a damaged one.
- `ErrorKind::errno()` returns a symbolic `Errno` (one per kind); `Errno::linux()` gives the number. `Unsupported` is `EOPNOTSUPP`, never `ENOSYS`; `NotRecognized` and `InvalidInput` are `EINVAL`, `Corrupt` is `EUCLEAN` (Linux `EFSCORRUPTED`), `LimitExceeded` is `EOVERFLOW` and `InvalidHandle` is `ESTALE`. One kind maps to one errno, so FAT chmod refusals give `EOPNOTSUPP` where Linux vfat gives `EPERM`. `Errno::darwin()` for macFUSE is additive.
- An error either shows its source's text in `Display` or returns it from `source()`, never both, so `anyhow`'s `{:#}` prints each message once.
- **Kernels** keep their own error without `alloc`. The errno mapping is `err.kind().errno()`, or a `match` on `(err.kind(), err.device_error())`. Filesystem failures have no device error.
- **std users** use `?` into `std::io::Error` or `Box<dyn Error>`. A device error that is already an `io::Error` comes back as itself, found by a downcast that does not allocate, so `raw_os_error()` survives. Any other device error becomes the source of an `io::Error` with kind `Other` and can be downcast back out. Filesystem failures map their kind (`NotFound` to `NotFound`, `ReadOnly` to `ReadOnlyFilesystem`, and so on).
- **Drivers that take the device by value** fail to mount with `MountError<D, E>`, which gives the device back. `?` still converts it to `Error<E>`, `PathError` or `io::Error`, dropping the device.
- **Generic code** names the device error through the trait: `fn install<F: FileSystem>(fs: &mut F) -> FsResult<(), F::DeviceError>`.
- **Code that needs paths or mixes devices** returns `PathError` (`alloc`): kind, message, the tree path, and with `std` the host path, with any device error boxed as its `source()`. Writers, tree edits, `copy_tree`, `read_tree` and the host module return it, and it takes `?` from any `Error<E>` or `MountError<D, E>` (NF-ERR-04).
- No `String` payloads without `alloc`. No foreign types (`bytemuck::PodCastError`, `PathBuf`) in the public API outside `host`.
- Module-level `Error` and `Result` aliases (`write::Error`, `modify::Error`) are removed.

Rejected:

- `AnyError` (a kind plus a boxed device error) and a separate `host::Error`. `PathError` does both jobs and also carries the paths a writer needs.
- A generic `BuildError<E>` for writers. It cannot take `plan`'s `Infallible` errors with `?`, because `From<BuildError<Infallible>> for BuildError<E>` overlaps `From<T> for T`.
- `WriteError<E>` and `StorageError<E>` at the device layer (S5, decision B). `ErrorKind::ReadOnly` already made the distinction.
- The step 6 draft's private `Context` with typed accessors (`err.sector()`, `err.cluster()`). `Location` and the detail code cover it without a type per crate.

### 4.7 Shared input tree for writers

One mode-independent `Tree` in `hadris-fs` (behind `alloc`) feeds every
writer, `copy_tree` and extraction:

```rust
let mut tree = Tree::new();
tree.insert("boot/grub/grub.cfg", Node::file(Content::bytes(cfg)))?;
tree.insert("install.wim", Node::file(host::file("/data/install.wim")?))?;
tree.insert("empty", Node::dir())?;
tree.insert("latest", Node::symlink("releases/3.0"))?;
tree.insert("dev/console", Node::special(FileType::CharDevice, Some(DeviceNumber::new(5, 1))))?;
tree.link("bin/sh", "bin/busybox")?;
tree.replace("etc/motd", Node::file(Content::bytes(motd))
    .with_attrs(SetAttr::new().with_permissions(Permissions::new(0o644))))?;

let (tree, skipped) = host::read_tree("rootfs/", &TreeOptions::new().with_clamp(epoch))?;
```

- `Tree` has `new`, `insert`, `link`, `remove`, `replace` and `get`. Paths are `/`-separated bytes. A leading `./` is ignored, and `.` and `..` components are refused, so extraction cannot write outside its directory. Children are sorted by name bytes, so output does not depend on the host's listing order (BUILD-REPRO-01).
- `Node` is `file(Content)`, `dir()`, `symlink(target)` or `special(FileType, Option<DeviceNumber>)`, with `with_attrs(SetAttr)`. Formats ignore what they cannot store and report it as a warning.
- `Content` is opaque and cloneable: `Content::bytes`, `host::file(path)`, content read lazily from a mounted volume by `read_tree`, and extents of an existing ISO session. Its length is fixed when it is made, so planning does no I/O. The lazy kinds are private; `Content::source` for user-supplied content is additive in 3.x. Lazy content is readable only by writers of the mode that produced it; the other mode's plan fails with `Unsupported` naming the path, so async code never blocks on a hidden sync read (NF-MODE-01).
- Streaming is the normal path. `unstable-streaming` goes away. No layout state on the tree; writers keep their planning structures `pub(crate)`.

Every format has the same two entry points, and both return the same report:

```rust
let report = iso::plan(&tree, &opts)?;                   // no I/O
let report = iso::sync::write(&mut out, &tree, &opts)?;  // plans again first
report.size(); report.warnings(); report.extents("install.wim");
```

- `fmt::plan(&tree, &opts) -> Result<Report, PathError>` is a free function outside `sync` and `r#async`, since planning does no I/O. It returns exactly the report `write` returns. `write` plans again first, so an option error never touches the device.
- `Report` has `size`, `warnings` (each a `Warning` with a non-exhaustive `WarningKind`: `Renamed`, `Truncated`, `Deduplicated`, `Relocated`, `Dropped(Field)`, `Skipped`, `Boot`, plus the path and the name stored), and `extents(path)`, several for a multi-extent file. It implements `Display`.
- FAT and exFAT write a tree through format plus `copy_tree` and report the same way (`fat::sync::write`, `exfat::sync::write`).
- Writers take a fixed `with_time(DateTime)` and no clock. Volume ids and GUIDs derive from the time plus the tree unless `with_seed` is given, so output is reproducible by default (BUILD-REPRO-01, NF-DET-01).

Between mounted volumes, trees and the host:

| From | To | Call |
|---|---|---|
| `Tree` | mounted filesystem | `sync::copy_tree(&tree, &mut fs, dir)`, any `FileSystem`, bare tier |
| mounted volume | `Tree` | `sync::read_tree(&vol, path)`, lazy content through a clone of the `Volume` |
| host directory | `Tree` | `host::read_tree(dir, &TreeOptions) -> (Tree, Vec<PathError>)` |
| `Tree` | host directory | `host::write_tree(dir, &tree) -> Report` |
| cpio stream | `Tree` | `cpio::sync::read_tree(&mut reader)` |

- `copy_tree` keeps directory times (set after their children), drops fields the target does not store and skips nodes it cannot hold (symlinks, special files and extra hard-link names on FAT), each in the report. An existing name fails with `AlreadyExists`; merge and overwrite policies are 3.x.
- `read_tree` keeps hard links (by `NodeId`), symlinks and devices. Its content is read lazily, so converting an 8 GB ISO to exFAT does not need 8 GB of RAM (BUILD-COPY-01). `Volume::into_inner` fails while the tree lives. The sync form needs `std`, since the sync `Volume` does.
- `host::read_tree` takes a symlink policy (`Symlinks::{Keep, Follow, Skip, Fail}`), an error policy (`OnError::{Fail, Skip}`), an exclude filter, an owner override and an mtime clamp. It returns the tree plus the errors skipped under `OnError::Skip`: a skipped unreadable entry is an error the user chose to tolerate, not a lossy conversion.
- `host::write_tree` never writes outside `dir`: names with a drive prefix, `\` or NUL fail, and no existing symlink under `dir` is followed. Symlinks from the tree are created last. It keeps permissions, times and hard links; owners are not restored and device nodes, FIFOs and sockets are skipped, both reported. An existing file fails with `AlreadyExists` and existing directories merge. It has no options in 3.0; `write_tree_with` is additive.

Rejected:

- A separate `Plan` or `Layout` type passed to `write`. One report type means one thing to print and test.
- `copy_tree(src_fs, dst_fs)`, a `TreeSink` trait implemented by volumes and writers, and `Tree: FileSystem`. The source of every copy is a tree; the 3.0 trait has no `symlink`, `link` or `mknod`, so a tree cannot be a filesystem.
- The earlier `add_file`, `add_dir`, `add_symlink`, `add_device`, `add_hard_link` and `set_metadata`, `Content::path`, `Tree::from_fs` and `TreeExt::from_filesystem`. They became `insert`, `link`, `replace`, `host::file`, `host::read_tree` and `read_tree`.
- Loading file data into the tree when reading a volume, and blocking reads inside async writers.

### 4.8 Sync and async

Keep the `strip_async!` code generation and use it everywhere:

- `hadris-io` and `hadris-storage` move from hand-copied files to the same generator. Every async fix is written once.
- Mode-independent types (raw layouts, names, options, metadata, errors, trees, `NodeId`, `plan`) are defined once, outside the generated modules.
- Only types that do I/O live in `sync` and `r#async`.
- The async source is written as `fn f(..) -> impl Future<Output = T> + Send` with `Send` supertraits. The generator turns it into `fn f(..) -> T`, removes `async` and `.await`, and drops the `Send` supertraits and the `+ Send` on captured arguments.
- The shared tier has two modes, `sync` and `r#async`, and async futures are `Send` whenever the device is. Non-`Send` async exists only in `hadris-io` and `hadris-storage` (the `local` modules) and in the embedded API, which takes `local::BlockDevice`. The `async_send` mode and the `async-send` feature are gone (4.17); [Q2](#7-open-questions) has the history.
- Every crate exposes the same public items in both modes. The parity check in R11 enforces it and lists the intended differences: `Iterator` versus `next_entry`, the `std::io` impls, and the sync-only `host` module.
- Sync stays generated rather than wrapping async code in `block_on`: that was measured at 40 to 49% more code and 70% more worst-case stack on thumbv7em (4.15).
- `hadris-macros` gains span-preserving errors so contributors see the right line.

### 4.9 Feature flags

| Feature | Meaning |
|---|---|
| `alloc` | Heap-backed items: `FatFs` and `ExFatFs`, the async `Volume`, `Tree` and the writers, `PathError`, `copy_tree`, `open` and `AnyFs`, `Walk::new`, `Box` forwarding. |
| `std` | Implies `alloc`. The sync `Volume`, `std::io` impls on sync handles, `From<Error<E>> for std::io::Error`, `StdIo`, `SystemClock`, the `host` module, sync `read_tree` and ISO `Session`. |
| `embedded-io` | `FromEmbedded<T>` for embedded-io and embedded-io-async streams. Nothing else names embedded-io. |
| `sync` | Sync API. On by default. |
| `async` | Async API: `Send` futures in the shared tier, `local` futures in the embedded API. |
| `write` | Undecided (S4 in 4.17): it either goes, leaving writers always compiled, or stays as "can create images" in every crate. Settled with the feature rework. |
| `unstable-*` | Enables a preview module, such as `unstable-ntfs`. Never changes stable items. |

- `read` is dropped; reading is always available. `async-send` is dropped.
- `crc` and `rand` stop being user-facing features. CRC is always compiled. GUIDs come from the caller or the seed (4.10).
- `cache` stops being a feature. `Cache<D>` is always available and costs nothing unless constructed.
- No feature changes behaviour. Path semantics (4.12), the time zone default (post-pass decision C) and the node table (4.5) are values or types, so enabling a feature anywhere in the build only adds items. The same holds for `Walk`: `Walk::new` needs `alloc` and `Walk::with_stack` does not, as two constructors, not one whose stack depends on a feature.
- `hadris-fs` defaults to `std` and `sync` like every other crate (D3).
- The umbrella forwards the same axes plus one feature per format.

FAT and exFAT without `alloc` are the embedded API (4.15) plus `format`,
`check` and the raw crates, all of which need no allocator. The V2 `lfn` and
`dirty-file-panic` features switched behaviour and were removed with the V2
driver: `FatFs` always reads and writes long names, and it has no writer that
can be dropped unfinished.

### 4.10 Partition GUIDs and randomness

No hidden RNG. `Gpt::new(disk_guid: Guid, ..)` and `GptEntry::new(unique_guid:
Guid, ..)` take GUIDs, and `Guid::random()` exists only with `std`. Image
writers and formatters take `with_time` and an optional `with_seed`; volume
serials, ISO and UDF ids and the GUIDs of hybrid images derive from the time
plus the tree (or the seed), so the same input gives the same image and
different content gives different ids. `FatOptions::with_serial` and
`ExFatOptions::with_serial` set the serial directly.

### 4.11 Crate layout

| V2 crate | V3 |
|---|---|
| `hadris-io` | Kept. `ErrorType`, `Read`, `Write`, `Seek`, `ExactError`, `FromEmbedded`, `StdIo`, in `sync`, `r#async` and `local`. |
| `hadris-storage` | Kept and adopted by every filesystem. `BlockDevice` in `sync`, `r#async` and `local`, `Partition`, `Vec<u8>`, `StreamDevice`, `MemDevice`, `Cache`, `ByteView`. |
| `hadris-fs` | New. Vocabulary, `Error<E>`, `PathError`, `MountOptions`, `FileSystem`, `Volume` and its handles, `Walk`, `Tree`, `Content`, `Report`, `copy_tree`, `read_tree`, `Finding`, `Severity`, `CheckReport`. |
| `hadris-fs-contract` | New, 0.x, outside 3.0 semver. The driver test kit (4.3). |
| `hadris-<fmt>-raw` | New, one per format, each with its own version: I/O-free codecs and `raw::io` primitives (4.15). `hadris-fat-raw` covers FAT and exFAT. |
| `hadris-common` | Internal. Endianness and fixed-size string types, merged with `hadris-fixed` into one set. Documented as not for direct use. |
| `hadris-fixed` | Merged into `hadris-common`. |
| `hadris-path` | Merged into `hadris-fs`. |
| `hadris-macros` | Kept, internal. |
| `hadris-archive` | Removed. The umbrella re-exports `hadris-cpio` directly. |
| `hadris-cd` | Removed (S3). The bridge writer is `hadris_udf::plan_bridge` and `hadris_udf::{sync, r#async}::write_bridge`. |
| `hadris-block`, `hadris-optical` | Removed (S2). `detect`, `open` and `AnyFs` move into the umbrella. |
| CLI crates | Replaced by one `hadris` binary with subcommands (S1), in the `hadris-cli` package (Q15). |
| `hadris-vfs` | New, `std` only, can ship in 3.x. Type erasure, a mount table for composing volumes, and a `fuser` adapter (prototyped in `experiments/fuse-prototype`). The sync `FileSystem` is dyn-compatible, so the sync form needs no second trait: an erasing wrapper boxes the device error, and one `Vec<Box<dyn FileSystem<..> + Send>>` holds FAT and ISO volumes with different device errors. The async form needs an object-safe trait with boxed futures, since the async trait is not dyn-compatible. Lost: the typed device error (boxed, error path only) and format extras. |
| Format crates | Kept. |
| `hadris` | Umbrella. Re-exports `io`, `storage` and `fs` so `no_std` users need one dependency, with flat paths (`hadris::fat`, `hadris::iso`). Holds `detect`, `open`, `AnyFs` and the `host` module, since `host::open` needs every format. |

### 4.12 Path resolution

Path semantics are a value, `Resolve`, never a cargo feature. Features unify
across the build graph: if `alloc` or a `posix` feature switched how `..`
works, one dependency turning it on would change what paths mean for every
other crate in the build. A value only affects the code that passes it, so
two volumes in one program can resolve differently.

```rust
#[non_exhaustive]
#[derive(Default)]
pub enum Resolve {
    #[default]
    Lexical,     // `..` removes the previous component of the text; symlinks are never followed
    Follow,      // POSIX: `..` is the real parent; symlinks are followed, 40 at most
    NoFollow,    // POSIX, but a symlink in the last component is returned, not followed
}
```

| | `Lexical` (default) | `Follow` | `NoFollow` |
|---|---|---|---|
| `..` | Removes the previous component of the text | Goes to the real parent through `parent` | Same as `Follow` |
| `/a/missing/../b` | `/b` | `NotFound` | `NotFound` |
| `/file.txt/..` | `/` | `NotADirectory` | `NotADirectory` |
| Trailing `/` on a file | Ignored | `NotADirectory` | `NotADirectory` |
| Symlinks mid-path | Not followed; gives `NotADirectory` | Followed, 40 at most (`Symlink`, like `ELOOP`), absolute and relative targets | Followed |
| Symlink as the last component | Returned | Followed | Returned (`lstat`, `O_NOFOLLOW`) |
| Driver methods used | `lookup` | `lookup`, `stat`, `parent`, `readlink` | Same as `Follow` |
| `alloc` | Not needed | Not needed | Not needed |

`Lexical` is the default because it works on every filesystem, including
ones without symlinks, and costs the least: one `lookup` per component and one
pin at a time. It is how Windows and URL resolution treat `..`. On a
filesystem without symlinks it agrees with POSIX whenever every component
exists. `Follow` and `NoFollow` splice link targets into a fixed buffer in the
resolve call, so POSIX semantics need no allocator; a path plus link text
beyond the buffer fails with `LimitExceeded`, like `ENAMETOOLONG`.

Choosing a policy, per tier:

- Bare: `fs.resolve(path, Resolve::Follow)` for one call. `resolve` has a default body, so a driver only overrides it to resolve faster.
- Shared: `Volume::with_resolve(fs, Resolve::Follow)`. Every path method and handle on that volume uses it, and the whole path resolves under one lock hold. `symlink_metadata` always uses `NoFollow` for the last component (LINK-SYMLINK-04).
- Embedded: names are passed one component per call, with `create_dir_all(dir, path)` the one path method.

Rejected: a `Resolver` trait with `Lexical` and `Posix<N>` implementations and
a `WithResolver<D, R>` driver wrapper. LINK-SYMLINK-03 and -04 need three
behaviours and no action needs a user resolver. The enum is non-exhaustive, so
a jail that refuses `..` past a starting directory can be added in 3.x.

### 4.13 Known costs and limitations

Each row is a deliberate trade, what it buys, and what a user does about it.

| Cost | Why it stays | What users do |
|---|---|---|
| Device errors must be `core::error::Error + Send + Sync + 'static` in the shared tier | Makes every device error erasable into `PathError` and `io::Error` with no where-clauses | Wrap an error that holds an `Rc` or raw pointer, or use the embedded API with `local::BlockDevice` |
| A non-io device error becomes `io::ErrorKind::Other` in std | std has no kind for "your device's enum" | Downcast `io::Error::into_inner()` to get it back |
| Generic code carries `F::DeviceError` | Keeps the device error without allocation | Use `FsResult<T, F::DeviceError>`, or return `PathError` |
| `FatFs` and `ExFatFs` need `alloc` | An unbounded node table with no type parameter (4.5) | Firmware uses the embedded API |
| The sync `Volume` needs `std`, so a `no_std` sync user has no handles | A blocking mutex needs the OS | Hold the driver under your own lock; `Volume::with_lock` is additive |
| Sync `read_tree` and ISO `Session` need `std` | They read lazily through the sync `Volume` | Build trees in memory without `std`; the writers themselves need only `alloc` |
| Lazy content is readable only in the mode that produced it | No hidden blocking in async code | Read the source in the writer's mode |
| Read-only is detected up front by `writable()` and later on the first refused write | A write probe is harmful (4.2) | `MountOptions::read_only()` when it is known; `capabilities().writable()` afterwards |
| The default resolution is lexical, not POSIX | Works on every format, costs least | `Resolve::Follow` |
| Calls on one `Volume` serialize; a path resolves under one lock hold | One lock per call keeps format crates lock-free | Use several volumes, or the bare driver under your own scheme |
| Holding `vol.lock()` and calling a path method on the same volume deadlocks | The guard is a plain lock guard | Drop the guard first; documented on `lock()` |
| Removing the last name of an open file fails with `Busy` | POSIX unlink of an open file needs orphan tracking, and a crash leaves lost clusters (Q3) | Close first; orphans can come in 3.x without a break |
| `truncate` and `setattr` are separate calls | They fail with different kinds | A FUSE `setattr` with size and mode makes two calls, not atomic |
| One errno per kind | A stable, documented mapping | FAT chmod refusals give `EOPNOTSUPP` where Linux vfat gives `EPERM` |
| `DirEntry` is 768 bytes | One owned entry type with no allocation | Keep entries on the heap when collecting many |
| The FAT UTC offset is read once per mount | Times must be consistent within a mount | Remount after a daylight-saving change |
| Each device type monomorphizes the driver | Typed errors and static dispatch | Use one device type per binary where size matters |
| `read_exact` and `write_all` return `ExactError<E>` | Short reads and zero-length writes need their own cases, as in embedded-io | `?` converts into `Error<E>` |
| `ReadDir` is not an `Iterator` in the async mode | No async iterator in core | `next_entry().await` |
| A dropped embedded `File` keeps its slot until `unmount` | The handle is a slot index and cannot reach the volume in `Drop` | Call `close`; `sync` and `unmount` still publish its size |
| Embedded FAT folds ASCII by default, shared FAT folds Unicode | Keeps the Unicode tables out of firmware flash (NF-FLASH-02) | `Options::new().with_fold(hadris_fat_raw::fold_unicode)` |

Experiment E1 built the lock-placement prototype for `thumbv7em-none-eabihf`
with no allocator, with `alloc`, and with std. Its raw tier that opens a FAT
volume and reads a file was 6.0 KB of `.text` (`opt-level = "s"`, LTO), and
async raw was 8.7 KB. 64-bit division (0.9 KB) and `memcpy` (1 KB) are fixed
costs; FAT should shift by the cluster size instead of dividing. The binaries
were linked and measured, not run on hardware. The embedded API's own
footprint targets are in 4.15.

Not yet verified: embedded-io behind a feature, the async `FromEmbedded`
adapter, and a tokio file device.

### 4.14 Compatible 3.x additions to the frozen traits

The step 6 review and the API prototype found these gaps. Each fits R10: a
default method, a new constructor, a new variant of a non-exhaustive enum, or
a new private field with a getter, plus a `Capabilities` flag where callers
must ask first. None needs a break, so none blocks 3.0.

| Addition | Default | Needed by |
|---|---|---|
| `pin(node)` for an id just returned by `readdir` | `Unsupported` | FUSE `readdirplus`, `getdents` plus `stat` without a second directory scan |
| `symlink`, `link`, `mknod` | `ReadOnly` | NTFS and UDF write, FUSE `symlink`, `link` and `mknod`; `Tree: FileSystem` |
| `RenameMode::Exchange` | Formats refuse unknown modes with `Unsupported` | `renameat2` |
| `UdfFs::mount_writable` | New entry point; `mount` stays read-only | UDF write on random-access media |
| An exFAT write entry point in the embedded API | New entry point; `ExFat::mount` stays read-only | NF-NOALLOC-02 exFAT write |
| Extended attributes and named streams | `Unsupported` | NTFS ADS, UDF named streams, macOS clients |
| A `lookup` that also returns the stored name | Lookup plus a listing scan | Case-insensitive formats, the FAT CLI |
| `FsStats` available blocks and free file slots | Free blocks, unknown | FUSE `statfs` `bavail` and `ffree` |
| Per-field time resolution in `Capabilities` | The modification time's | FAT (created 10 ms, modified 2 s, accessed 1 day) |
| `DateTime` conversions to and from `SystemTime` | None | Every std adapter |
| Orphan tracking: unlink of an open node succeeds | `Busy` | POSIX semantics (Q3) |
| `Errno::darwin()` | None | macFUSE |
| `Volume::with_lock(fs, raw_lock)` | None | `no_std` sync users with `alloc` who want handles |
| `Content::source` | None | User-supplied lazy content |
| `copy_tree` merge and overwrite policies | `AlreadyExists` | Updating a populated volume |
| cpio `Entry::link_id()` and a streaming `host::write_cpio` | `read_tree` pairs hard links in memory | Extractors that stream large archives |
| An embedded FAT-or-exFAT enum | Separate `Fat` and `ExFat` | SDXC firmware |
| `write_planned(dev, &tree, &opts, &report)` | `write` plans again | Large trees where planning is measurably slow |

### 4.15 Layers per format: raw, shared, host and embedded

Updated to the API prototype (4.18); where the two differ, 4.18 decides.

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
<fmt>::{sync,async}   FileSystem drivers, Volume, handles, format and check;
                      kernels, host applications, servers, FUSE; requires
                      alloc for the FAT and exFAT drivers
host (std, sync)      open(path), FileDevice, read_tree, write_tree
<fmt>::embedded       handle-based firmware API over raw; fixed buffers, no
  ::{sync,async}      node table, no Unicode tables unless asked
```

**Raw crates.** Each format gets a `hadris-<fmt>-raw` crate with its own
version, so the low-level API can evolve without breaking the format crate's
major version. Users whose case the higher layers do not cover are pointed
here.

- The I/O-free part is pure functions and small state machines over byte slices: for FAT, boot sector parsing into `Geometry`, FAT entry encode and decode, `ChainGuard` (cycle detection), free-cluster scans, directory slot parsing and encoding, long-name assembly and checksums, short-name generation, dates with a UTC offset, the format layout planner and boot fields (hidden sectors, CHS geometry), and name folding as the function pointers `fold_ascii` and `fold_unicode` (Unicode tables link only when referenced). exFAT adds entry sets (parse, validate, seal), set checksums, name hashes, the up-case decoder, bitmap helpers and times. Most of this exists today as the private `codec` modules (about 2,900 lines).
- `raw::io` is a thin layer of device primitives generic over the device only, borrowing a caller buffer: `read_geometry`, FAT `get`/`set` (every copy), `next`, `runs` (contiguous extents, cycle guarded), `allocate`/`free_chain` with FAT writes batched per sector, directory iteration with a cached position, `write_slots`, `mkfs`, and `check` that runs without mounting. exFAT adds the bitmap, a lazy up-case index and `write_set`, which writes a whole entry set per block, secondaries before the primary. Ordering-sensitive sequences live here once, so both the shared driver and the embedded API inherit the same crash ordering. `mkfs` and `check` reach users as `format` and `check` in the format modules, which need no allocator.
- A fully sans-IO design was considered. It works for the codecs, but operations that read, decide and read again (chain walks, directory iteration, allocation) would need a hand-written state machine each on stable Rust. `raw::io` is instead generated per mode from one source.

**Shared drivers.** `FatFs<D>` and `ExFatFs<D>` have one type parameter and
require `alloc`. The clock and code page are runtime `MountOptions`, and the
node table is an internal heap table with an optional cap
(`MountOptions::with_node_limit`). `NodeTable`, `FixedTable` and `HeapTable`
leave the public API. `Volume<F>` has no lock parameter: the sync one uses
the std mutex (`std`), the async one a portable async mutex (`alloc`).
`LockKind`, `Spin`, `Local` and the `embassy-sync` feature are removed;
firmware uses the embedded API, and a `no_std` kernel holds the driver under
its own lock. Drivers without allocation needs (ISO, UDF, NTFS readers) keep
working without `alloc` at this tier.

**Modes.** The shared tier ships `sync` and `async`, where `async` is today's
`async_send` (futures are `Send` when the device is). Non-`Send` async
remains in `hadris-io`, `hadris-storage` and the embedded API. Sync stays
generated from the async source by macro: a measurement on 2026-09-24 of
async code driven by `block_on` over an always-ready device, with fat LTO at
opt-level `s` and `z`, found +40 to 49% code, +70% worst-case stack (26 KB to
45 KB on thumbv7em), 40 or more poll state machines left in the binary and
1.3 to 2.5 times slower host throughput, with identical images.

**Host.** `host` (with `std`, sync only) adds `open(path)`, which detects the
format and mounts it read-only as an `AnyFs`, `FileDevice` for image files
and host block devices, `read_tree` and `write_tree` between host
directories and trees, and `file` and `source_date_epoch` for builders.
Formatting is the shared `format` in every tier. Errors that carry a tree
path or host path use the crate-wide `PathError` (`alloc`), which replaces
the planned `host::Error` and `AnyError`. The path methods named after
`std::fs` live on the shared `Volume`. The FAT code page defaults to `Cp437`
in every tier.

**Embedded.** `hadris_fat::embedded::{sync, r#async}` and
`hadris_fat::exfat::embedded::{sync, r#async}` are separate, handle-based
APIs built only on the raw layer:

```rust
pub struct Fat<D, const FILES: usize = 4> { .. }
impl<D: BlockDevice, const N: usize> Fat<D, N> {
    pub fn mount_with(dev: D, options: Options) -> Result<Self, MountError<D, D::Error>>;
    pub fn root(&self) -> Dir;
    pub fn open_dir(&mut self, parent: Dir, name: &str) -> FsResult<Dir, D::Error>;
    pub fn open(&mut self, dir: Dir, name: &str, options: OpenOptions) -> FsResult<File, D::Error>;
    pub fn read(&mut self, file: &File, buf: &mut [u8]) -> FsResult<usize, D::Error>;
    pub fn write(&mut self, file: &File, buf: &[u8]) -> FsResult<usize, D::Error>;
    pub fn flush(&mut self, file: &File) -> FsResult<(), D::Error>;
    pub fn close(&mut self, file: File) -> FsResult<(), D::Error>;
    pub fn list(&mut self, dir: Dir, from: DirCursor, each: impl FnMut(&Entry) -> ControlFlow<()>)
        -> FsResult<(), D::Error>;
    // mount, create_dir, create_dir_all, open_node, seek, set_len, metadata, set_attr,
    // remove_file, remove_dir, remove_dir_all, rename, label, stats, was_dirty, sync, unmount
}
```

Device blocks are 512 bytes, `File` is a move-only slot index, and errors
are `Error<E>`. The targets are under 2 KB of RAM and under 2 KB of mount
stack with 4 file slots, device excluded, checked in CI on `thumbv6m`,
`thumbv7em` and `riscv32imc`. 3.0 ships FAT12/16/32 read and write as `Fat`
and exFAT read-only as the separate `ExFat`; exFAT write follows in 3.x
through a new entry point. Pass 4 in 4.18 has the details.

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
crate outside 3.0 semver. Path resolution is the `Resolve` enum (`Lexical`,
`Follow`, `NoFollow`) on `resolve` and `Volume::with_resolve`; the
`Resolver` trait, `Posix<N>` and `WithResolver` are removed.

**What this settles from the review.** C1 to C6, D1, D7 and D8, and B5 and B6
as targets of the embedded API. A1, A2, A5, B3 and B4 move into `raw::io`
primitives; A4 disappears with the `Vec` queue. Decisions: Q9.

### 4.16 Review decisions on API shape

Decided by the user on 2026-09-24 from the V3 review (items D2 to D12), and
updated to the API prototype (4.18):

| Item | Decision |
|---|---|
| D2 labels and serials | Text is always `label()`, a numeric id is always `volume_serial()`, in every crate. `label()` is a method of the `FileSystem` trait, so generic code and the openers can show it. `volume_serial()` lives on the type each driver's `info()` returns. UDF reads its other ids through `info().id(UdfId)`. |
| D3 features | `hadris-fs` defaults to `std` and `sync` like every other crate. `write` keeps meaning "can create images" in every crate, documented per crate. |
| D4 exFAT detection | `ImageFormat::Fat(FatKind)` and a separate `ImageFormat::ExFat`; `FatVariant` is removed. |
| D5 detection result | `detect` returns a `Detection` listing every format found, each with the error a mount would give; an empty one means nothing was found. |
| D6 `OpenFile` | Removed. The bare tier opens and closes nodes with `open(node, mode)` and `close(node)`; the shared tier's `File` has `close(self)`, so a double close does not compile. |
| D9 directory entries | `DirEntry::metadata()` returns the metadata the directory entry already stores, filled by every current driver. The node-based, cycle-safe walk is `Walk`; node calls on a shared volume go through `vol.lock()`. |
| D10 FAT times | `MountOptions::with_utc_offset`. The host tier defaults to the host's local UTC offset, as Windows and Linux vfat assume for FAT; the embedded API defaults to UTC. Both are overridable. The host default comes from the host module (`host::mount_options()`, which `host::open` uses), not from a feature switch: `MountOptions::new()` is UTC on every target. exFAT reads and writes its UTC offset fields. |
| D11 permissions | `Metadata::permissions()` and `SetAttr::with_permissions()`; the `mode` spellings leave `hadris-fs`. cpio keeps its raw `mode()`. |
| D12 trait additions | Settled by the catalog: `forget(node, count)`, `Metadata::generation()`, `allocated()` and `device()` are 3.0. `link`, orphans and the rest of 4.14 stay 3.x additions. |

### 4.17 Workspace simplifications

Decided by the user on 2026-09-24 after the layering pass (4.15), and
updated to the API prototype (4.18):

| Item | Decision |
|---|---|
| S1 CLIs | One `hadris` binary with subcommands (`fat`, `iso`, `udf`, `cpio`, `detect`) replaces the five CLI crates, with one set of flags, overwrite rules and output handling. The 2.x binary names are not installed. |
| S2 detection | `hadris-block` and `hadris-optical` are removed. Detection and opening move into the umbrella as `detect` (one `ImageFormat` enum for block, partition, optical and archive images) and `open`, in `sync` and `async`. `open` returns the `AnyFs` enum, which implements `FileSystem` and reaches format extras by `match`. |
| S3 bridge writer | `hadris-cd` is removed; the ISO and UDF bridge writer becomes `hadris_udf::plan_bridge` and `hadris_udf::{sync,async}::write_bridge`. |
| S5 storage errors | `WriteError`, `StorageError` and `OutOfRange` become one storage error type, and that type is the crate-wide `Error<E>`. `BlockDevice::write_blocks` and `flush` return `Error<E>`: a device that refuses writes returns kind `ReadOnly`, any other failure `Error::device`. The `WriteError` in 4.2 is superseded. |
| `impl_fs_driver!` | Dropped. `FileSystem`'s write methods default to `ReadOnly`, which removes the macro's job. The redesign brings it back only for duplication it can name. |
| S6 path helpers | Path methods exist only on the shared `Volume`, as inherent methods named after `std::fs`; the bare-driver tier keeps node-level calls. `DriverExt` and `PathExt` are removed, with no extension trait in their place. |
| Async naming | In the shared tier `r#async` means futures that are `Send` when the device is (the former `async_send`). The embedded API's `r#async` is non-`Send`. `hadris-io` and `hadris-storage` offer `sync`, `r#async` (`Send`) and `local` (non-`Send`) device traits. Features are `sync` and `async`; `async-send` is removed. |
| S4 `write` | Undecided: dropping it leaves writers always compiled and shrinks the feature matrix; keeping it makes it stable for 3.x. Settled with the feature rework. |

### 4.18 Action catalog and API prototype

Decided on 2026-09-24. [docs/v3/actions.md](v3/actions.md) lists every action on FAT, exFAT, ISO 9660, UDF and cpio with a stable ID, the users who need it and a verdict (`3.0`, `3.x` or `no`), plus the non-functional constraints (`NF-*`). An action marked `3.0` that does not work is a bug; each ID gets a conformance test in hadris-tests.

Before more implementation, the API is redesigned against the catalog as a prototype: a standalone crate with `todo!()` bodies and realistic signatures (lifetimes, `Send` bounds, error types), never merged. An item enters the prototype only when a catalog action cannot be written without it, and names the action IDs it serves. Every `3.0` action gets a usage snippet that must compile, and a FUSE-shaped adapter is written against it as a test of the core set. Sections 4.15 to 4.17 were inputs to check against the prototype and have since been updated to match it, and sections 3, 4.1 to 4.14 and 5 were rewritten from it.

Pass 1 (core mounted API), accepted on 2026-09-24:

- One node trait `FileSystem` on `&mut self` with the catalog's operation names; write methods default to `ReadOnly`, so ISO and UDF implement the same trait. It replaces `FsDriver`, `Access`, `AsDriver` and `OpenFile` (D6 is superseded).
- The bare tier tracks opens per node: `open(node)`, `read`/`write` at an offset, `close(node)`. Positions and append live in the caller or in `File`. The bare tier trusts the caller's open mode.
- `Volume<F>` has no lock parameter (sync uses the std mutex, async a portable async mutex needing only `alloc`); `LockKind` from 4.15 is dropped. A no_std sync user has the node API; `Volume::with_lock` can be added later.
- `truncate` stays separate from `setattr`, since they fail differently.
- UDF read-write arrives in 3.x through a new `UdfFs::mount_writable`; `mount` stays read-only in every version.
- `forget(node, count)`, generation, allocated size and device number are 3.0 (the catalog overrides D12). No-follow resolution is 3.0 (overrides 4.13).

Pass 2 (builders), accepted on 2026-09-24:

- One mode-independent `Tree` feeds every writer and `copy_tree(&Tree, &mut F, dir)`. A mounted volume becomes a tree with `read_tree(&Volume<F>, path)`, whose content is read lazily, so large conversions do not load file data into memory. Lazy content is readable only in the mode that produced it; the other mode fails with `Unsupported`.
- Every format has `fmt::plan(&tree, &opts)` and `fmt::{sync,async}::write(dev, &tree, &opts)`, both returning one `Report` (size, warnings, extents). FAT and exFAT write through format plus `copy_tree`.
- One path-carrying error, `PathError` (alloc), is used crate wide by writers, tree edits and host helpers. It replaces `host::Error`.
- Reproducible output comes from `with_time` and `with_seed` on the options; there is no clock parameter, and ids and GUIDs derive from time plus tree. mtime clamping lives in `host::TreeOptions`.
- `Hybrid` is a struct with constructors (`mbr`, `gpt`, `gpt_hybrid_mbr`), not an enum, so APM in 3.x is a new method.
- cpio has an alloc-free stream reader over a caller path buffer, a streaming writer, and `cpio::{sync,async}::read_tree`.
- Pass 1 changes: `BlockDevice` gains a defaulted `max_block_count` for growable outputs, and a fallible `host::FileDevice::new(File)` replaces `impl BlockDevice for File`, since `block_count` cannot fail.
- Open: `Content::source` for user lazy content is added later; the cpio stream entry has no dev/ino accessor yet; `copy_tree` fails with `AlreadyExists` on an existing name; `FatOptions` and `ExFatOptions` merge with the format pass's `FormatOptions`; cpio writing without alloc is 3.x.

Pass 3 (per-format extras), accepted on 2026-09-24:

- `format(&mut dev, &opts)` returns the geometry and needs no allocator; the caller then mounts with its own `MountOptions`. `FatOptions` and `ExFatOptions` are also the format options. The partition offset defaults to a new `BlockDevice::disk_offset`, and `with_partition_offset` overrides it.
- Checks are free functions on an unmounted device: `check(&mut dev, scratch, on_finding)` with a caller-lent scratch buffer and a callback per finding. Every format reports one shared `Finding` with a `Severity` (`Error`, `Warning`, `Notice`) and a per-format `Detail`, which uses the same codes as mount errors.
- Extras are inherent methods with the same name on every driver (`info`, `extents`, `records`, `read_raw`); the shared trait stays closed. The serial is read through `info()`. `Extent` is reused for file maps and record locations and gains a file offset and an unwritten flag.
- `detect` lists every format found, each with the error mount would give. `open` returns an `AnyFs` enum, which reaches extras by `match`.
- `Walk` stops at 1024 levels with `LimitExceeded`. It uses a heap stack under `alloc` and a caller-lent stack without it, through distinct constructors, not a feature switch.
- Extraction is `host::write_tree(dir, &tree)`, with no options until 3.x.
- Pass 1 changes: `BlockDevice::disk_offset`; `SetAttr` sets attributes; `MountOptions` gains `with_node_limit` and `backup_boot`; `Error` is `Clone` and `Copy` when the device error is; the trait contract gives a directory one id however it is reached.
- Pass 2 changes: format modules are no longer alloc-only, and builder items are gated one by one instead; `Tree` rejects `..`; `FileDevice` tracks whether its file is writable.
- Open: `FileDevice` does not know a partition's start offset for `/dev/sdb1`; `SystemArea` and `Guid` belong in the partition crate; the async module shows a subset; cpio extraction builds the whole tree in memory.

Pass 4 (embedded), accepted on 2026-09-24:

- Firmware gets separate modules, `hadris_fat::embedded` and `hadris_fat::exfat::embedded`, each with `sync` and `r#async`. They are built on the raw layer, not on `FatFs`, and need no allocator. The async variant takes a new `local::BlockDevice` whose futures are not `Send`.
- `Fat<D, const FILES: usize = 4>` and `ExFat<D, const FILES: usize = 4>` are separate types, so FAT-only firmware does not link the exFAT reader. Embedded exFAT is read-only in 3.0; 3.x writes arrive through a new entry point, and `mount` stays read-only in every version.
- Device blocks are 512 bytes; other sizes are refused with `Unsupported`. FAT sectors of 512 to 4096 bytes are read in 512-byte pieces through one 512-byte cache in the struct.
- `File` is a slot index consumed by `close`, with a 16-bit generation so a stale handle or one from another volume fails with `InvalidHandle`. A dropped `File` keeps its slot until `unmount`; `sync` and `unmount` still publish its size. `Dir` is `Copy` and holds no slot. Names are passed per call, with `create_dir_all(dir, path)` as the one path method; `list` takes a callback and lends each `Entry`, whose UTF-16 name lives on the call's stack. `Entry::node()` with `open_node` opens a listed file by its entry position, valid until the directory changes.
- The embedded API has its own `Options`: a `fn() -> DateTime` clock, a `fn(u16) -> u16` fold that defaults to `hadris_fat_raw::fold_ascii` with `fold_unicode` as a one-line opt-in, UTC offset, and CP437 as the default code page. The slot count is the const parameter.
- Format and check are the shared, already alloc-free `fat::sync::{format, check}`.
- Footprint (prototype estimate, device excluded): `Fat<(), 4>` is 760 bytes on thumbv7em and 776 on aarch64, `ExFat<(), 4>` 792 and 808: one 512-byte cache, the 80-byte geometry, the options and 32-byte FAT slots (48 for exFAT). The 16-bit generation fits in existing padding, so the sizes are those of the 8-bit layout. Both types are const-asserted under 2048 bytes. Stack and flash need the real crate, with `-Z emit-stack-sizes` and a size report on thumbv7em in CI.
- Changes forced on passes 1 to 3, all additive: `local::BlockDevice` without `Send` (with a `Partition` impl), `fat::raw::fold_ascii` and `fold_unicode`, and the `embedded` modules. No signature changed.
- Open: a combined FAT-or-exFAT type for SDXC firmware can be added later; the cancel safety of the embedded async API (NF-CANCEL-01) is not specified yet and must be tested in the real crate. Shared FAT folds Unicode while embedded folds ASCII, so a name that differs only in non-ASCII case matches in one tier and not the other; this is documented per tier.

---

## 5. Per-crate changes

Each section lists the API, then the V3 feature work. Items marked "3.x" may
land after 3.0.0 because their shape ships in 3.0. The shared vocabulary,
the `FileSystem` trait, `Volume`, the tree and the errors are in section 4;
this section covers what each crate adds.

### 5.1 `hadris-fat`

**Crates and modules.** `hadris-fat-raw` holds the I/O-free codecs and the
`raw::io` primitives for FAT12/16/32 and exFAT (4.15). `hadris-fat` has the
shared drivers, the format extras and the embedded API. exFAT is the
`hadris_fat::exfat` module with its own `sync`, `r#async` and
`embedded`, because its names (`ExFatOptions`, `Detail`, `Geometry`) differ
from FAT's and R5 keeps them out of the crate root.

```rust
let geo = fat::sync::format(&mut dev, &FatOptions::new().with_label("BOOT"))?;
let mut fs = FatFs::mount(dev, MountOptions::new())?;     // FatKind::{Fat12, Fat16, Fat32}
fs.info().kind(); fs.info().volume_serial();
fs.set_label(Some("DATA"))?;
fs.setattr(node, &SetAttr::new().with_attributes(Attributes::HIDDEN))?;
let n = fs.extents(node, 0, &mut extents)?;               // FIEMAP-style map
let exfat = hadris_fat::exfat::sync::ExFatFs::mount(dev2, MountOptions::new())?;

let mut scratch = [0u8; 4096];
let report = fat::sync::check(&mut dev, &mut scratch, |f| eprintln!("{f}"))?;
```

- `FatFs<D>` and `ExFatFs<D>` have one type parameter and need `alloc`. The node table is private (4.5), and the clock, UTC offset and code page are runtime `MountOptions`, so there are no `'static` borrows in the type and no `Sync` supertraits (#83). They implement `FileSystem` directly, with no macro (post-pass decision A).
- exFAT is a separate driver, not a `FatKind`. Its entry sets, allocation bitmap and up-case table share little with FAT12/16/32, so one type would branch on the kind in every method. Both share the codecs that do overlap. `ExFatFs` is stable in 3.0; `unstable-exfat` is gone (Q5).
- `format(&mut dev, &opts) -> FsResult<Geometry, D::Error>` needs no allocator; the caller then mounts with its own options (VOL-FORMAT-07). `FatOptions` (`with_kind`, `with_size`, `with_label`, `with_time`, `with_seed`, `with_cluster_size`, `with_sector_size`, `with_reserved_sectors`, `with_fat_count`, `with_root_entries`, `with_media`, `with_oem_name`, `with_serial`, `with_partition_offset`, `with_alignment`) is `Copy` with the label inline, and serves both `format` and the tree writer `fat::sync::write`. `ExFatOptions` is the same without the FAT-only knobs. They replace `FormatOptions`, `FatVolumeFormatter`, `FatFormatOptions`, `ExFatFormatOptions`, `format_exfat` and `ExFatLayoutParams`. The partition offset defaults to `BlockDevice::disk_offset`. Without `with_kind`, volumes below 16 MiB are FAT12, below 512 MiB FAT16, and larger ones FAT32; the cluster size starts from the V2 tables and doubles or halves until the count fits. Option errors are checked before any write.
- `check(&mut dev, scratch, on_finding) -> FsResult<CheckReport, D::Error>` runs on the unmounted device with no allocator. It reports each shared `Finding` (4.15) with a `fat::Detail` code through the callback. Cross-links and lost clusters need a bit per cluster: the scratch buffer covers a window of clusters, and the tree is walked once per window, so the findings do not depend on its size. The tree is walked without a stack by following `..` entries. A `repair` pass and an `analysis` module are 3.x.
- Extras: `info()` returns `fat::Geometry` (the type `raw::parse_boot` and `format` return), `extents`, `records`, `read_raw`, `was_dirty` (false on FAT12, which has no flag), `set_label(Option<&str>)` and `set_volume_serial`. FAT attribute bits are `stat().attributes()` and `SetAttr::with_attributes`; cluster chains are `extents`. This replaces `kind()`, `label()` as an inherent method, `set_label(&VolumeLabel)`, `fat_attributes`, `set_fat_attributes` and `cluster_chain`.
- The raw layer is the `hadris-fat-raw` crate: `FatKind`, `Geometry`, `RootLocation`, `parse_boot`, `boot_code`, `Slot`, `ShortEntry`, `LongEntry`, `lfn_checksum`, and the folding functions `fold_ascii` and `fold_unicode`; its `exfat` module adds `boot_checksum`, `set_checksum`, `name_hash` and `EntrySet`. `hadris-fat` re-exports only the raw items its own signatures use (`FatKind`, `Geometry`, `Detail`, `exfat::Detail` and `check`), not the crate (R12, Q13).
- Shared FAT folds Unicode case as Windows does, and shared exFAT compares through the volume's up-case table, with no folding option. The code page defaults to `Cp437`.
- The FAT sector cache is gone as a separate thing. Users wrap the device in `Cache<D>`. `FatSectorCache`, `CachedFat`, `with_cached_fat` and `fat_cache` are removed (#27).
- `expect("Fixed root info required ...")` sites return `ErrorKind::Corrupt`. exFAT internals (`allocate_cluster`, `sync_bitmap`, `parse_entry_set`) become private; the pieces tools need are in `raw`.

**Embedded API.** `hadris_fat::embedded::{sync, r#async}::Fat<D, const
FILES: usize = 4>` reads and writes FAT12/16/32, and
`hadris_fat::exfat::embedded::{sync, r#async}::ExFat<D, const FILES: usize =
4>` reads exFAT (4.15). Both are built on `hadris-fat-raw`, not on `FatFs`,
and need no allocator. Sync takes `sync::BlockDevice`, async takes
`local::BlockDevice`. `Options` (`new`, `read_only`, `with_clock(fn() ->
DateTime)`, `with_utc_offset`, `with_code_page`, `with_fold`) defaults to
UTC, CP437 and ASCII folding. `File` is a move-only slot index with a 16-bit
generation; `Dir` is `Copy`; `list` lends each `Entry` to a callback.
Formatting and checking are the shared `fat::sync::{format, check}`.

**Feature work.**

- Random-access writes, read-write handles, grow through `truncate`.
- Create the root label entry when absent, and write the BPB label.
- Exact free space on FAT12/16 by scanning, cached after first use.
- Removing an open node fails with `Busy`; a pinned node is removed and answers `NotFound` (4.5, Q3).
- Close audit items C2 (cancellation safety of compound async operations) and B3 to B7, or confirm they are fixed. The embedded async API reuses `raw::io`'s write ordering, so a dropped future should leave what a power cut leaves; the real crate must test it (NF-CANCEL-01).
- exFAT: done in step 12 and stable in 3.0. Embedded exFAT write is 3.x through a new entry point (NF-NOALLOC-02).
- TexFAT volumes (two FATs) are mounted, written with both copies kept equal, and formatted. TexFAT transactions and fsck repair: 3.x.

### 5.2 `hadris-iso`

**Reading.** One reader, `IsoFs<D>`, in each mode:

```rust
let mut iso = IsoFs::mount(dev, MountOptions::new())?;
let node = iso.resolve(b"/boot/grub/grub.cfg", Resolve::Lexical)?;
let mut buf = [0u8; 2048];
if let Some(catalog) = iso.boot_catalog(&mut buf)? {
    for entry in catalog.entries() { let image = iso.boot_image(&entry)?; }
}
iso.info().id(IsoId::Volume);                  // the PVD bytes
```

- `IsoReader` and `IsoImage` merge into `IsoFs`, which needs no allocator (NF-NOALLOC-03). It implements `FileSystem` with the write half left at its `ReadOnly` defaults. Node ids are record locations, so there is no node table.
- A mount uses Rock Ridge, then Joliet, then the primary tree, unless mount options choose (DIR-LOOKUP-01). Listings show the highest version of a name without `;N`, and `lookup` accepts an explicit `;N` (DIR-LOOKUP-03).
- ISO needs device blocks of at most 2048 bytes and refuses a 4096-byte device with `Unsupported` (IO-OPEN-01).
- Extras: `info()` returns `iso::VolumeInfo` (`block_size`, `volume_space_size`, `id(IsoId)`, `date(IsoDate)`, returning bytes, since PVD bytes are often not ASCII). `boot_catalog(&mut buf) -> Option<BootCatalog<'b>>` parses the El Torito catalog lazily from the caller's buffer, with `CatalogEntry`, `Platform` and the builder's `Emulation`. `boot_image(&entry)` returns an `Extent`, read with `read_raw`. `iso::SystemArea` over caller bytes lists MBR, GPT and APM entries of a hybrid image (BOOT-HYB-05). `records(node)` plus `read_raw` replaces `view.raw_record(node)`.
- `check(&mut dev, scratch, on_finding)` verifies an image offline with `iso::Detail` codes (CHECK-ISO-01). The CLI stops parsing boot records by hand.
- `IsoStr::as_str` returns `Result`. Panicking `best_choice` and `primary` are removed.
- Raw record and FID iterators, the Rock Ridge decoder, d-character codecs and descriptor selection move to `hadris-iso-raw` in 3.x (4.15).

**Writing.**

```rust
let opts = IsoOptions::new()
    .with_level(IsoLevel::L3)
    .with_joliet()
    .with_rock_ridge()
    .with_relocation(Relocation::RrMoved)
    .with_id(IsoId::Volume, "INSTALL")
    .with_el_torito(ElTorito::new().with_entry(BootEntry::bios("boot/eltorito.img"))
        .with_entry(BootEntry::uefi_appended(0)))
    .with_hybrid(Hybrid::gpt_hybrid_mbr().with_appended(AppendedPartition::esp(esp)))
    .with_time(epoch);
let report = iso::plan(&tree, &opts)?;
let report = iso::sync::write(&mut out, &tree, &opts)?;
```

- `IsoLevel::{L1, L2, L3}` replaces `BaseIsoLevel` and `EntryType`. Rock Ridge is present if and only if `with_rock_ridge` is set; Joliet if and only if `with_joliet` is set (long Joliet names are a 3.x method); the ISO 9660:1999 tree is `with_iso1999`, so "Level 3" means one thing. `supports_rrip`, `RripOptions.enabled` and `long_filenames` are removed.
- `with_relocation(Relocation::{RrMoved, DotRrMoved, Refuse})` offers only the two names libarchive reads (#123, #124).
- Descriptor ids and dates are `with_id(IsoId, &str)` and `with_date(IsoDate, DateTime)`, checked in `plan`, instead of eleven setters or a `VolumeIdentifiers` struct.
- `IsoOptions` has no clock parameter. `with_time` fixes every timestamp, and ids derive from the time plus the tree unless `with_seed` (4.10).
- `ElTorito` holds `BootEntry` values (`bios`, `uefi`, `uefi_appended`, `with_load_size`, `with_boot_info(BootInfo::{Table, Grub2})`, `with_emulation`), each with an image given as a tree path or, for EFI, an appended partition, so the ESP is stored once for El Torito and GPT (BUILD-ESP-01). `BootSectionOptions` and the tuple list are removed.
- `Hybrid` is a struct with constructors (`mbr`, `gpt`, `gpt_hybrid_mbr`) and `with_bootstrap` and `with_appended`, not an enum, so APM in 3.x is a new method. Soft boot problems are `WarningKind::Boot`; a bootstrap over 446 bytes or a load size past the image fails the plan.
- `sector_size`, `with_charset` and `with_min_blocks` are removed; no 3.0 action needs them.
- The writer runs in both modes and without `std`. The output is a `BlockDevice`; a two-pass `write_stream` that needs only `Write` is 3.x.
- `iso::Guid`, `SystemArea` and `TablePartition` belong in the partition crate's raw layer and move there (5.6).

**Sessions and modification.** `IsoModifier` is replaced by a session that
reads the newest session into a `Tree` whose content points back at existing
extents:

```rust
let mut session = hadris_iso::sync::Session::open(dev)?;
session.tree_mut().insert("new.txt", Node::file(Content::bytes(b"hi")))?;
session.tree_mut().remove("old.txt")?;
let report = session.write(&opts, SessionMode::Append)?;   // new session after the last one
// SessionMode::Rewrite rebuilds in place for rewritable media and images
```

`Session<D>` owns the device (`open(dev)` fails with `MountError`, which gives
it back) and has `tree`, `tree_mut`, `options`, `plan`, `write` and
`into_inner`. `Append` writes a real new session: data and descriptors after
the previous session, previous extents reused, and the descriptors at block
16 updated as growisofs does on overwritable media. `Rewrite` replaces V2's
in-place behaviour, keeps hybrid boot data and updates the backup GPT.
Remastering to a new file is `iso::sync::write(out, session.tree(), &opts)`,
so there is no second method. The sync `Session` needs `std`, because its
lazy content reads through the sync `Volume`.

**Feature work.** Joliet beyond the BMP, zisofs read and write, RRIP SF and
RR, Apple Partition Map in hybrid images, `write_stream`. All fit existing
shapes and can be 3.x.

**Visibility.** `PendingRecords`, `FileTreeWalker`, `WrittenFiles`,
`DirectoryId`, `MovedDirectory`, `RripBuilder`, `SystemUseBuilder`,
`ElToritoWriter`, `IsoCursor`, `convert_l1/l2/l3` become `pub(crate)`.

### 5.3 `hadris-udf`

- `UdfFs<D>` implements `FileSystem`: path lookup, `read` at an offset, and metadata with times, permissions and owners. It needs no allocator. Node ids are ICB locations.
- `UdfFs::mount` is read-only in every version, and the write half answers `ReadOnly`. UDF on random-access media is a real read-write filesystem, so 3.x adds `UdfFs::mount_writable` and overrides the write methods for Type 1 partitions, with no second modifier API. A read-write default on `mount` was rejected: an existing mount would start writing the integrity descriptor after a minor upgrade (NF-STABLE-02).
- Extras: `info()` returns `udf::VolumeInfo` (`revision`, `block_size`, `partitions`, `implementation`, `domain`, `id(UdfId)`, `recorded`, `integrity_recorded`, `volume_serial`), with `EntityId`, `PartitionInfo` and `PartitionKind`; `was_dirty`; `extents`, `records` and `read_raw`. UDF ids decode CS0 to `&str`.
- The writer takes the shared `Tree` and `UdfOptions` (`with_revision(UdfRevision)`, `with_id(UdfId, &str)`, `with_time`, `with_seed`) and returns the shared `Report`: `udf::plan` and `udf::{sync, r#async}::write`. `UdfRevision` has 2.50 and 2.60 variants that the plan refuses, so 3.x support turns an error into success without a new variant. `SimpleFile`, `SimpleDir` and `UdfReport` are removed.
- **Bridge writer.** The ISO 9660 and UDF bridge moves here from `hadris-cd`: `udf::plan_bridge(&tree, &IsoOptions, &UdfOptions)` and `udf::{sync, r#async}::write_bridge(dev, &tree, &IsoOptions, &UdfOptions)`. It takes the two format options directly, not a `CdOptions` holding both, and points the UDF volume at the extents of the ISO `Report` instead of reopening the output.
- The UDF serial is set at build time (`UdfId::VolumeSet` or the seed); there is no serial change on a mounted UDF volume in 3.0.
- Low-level descriptor writers (`write_lvid(location, close: bool)`, `write_fids`) become `pub(crate)` or move to the raw crate with enums instead of bools.
- No trait bounds on the `UdfFs` struct definition. `write` no longer needs `std`. The dead `modify.rs` is deleted.
- Reader coverage: UDF 1.50 and 2.01 reading, prevailing-descriptor selection, allocation-extent chaining, extended allocation descriptors, and the backup anchors and reserve sequence for `MountOptions::backup_boot()`.
- 3.x: VAT, sparing tables, metadata partitions (2.50 and later), stream directories and a UDF `check` (CHECK-UDF-01), all behind the same `UdfFs`.

### 5.4 `hadris-cd`

Removed (S3). Its one job, the ISO 9660 and UDF bridge image, is
`hadris_udf::plan_bridge` and `hadris_udf::{sync, r#async}::write_bridge`
(5.3). Metadata, symlinks and streaming inputs come from the shared `Tree`,
and the layout comes from the ISO `Report`, so `CdOptions`, `LayoutManager`
and the reopen-the-output step have no successor. `detect` reports a bridge
image as `ImageFormat::IsoUdfBridge`, then `Iso`, then `Udf`.

### 5.5 `hadris-ntfs`

- `NtfsFs<D>` implements `FileSystem` with the write half at its `ReadOnly` defaults and metadata including times. Security descriptors are native (`security_descriptor(node)`), since `Metadata` stays `Copy`.
- Errors are `Error<E>` with an `ntfs::Detail`; `NtfsError` is gone. `raw::*` is no longer glob re-exported. `attr` types with raw `u8`/`u32` codes move to `raw`; the public API uses enums.
- Native API for streams: `streams(node)` lists named data streams, and `read_stream_at(node, name, offset, buf)` reads one.
- `mount` seeks to the boot sector instead of reading from the current position (falls out of `BlockDevice`).
- `detect` in the umbrella reports `ImageFormat::Ntfs`. The record layer is exposed under `unstable-ntfs` (4.15). `open` and `AnyFs` do not reach NTFS in 3.0 ([Q12](#7-open-questions)).

**Feature work.** `$MFTMirr` fallback, compressed streams, reparse points
(exposed as symlinks where they are symlinks or junctions), keyed B-tree
lookup. Encrypted streams stay `Unsupported`. Write support is 3.x behind the
`FileSystem` write methods, and `$LogFile` replay comes with it. NTFS stays in
`unstable` until its read side passes a conformance slice (Q5).

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
    let fs_dev = hadris_part::sync::open(&mut dev, &p)?;   // Partition<&mut D>
}
```

- `MbrType(u8)` with associated constants replaces `MbrPartitionType` and the 256-variant `MbrPartitionTypeFull`.
- GPT type GUIDs move to `gpt::types`. `Guid` implements `FromStr`; the inherent `from_str` becomes `Guid::parse_const`. `Guid::random` exists only with `std`.
- `Gpt` fields are private. Edits go through methods that keep CRCs correct. Writing always computes CRCs.
- `HybridMbr` has private fields and `add_mirrored(..)`, identical with and without `alloc`.
- The six `*ReadExt`/`*WriteExt` traits and `PartitionInfoTrait` collapse into `read`, `write`, `create`, `scan` and `open` in each mode, and the layouts and CRC in the raw layer. `open` returns the shared `Partition<D>` (4.2), so the filesystem mounted on it formats with the right hidden sectors.
- Bool parameters (`bootable`) become `PartitionFlags`.
- The raw layer gains the system-area types ISO needs for reading hybrid images (`SystemArea`, `TablePartition`) and `Guid`, which `hadris-iso` re-exports.

**Layout builder.** One flow for "disk image with partitions":

```rust
let layout = DiskLayout::gpt(disk_guid)
    .with_alignment(Alignment::MiB1)
    .partition(PartitionSpec::new(gpt::types::EFI_SYSTEM, Size::MiB(100)).with_name("EFI"))
    .partition(PartitionSpec::new(gpt::types::LINUX_FILESYSTEM, Size::Remaining));
let disk = hadris_part::sync::create(&mut dev, &layout)?;   // build(block_count, block_size) + write
let mut esp = hadris_part::sync::open(&mut dev, &disk.partition(0).unwrap())?;
hadris_fat::sync::format(&mut esp, &FatOptions::new())?;
```

**Feature work.** Extended and logical MBR partitions (EBR chains), falling
back to the backup GPT when the primary is corrupt, UTF-16 partition names,
remove and resize, overlap checks in every edit. APM for hybrid ISO images is
3.x (BOOT-HYB-04).

### 5.7 `hadris-cpio`

```rust
let mut path = [0u8; 4096];
let mut reader = cpio::sync::Reader::new(StdIo(stdin), &mut path);
loop {
    while let Some(mut entry) = reader.next_entry()? {
        entry.path(); entry.metadata(); entry.read(&mut buf)?;
    }
    if !reader.next_segment()? { break; }          // microcode, then the main archive
}

let mut writer = cpio::sync::Writer::new(out, &CpioOptions::new().with_format(Format::Newc));
writer.append("etc/hostname", &Node::file(Content::bytes(b"box\n")))?;
let mut w = writer.append_file("big.bin", &SetAttr::new(), len)?;
w.write_all(chunk)?; w.finish()?;
let (out, report) = writer.finish()?;
```

- The reader needs no allocator: `Reader<'b, R>` over a caller path buffer, so the caller picks the buffer size (NF-NOALLOC-04). Entries borrow the reader and have `path`, `metadata`, `format`, `offset`, `data_offset` and `read`; dropping an entry skips the rest of its data. `next_segment()` continues after a trailer, so concatenated archives (microcode plus main initramfs) keep their boundaries visible. It replaces `continue_after_trailer()`.
- An archive that ends at an aligned entry boundary without a trailer stays valid, as the Linux initramfs format allows (`LINUX-INITRAMFS-NEWC:archive#optional-trailer`). A trailer cut off mid-entry is `Corrupt`.
- `skip_entry_data_owned`, `next_entry_alloc` and similar twins are removed.
- Sizes are `u64` at the API boundary, with `LimitExceeded` when a newc field overflows.
- The streaming writer (`alloc`): `append(path, &Node)` for known content, `append_file(path, &SetAttr, len) -> EntryWriter` for data produced while writing, and `finish() -> (W, Report)`, which returns the sink so segments can be concatenated (BUILD-CPIO-CAT-01). The tree writer is `cpio::sync::write(&mut sink, &tree, &opts)` like every other writer, writing at the sink's end; `cpio::plan` returns its report. `write_tree(&Tree)` goes.
- `cpio::{sync, r#async}::read_tree(&mut reader)` builds a `Tree` from a stream, applying `relative_path` and failing on `..`. It replaces `Tree::from_cpio`.
- `CpioOptions` (`with_format`, `with_time`) uses `Format::{Newc, Crc, Odc, Binary}` instead of `crc(bool)`. Odc read and write are new; `Binary` is read-only and refused by the writers.
- `check(source, scratch, on_finding)` validates a stream with `cpio::Detail` codes (CHECK-CPIO-01).
- Hard links follow GNU cpio: data on the last link, metadata copied from the target.
- `RawNewcHeader::build` takes a `NewcFields` struct.
- The `hadris cpio` subcommand supports `-` for stdin and stdout.
- 3.x: cpio write without `alloc` (NF-NOALLOC-05), `Entry::link_id()` for stream consumers that pair hard links themselves.

### 5.8 `hadris-block` and `hadris-optical`

Removed (S2). Detection and opening move into the umbrella crate (5.9):

- `detect(&mut dev) -> FsResult<Detection, D::Error>` reads a few blocks and never writes. `Detection` (`first`, `iter`) lists every format found, most specific first, with no allocation: a hybrid ISO is `Iso` then `Gpt` or `Mbr`, a bridge image `IsoUdfBridge`, `Iso`, `Udf`. Each `Candidate` has its `ImageFormat` and, when its first structures are damaged, the error mount would give (`damage()`, kind `Corrupt` with the format's detail code), so "damaged" never reads as "not this format" (D5).
- `ImageFormat` is one non-exhaustive enum for block, partition, optical and archive images: `Fat(FatKind)`, `ExFat`, `Iso`, `Udf`, `IsoUdfBridge`, `Cpio(Format)`, `Mbr`, `Gpt`, `Ntfs`. `FatVariant` and `BlockFormat::Fat(FatVariant::ExFat)` are gone (D4).
- `open(dev, MountOptions) -> Result<AnyFs<D>, MountError<D, D::Error>>` (`alloc`) mounts the first filesystem `detect` finds with the caller's options. A device holding only a partition table or an archive fails with `NotRecognized`, giving the device back. So does an NTFS volume, with the message `"ntfs"`, while `detect` still lists it (Q12).
- `AnyFs<D>` is a non-exhaustive enum (`Fat`, `ExFat`, `Iso`, `Udf`; `Ntfs` is added once NTFS is stable, Q12) that implements `FileSystem` and reaches format extras by `match`. It is an enum, not a `Box<dyn FileSystem>`, because the async trait is not dyn-compatible. It replaces the `OpenVolume` and `OpenOpticalImage` structs and their `as_*` accessors.

### 5.9 `hadris` (umbrella)

- Re-exports `hadris-io`, `hadris-storage` and `hadris-fs`, and each format crate at a flat path behind a feature of its name (`hadris::fat`, `hadris::iso`, `hadris::udf`, `hadris::cpio`, `hadris::part`; `hadris::ntfs` behind `unstable-ntfs`).
- `hadris::{sync, r#async}::{detect, open, AnyFs}` (5.8).
- `hadris::host` (`std`, sync only, 4.15): `open(path)` detects and mounts read-only as `AnyFs<FileDevice>` with `host::mount_options()`; `FileDevice` (`open`, `new`, `into_inner`); `read_tree`, `write_tree`, `TreeOptions`, `Symlinks`, `OnError`; `file` and `source_date_epoch` for builders; `mount_options()` and `local_utc_offset()`, the host defaults of post-pass decision C; and `StdIo`. Its errors are `PathError` with the host path set.
- The umbrella has no binary. The `hadris` binary is its own package, `hadris-cli` (`cargo install hadris-cli`, Q15), so library users never compile clap and CLI changes do not move the umbrella's version. It replaces the five CLI crates, with subcommands `fat`, `iso`, `udf`, `cpio` and `detect` and one set of flags, overwrite rules and output handling (S1); `extract` takes `-p/--path` in every format. The 2.x binary names are not installed.

---

## 6. Migration plan

Each step is one PR against the long-lived `next` branch, so `main` keeps
shipping 2.x fixes. Bugs found during the survey are fixed on 2.x first, on `fix/survey-bugs`:
path traversal in the cpio and UDF CLI extract commands, and GPT CRCs of 0
without the `crc` feature. Locks held across `.await` cannot be fixed without
the V3 changes in 4.2 and 4.4 (the device lives inside the mutex and every
await is I/O on it), so 2.x documents async volumes as single-task.

Steps 1 to 13 are done and record what was built before the action catalog
and the API prototype (4.18). They use the names of that time (`FsDriver`,
`impl_fs_driver!`, `NodeTable`, `LockKind`, `AnyError`, `hadris-block`, `hadris-cd`,
the `async_send` mode); sections 3 to 5 describe what replaces them, and the redesign steps after step 13 carry the
code over.

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
The implementation redesign then brings the code to sections 3 to 5, one PR
into `next` per step, each leaving the workspace building and tested:

- R1. **Docs.** Sections 3, 4.1 to 4.14 and 5 rewritten from the prototype.
- R2. **Error shape.** `Error<E>` with its static context and `Detail::of`, `NotRecognized`, `Errno`, `PathError`, and the storage errors folded in (decision B). Almost every later signature depends on this. Done in three PRs: the error, kinds, `Errno`, `Location` and `DetailCode` live in `hadris-io` and `BlockDevice` returns them, with `WriteError`, `StorageError` and `OutOfRange` gone (Q10, Q11); `AnyError` became `PathError`; and every format crate dropped its `Error` wrapper for a `Detail` of numbered codes read with `Detail::of` and `Detail::from_code`. The ISO 9660, UDF, cpio and hybrid writers return `PathError` with the failing file's path; `hadris-block` and `hadris-optical` return `MountError` and pass a driver's refusal through unchanged; bytes that are not the format fail with `NotRecognized` in every reader, FAT and exFAT included. Deferred: `fat::Detail`, shared with the check findings, to R4; per-entry paths from `copy_tree` to R6.
- R3. **Devices.** `BlockDevice` gains `max_block_count`, `disk_offset` and `writable`; `local::BlockDevice`; `Partition`; `host::FileDevice` replaces the `std::fs::File` device. Done in one PR: the three methods are defaulted (`block_count`, 0, false) and forwarded by `&mut D`, `Box<D>`, `Cache` and `Partition`, and FAT and exFAT mount a device that is not `writable` read-only. `Vec<u8>` is a growing device with 512-byte blocks. `Partition<D>` is one byte-window type for every mode, adds its offset to `disk_offset` and replaces `Slice`, and `hadris_part::open` returns it. `hadris-io` and `hadris-storage` gain `local` modules generated from the `r#async` source, which R5 leaves in place when `async_send` becomes `r#async`. `hadris_storage::host::FileDevice` (`open`, `new`, `into_inner`) measures its size with `host::file_len`, tells whether `new`'s file accepts writes with a write of no bytes, and grows an image file; the umbrella re-exports it from `hadris::host` in R6. Deferred: formatting defaults the FAT hidden sectors and exFAT partition offset to `disk_offset` when `FatOptions` and `ExFatOptions` become the format options, since it changes the bytes a partition format writes; writers check the planned size against `max_block_count` in R6; `FileDevice::open` returns `std::io::Error`, not `PathError` with the host path, until the host module decides.
- R4. **Raw layer.** The FAT and exFAT codecs move into `hadris-fat-raw` with `raw::io`, and `check` moves onto it. Done in six PRs. `hadris-fat-raw` has its own version (0.1.0) and needs neither `std` nor an allocator; it holds the codecs, and `io` and `exfat::io` hold the device primitives. The primitives are generated for `sync`, `r#async` and `async_send` from one source and borrow a caller-lent `BlockBuf`:
  - FAT: `read_geometry`, `read_fat`, `get`, `get_copy`, `set` (active copy first) and `mirror`; `next`, `walk` and `run` (the `runs` above); `allocate`, `allocate_run` and `free_chain`, batched per device block with their progress in a `Held`; `slot_offset` with a `DirWalk` that keeps its chain position; `read_slot`, `write_slots`, `clear_slots`, `count_free`, `write_fs_info` and `mkfs`.
  - exFAT: `read_boot` (with the backup region) and `read_volume`; a lazy `Upcase` index with `upcase`; the bitmap (`bit`, `set_bit`, `bitmap_bytes`, `count_free`); `allocate`, `allocate_run` and `free_chain`; `write_set` (the secondary entries' blocks before the File entry's) and `clear_set`; `slot_offset` with its own `DirWalk`; and the `VolumeDirty` and `PercentInUse` writes.

  `FatFs` and `ExFatFs` are rebuilt on these primitives and keep their write order. `check(&mut dev, scratch, on_finding)` is the former checker of each format, moved onto the primitives. It runs on an unmounted device with no allocator, reports `hadris_fs::Finding`s and returns a `CheckReport`, and `hadris_fat::sync::check` and `hadris_fat::exfat::sync::check` re-export it. `Finding`, `Severity` and `CheckReport` are in `hadris-fs`. `Detail` and `exfat::Detail` are in `hadris-fat-raw`, re-exported as `hadris_fat::Detail` and `hadris_fat::exfat::Detail`, and mount errors, read errors and findings share them (the R2 deferral).

  Decisions:
  - A `Finding` carries a `DetailCode` rather than a bare `u16`, like `Error`, since every format shares the type. exFAT's domain is `hadris-fat::exfat`.
  - `CheckReport` has only `findings` and `passes`, so the CLI takes cluster counts from `stats` and file counts from a walk.
  - The first 1 KiB of `scratch` holds paths, and at least 512 bytes more hold the bitmap. A path is the long names in UTF-8, or the short names with their stored bytes, cut at the last whole name that fits.
  - A damaged FAT32 boot sector or exFAT main boot region is a finding, and the check goes on from the backup.
  - A clear FAT16 or FAT32 clean-shutdown bit is a `Dirty` notice.

  Deferred:
  - `format(&mut dev, &opts) -> Geometry` with `FatOptions`, to R6, together with the `disk_offset` default for hidden sectors and the exFAT partition offset (R3's deferral).
  - Date decoding with a UTC offset, to R5 (done there).
  - `FatFs` still folds names with its own char fold, not `fold_unicode`.
  - `exfat::io` is tested through `ExFatFs`, since the raw crate has no exFAT image of its own.
  - `hadris_fat::raw` re-exports the whole raw crate, and the mode modules re-export its `check`, which ties `hadris-fat`'s API to the raw crate's version.
- R5. **Trait and volume.** `FileSystem` replaces `FsDriver` and its companions, `FatFs` and `ExFatFs` collapse to one parameter, the node tables go private, the lock kinds and resolvers go, and the `async_send` mode becomes `r#async`. Done in four PRs:
  - #178: the `async_send` modules become `r#async` and the non-`Send` `r#async` modules and the `async-send` feature go; `local` stays in `hadris-io`, `hadris-storage` and `hadris-fat-raw`.
  - #179: `FileSystem` (4.3) and `Volume<F>` (4.4) replace `FsDriver`, the `&self` trait, the access tiers, `DriverExt`, `PathExt`, the resolvers, the lock kinds, `impl_fs_driver!` and `path`. The vocabulary follows 4.3: `Name::check`, a non-zero `NodeId`, an inline-named `DirEntry`, `Capabilities::stores`, a `Metadata` builder, `Permissions`, `SetAttr`, `OpenMode`, `RenameMode` and `Resolve`. Every driver implements the trait directly, as do `OpenVolume` and `OpenOpticalImage`.
  - #180: `FatFs<D>` and `ExFatFs<D>` with `mount(dev, MountOptions)` and `unmount`; `MountOptions`, `CodePage`, `Ascii` and `Cp437` in `hadris-fs`; the node table private with `with_node_limit`; CP437 as the default code page; and the date decoding with a UTC offset that R4 deferred.
  - A docs PR: the website guides and this entry.

  Decisions where the spec was silent or the code differs from it:
  - `SetMetadata`, `FileTimes` and `DeviceKind` stay for the writer `Tree` until R6 replaces it; `Mode` became `Permissions` there too.
  - `copy_tree` and `import_from_host` copy files and directories only. A symlink, device node, FIFO or socket fails with `Unsupported`, since the trait cannot create them.
  - `File` implements the `hadris-io` `Read`, `Write` and `Seek` in both modes and the `std::io` traits in `sync`. The async `ReadDir` has `next_entry`; the sync one is also an `Iterator`. A dropped async `File` or `ReadDir` is released by the next call on the volume.
  - The contract kit also checks that invalid names fail with `InvalidInput` and that opening a directory fails with `IsADirectory`, and it keeps a pin on every node it checks.
  - `MountOptions::with_utc_offset` returns `Result`, failing past a day, so a bad offset never reaches a driver. `utc_offset() == None` is UTC with no offset recorded, which keeps the times earlier builds read.
  - Only `FatFs` and `ExFatFs` gained `mount` and `unmount` here. `IsoImage`, `UdfFs` and `NtfsFs` keep `open` until R7 brings `IsoFs` and `AnyFs`, and `backup_boot` is stored but no driver reads it yet (exFAT already falls back to its backup boot region).
  - `FatFs`, `ExFatFs` and `format` need `alloc`. Without it `hadris-fat` keeps `check` and the raw layer, and `hadris-block` keeps detection, until R6's `format` returns `Geometry` and R8's embedded API covers firmware.
  - The inherent FAT label getter is `volume_label()`, returning the stored `VolumeLabel`, so it does not shadow `FileSystem::label`. R7's `info()` and `set_label` replace it. A FAT label with bytes that are not ASCII reads as `Some("")` through the trait (Q13 changed this after R5: the label is decoded through the code page).
  - An ISO view's `label` decodes the volume identifier of the descriptor it reads (UCS-2 for Joliet, Latin-1 otherwise) without name mangling. UDF's is the logical volume identifier.
  - ISO and UDF `readdir` read each entry's metadata, so a damaged UDF child entry fails the listing it is in.
  - A clock or code page is a `&'static dyn` reference, as 4.3 says; a clock with runtime state is a `static` or leaked by the caller.

  Deferred: `hadris_fat::raw` still re-exports the whole raw crate (Q13, resolved after R5: it no longer does); `experiments/fuse-prototype`, outside the workspace and CI, still uses the removed API.
- R6. **Builders.** The new `Tree`, `plan` and `write`, `Report`, `copy_tree`, `read_tree` and the `host` module. Done in four PRs:
  - #184: Q13, the raw re-exports and code-page labels.
  - #185: `Tree`, `Node`, `Content`, `TreeEntry`, `ContentReader`, the shared `Report` with `Warning` and `WarningKind`, `copy_tree` with paths in its errors, `read_tree` from a `Volume` with lazy content, and the `host` module (`read_tree`, `write_tree`, `file`, `source_date_epoch`, `local_utc_offset`, `mount_options`), also under `hadris::host` item by item. The ISO 9660, UDF, bridge and cpio writers take the `Tree`: `plan` at the crate root does no I/O, and `write` plans again, checks `max_block_count` and every file's content, then writes. Writers take `with_time` and read no clock. The bridge moved into `hadris-udf` (`plan_bridge`, `write_bridge`), and `hadris-cd` wraps it until R7. cpio gained `Writer::append`, `append_hard_links` and `append_file`, and `read_tree`. `SetMetadata`, `FileTimes`, `DeviceKind` and the per-crate trees and reports are gone.
  - #186: FAT and exFAT `format(&mut dev, &opts) -> Geometry` without an allocator, `FatOptions` and `ExFatOptions` as 5.1 lists them, the `disk_offset` default R3 deferred, the `max_block_count` check, and `write(dev, &tree, &opts)` through `format` and `copy_tree`.
  - A docs PR: the website guides and this entry.

  Decisions where the spec was silent or the code differs from it:
  - `Content::stored`, `Content::stored_extents` and `ContentReader::check` are public: the bridge writer in `hadris-udf` builds on ISO extents, and every writer checks content before it writes.
  - `Tree` also has `entry` and `root`, returning a `TreeEntry` (`node`, `id`, `links`, `children`, `child`), since writers and `copy_tree` walk trees and see hard links.
  - `Report` has `new`, `set_size`, `push_warning` and `push_extent`, so format crates and `copy_tree` build it. Extent keys have no leading slash; warning paths keep the writer's form, `/a/b` (Q14 changed this after R6: both use the form `Tree::insert` takes).
  - `PathError` carries tree paths as bytes and host paths separately (`host_path`).
  - `copy_tree` does not apply the root's attributes to `dir`, and sets attribute bits with the times after the data, because writing sets FAT's archive bit.
  - `read_tree` of a single file names it by the path as given, not the stored name; the FAT CLI renames it (Q14 changed this after R6: it uses the stored name).
  - cpio's `CpioWriter` is `Writer`, and `Format::NewcCrc` is `Format::Crc`. `append_file` refuses `Crc` with `Unsupported`, since the checksum precedes the data. `CpioReader` is unchanged.
  - Serials and GPT GUIDs derive from `with_seed`, or from the time, not from the time plus the tree as 4.10 says (Q14 changed this after R6: they hash the tree too).
  - FAT and exFAT have no `plan`; `write` reports the volume size the options give (Q14: none in 3.0, additive later).
  - `FatOptions::with_partition_offset` and `ExFatOptions::with_partition_offset` take bytes, like `disk_offset`, and must be whole sectors. `with_label` takes a checked `VolumeLabel`, not `&str`. A failed `format` returns only the error, since it borrows the device.
  - FAT and exFAT `write` give nodes without times the options' time, so the volume does not depend on the mount clock.
  - `write_bridge` checks the UDF block size before the ISO part.
  - The host extract commands of the CLIs skip device nodes, FIFOs and sockets with the warnings `host::write_tree` reports.
  - `host::file` measures a device such as `\\.\PhysicalDrive2` with `file_len`, since its metadata fails on Windows.

  Deferred: `Session::plan` and sessions over a lazy `Volume`; the ISO and UDF option reshape (`IsoId`, `Hybrid`, `ElTorito`, `UdfId`, UDF `with_seed`) to R7; `WarningKind::Deduplicated`, which no writer emits yet; the cpio reader reshape; `experiments/fuse-prototype` still uses the removed API. Open points are [Q14](#7-open-questions).
- R7. **Extras and crate merges.** `detect`, `open` and `AnyFs` in the umbrella, the `info` and `extents` family, `Walk`, the removal of `hadris-cd`, `hadris-block` and `hadris-optical`, and the single `hadris` binary. Done in nine PRs:
  - #188: Q14.
  - #189: mounting. `IsoImage` and `IsoView` merge into `IsoFs` with `mount`, `mount_namespace` and `unmount`; `UdfFs` and `NtfsFs` trade `open` for `mount` and `unmount`. `MountOptions::backup_boot` mounts FAT32 from sector 6 and exFAT from its backup region, both read-only, and makes UDF read the end anchors and the reserve sequence first; ISO has no backup structures and NTFS ignores it. A UDF entry whose file entry is damaged is listed with the type its identifier records and fails `stat`, so one bad entry no longer fails the listing.
  - #190, options: the ISO and UDF reshape of 5.2 and 5.3. `IsoId`, `IsoDate`, `with_joliet()`, `with_rock_ridge()`, `with_relocation`, `with_iso1999`, `ElTorito::new().with_entry`, `BootEntry::{bios, uefi, uefi_appended}`, `with_boot_info(BootInfo::{Table, Grub2})`, `Hybrid::{mbr, gpt, gpt_hybrid_mbr}` with `with_bootstrap` and `with_appended(AppendedPartition::esp(content))`; `VolumeIdentifiers`, `RockRidge`, `Charset` and `HybridBoot` are gone. `UdfId` with `with_id` and UDF `with_seed`, whose serial leads the volume set identifier.
  - #191, Walk and FAT extras: `Walk` with `WalkEntry` and `WalkFrame` in `hadris-fs`, `Extent` with `file_offset` and `unwritten`, and on `FatFs` and `ExFatFs` `info`, `was_dirty`, `extents`, `records`, `read_raw`, `set_label` and `set_volume_serial`; `kind`, `volume_label`, `volume_id`, `cluster_size` and `cluster_chain` are gone.
  - #192, ISO and UDF extras: `IsoFs::info` (`VolumeInfo`), `boot_catalog(&mut buf)` returning a borrowing `BootCatalog` with `CatalogEntries`, `boot_image`, `records` and slice `extents`; `UdfFs::info` (`VolumeInfo` with `EntityId`, `PartitionInfo`, `PartitionKind`), `was_dirty`, `records`, `read_raw` and slice `extents`. `IsoId`, `IsoDate` and `UdfId` no longer need `alloc`.
  - #193, detection: `hadris::{sync, r#async}::{detect, open, AnyFs}` and `ImageFormat`, `Detection` and `Candidate` at the umbrella root, behind a new default `detect` feature that adds `fat`, `iso`, `udf` and `cpio`; `hadris::host::open(path)`.
  - #194: `hadris-block`, `hadris-optical` and `hadris-cd` are removed with the umbrella's `block`, `optical` and `cd` features. The bridge test and the ECMA TR/71 catalog move to `hadris-udf`, and the umbrella tests open FAT through a GPT partition.
  - #195: `hadris-cli` installs one `hadris` binary with `fat`, `iso`, `udf`, `cpio` and `detect`, and the five CLI crates and their binaries go.
  - A docs PR: this entry.

  After R7 the workspace publishes `hadris-io`, `hadris-storage`, `hadris-fs`, `hadris-common`, `hadris-macros`, `hadris-fat-raw`, `hadris-fat`, `hadris-part`, `hadris-ntfs`, `hadris-iso`, `hadris-udf`, `hadris-cpio`, `hadris` and `hadris-cli`. The umbrella re-exports `io`, `storage` and `fs` whole (the stable base), each enabled format crate at a flat path, the error items at its root, `ImageFormat`, `Detection` and `Candidate`, `sync` and `r#async` with `detect`, `open` and `AnyFs`, and a `host` module that re-exports `hadris-fs` host items, `StdIo` and `FileDevice` one by one.

  Decisions where the spec was silent or the code differs from it:
  - `MountOptions` has no namespace field, so choosing an ISO tree is `IsoFs::mount_namespace(dev, options, Namespace)`; `mount` takes the most capable tree. Open point: a `MountOptions` field would reach `open` and `AnyFs` too.
  - With `backup_boot`, a volume whose boot sector reads as FAT12 or FAT16 mounts from it, since those have no backup; a FAT32 volume without a valid backup fails with `Corrupt`.
  - `IsoOptions::with_min_blocks` stays, against 5.2: the bridge writer in `hadris-udf` needs it across the crate boundary. `UdfOptions::with_min_blocks` stays too, for free space after the files.
  - Rock Ridge's metadata choice is `IsoOptions::with_preserve(Preserve)`, since the `RockRidge` struct is gone; `NameCase`, `BootEntry::with_platform`, `with_load_segment` and `Hybrid::with_flags` stay, as nothing in 5.2 replaces them.
  - `IsoId` covers the nine descriptor identifiers, the copyright, abstract and bibliographic files included; `IsoDate` the four dates. Identifiers are stored as given (xorriso's behaviour, the old `Charset::Relaxed`) and `plan` checks their length; the ISO CLI's `--strict-charset` maps them itself.
  - Appended partitions go after the files inside the ISO 9660 volume, not after it as xorriso's `-append_partition` does, so the volume space size covers them and the GPT splits the ISO partition around the ESP as before. Only GPT tables list them; an MBR table with one fails with `Detail::HybridBoot`. `with_efi_partition` is gone: the ESP is the first appended partition, else the image of the only UEFI entry after the default one.
  - `with_joliet()` writes Joliet level 3; `JolietLevel` stays for reading.
  - `UdfId` has `Volume`, `VolumeSet`, `LogicalVolume` and `FileSet`; `LogicalVolume` and `FileSet` default to `Volume`, and `VolumeSet` to the 16-digit serial followed by the volume name. Each fails with `Detail::Identifier` when it does not fit its field, where the old single id was cut to 30 bytes.
  - `info()` returns a reference to the parsed descriptor. `extents(node, from, &mut [Extent])` fills whole extents that end after file offset `from` and returns how many; the caller continues from the end of the last. `records(node, &mut [Extent])` fails with `LimitExceeded` when the buffer is too short. A FAT file's records are its short entry only: its long-name entries sit before it in a directory its id does not name.
  - `set_label` takes `Option<VolumeLabel>`, not `Option<&str>`, as `FatOptions::with_label` does since R6. FAT `Geometry::volume_serial()` is an `Option`, since a FAT12/16 boot sector without an extended boot signature has no serial; `set_volume_serial` fails with `Unsupported` there.
  - `Walk::next` returns a `WalkEntry` (the `DirEntry` and its depth) and has `skip_dir`. `Walk::with_stack` lists as many levels as it has frames. The walk pins nothing, like `readdir`.
  - `iso::VolumeInfo::date` returns `Option<DateTime>` decoded from the descriptor, not bytes; `id` returns the stored bytes without trailing spaces. `BootCatalog::entries()` is an iterator over the checked catalog in the caller's buffer, and `boot_catalog` fails with `LimitExceeded` when the catalog does not fit. `boot_image` gives the diskette size for floppy emulation and the loaded sectors otherwise, since a no-emulation entry does not record its image's length. `IsoFs::descriptor(index)` and `rock_ridge(node)` stay.
  - ISO `records` gives the record a node id names (a directory's `.` record) and the records that follow it for a multi-extent file. UDF `records` gives the file entry block. `udf::VolumeInfo::volume_serial` reads the first 16 characters of the volume set identifier as hexadecimal (UDF 2.2.2.5) and is `None` when they are not. `was_dirty` is false when the integrity sequence cannot be read.
  - `detect` needs all four format features, through the `detect` feature, so what it recognizes never depends on the features enabled. It checks damage by mounting ISO 9660 and UDF read-only and by reading the FAT or exFAT boot region through the raw layer, without allocating; a reader's `NotRecognized` on a recognized signature is reported as `Corrupt` with the reader's message and detail. A protective MBR without the `EFI PART` header lists nothing, and a hybrid one without it only `Mbr`. `Mbr`, `Gpt` and `Cpio` carry no damage.
  - `open` tries the filesystem candidates in order with the caller's options and returns the first that mounts, so a bridge with a damaged UDF side opens as ISO 9660; when none mounts it fails with the first one's error. A bridge opens as UDF. Messages for devices without a filesystem: `"ntfs"`, `"partition table"`, `"archive"`, `"no filesystem recognized"`.
  - `AnyFs` has `unmount` and `into_inner` besides the trait. A test in the umbrella reads the trait's method list and fails until `AnyFs` forwards a new method.
  - NTFS gets no `info`, `extents`, `records` or `read_raw` in 3.0: it stays behind `unstable-ntfs` (Q12) and keeps `volume_serial` and its geometry getters. `iso::SystemArea` and the ISO `check` are deferred to 3.x.
  - The R5 note that a damaged UDF child entry fails its listing no longer holds: since #189 the entry is listed with the type its File Identifier Descriptor records and empty metadata, and `stat`, `open` and `lookup` of it fail with `Corrupt` (`damaged_entries_behind_listed_ids_are_corrupt`). A damaged File Identifier Descriptor itself still fails the listing at that point, since the lengths that locate the next one cannot be trusted.
  - The binary is the `hadris-cli` package, not a target of the umbrella library, so library users never compile clap and the CLI versions apart (Q15).
  - The CLI rules (S1 and triage G1): `create` takes the source and `-o/--output` and refuses an existing output unless `-f/--force` is given (FAT refused and the others replaced before), then replaces a file atomically or writes a device in place; unreadable source entries are skipped with a warning; `extract` never replaces an existing file, so cpio extraction refuses one too while still replacing what an earlier entry of the same archive created; in-image paths accept `/a`, `./a` and `a` in every format; `-V/--volume-name` names a volume; `list` and `check` stay aliases of `ls` and `verify`.
  - The bridge commands are `hadris udf bridge` and `hadris udf compare`, following the writer into `hadris-udf`. `bridge` takes the `iso create` flags and defaults (level 1, Joliet with `-J`), not `hadris-cd`'s level 2 with Joliet and ISO 9660:1999. `hadris-cd info` has no successor beyond `hadris detect` and the `info` commands.
  - `hadris detect` prints each candidate with its damage and fails when nothing is recognized.
- R8. **Embedded.** `Fat` and `ExFat` on the raw layer, with cross-target CI for size and stack.

14. **3.0.0-rc.1.** CI guardrails become blocking. Migration guide (`docs/hadris-3.0.0-migration.md`) with a V2 to V3 symbol table.

`hadris-vfs` and the 3.x feature items follow 3.0.0.

---

## 7. Open questions

**Q1. Lock placement.** Resolved. Four variants were prototyped on
`feat/v3-lock-prototype` under `experiments/lock-placement`: an external
wrapper over `&mut self` (A), a lock inside each format (B), a `&mut` driver
trait plus a `&self` trait with a shared `Volume` (C), and C with opt-in tiers
and device-typed errors (D). V3 took D, and scenarios S1 to S16 were its
acceptance tests. The API prototype (4.18) then kept D's external lock and
opt-in tiers but dropped the `&self` twin trait and the lock kinds: one
`FileSystem` trait on `&mut self` and `Volume<F>` with a fixed lock per mode
(4.3, 4.4).

**Q2. `Send` futures.** Resolved: the shared tier's `r#async` mode has `Send`
futures whenever the device is `Send`, and non-`Send` async is the `local`
device traits plus the embedded API (4.8, 4.17). History: `async fn` in
traits does not let a generic caller require `Send` futures, so tokio code
that spawns over a generic `F: FileSystem` did not compile (E2 baseline: 25
errors, "`<F as FileSystem>::sync` is an `async fn` in trait, which does not
automatically imply that its future is `Send`"). E2 tried every option:

| Option | Result |
|---|---|
| `trait_variant` 0.1.3 | Does not compile: default bodies are not wrapped in `async move`, and its blanket impl conflicts with the `&`, `&mut`, `Box` and `&F` forwarding impls |
| The same pattern by hand | A format crate can implement the `Send` variant or the local one for `FatFs<D>`, not both, so "`Send` when the device is" cannot be written |
| `+ Send` in the async traits | Works, but rejects every non-`Send` device: an `Rc` device, a `RefCell` volume, any embedded-io-async adapter |
| Return type notation | Unstable (E0658 on 1.98.1). On nightly a blanket `SendFileSystem` alias works with no trait changes |
| Spawn concrete types only | Works; a generic function that calls `spawn` cannot compile |
| **A third mode, `async_send`** | Works. The same source is generated a third time with `Send` futures and supertraits |

Step 4 added `async_send` as a third mode. The layering pass (Q9) then
dropped the non-`Send` async mode from the shared tier: its users are
firmware on single-threaded executors, which now have the embedded API. So
the `Send` form became the only shared async mode, named `r#async`, and the
non-`Send` form survives only where firmware needs it (`local`). When return
type notation is stable, nothing changes for users: the async trait is
already `Send`.

**Q3. Removing an open file.** Resolved, revised in step 6. POSIX semantics
(entry gone, clusters freed at the last close) need orphan tracking, and a
crash leaves lost clusters that fsck has to reclaim, so removing the last
name of an open node fails with `ErrorKind::Busy`. The first answer made any
pin block removal; the FUSE prototype showed that a kernel pins every cached
name, so every `rm` failed. Now a node is open between `open` and `close` (as
`File` holds it), a pinned node that is only looked up is removed, and its id
answers `NotFound` until its last `forget`. Orphans can come in 3.x without a
break, since they only turn `Busy` into a success; `forget` stays I/O-free
because freeing happens in `close` or `sync`.

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
a `NameCodec`? Still open. The 3.0 answer is `Capabilities::charset()`
returning a non-exhaustive `Charset` (`Bytes`, `Unicode`) plus
`max_name_bytes` in UTF-8 bytes; a finer description is additive.

**Q7. Driver type names.** Resolved: `<Format>Fs` for drivers (`FatFs`,
`ExFatFs`, `IsoFs`, `UdfFs`, `NtfsFs`), since `Volume` is the sharing wrapper
and what most users type. `FatVolume`, `UdfVolume` and `NtfsVolume` would read
as if they were already shared.

**Q8. FAT node table capacity.** Resolved by the layering pass (4.15): the
node table is private, grows on the heap, and has an optional cap
(`MountOptions::with_node_limit`); `FatFs<D>` needs `alloc`, and firmware uses
the embedded API (4.5). Before that, step 5 used a const-generic fixed array
(E1), and the step 6 review made the table a public `NodeTable` type
parameter with `FixedTable<64>` as the default and `HeapTable` under `alloc`.
The fixed default ran out under a FUSE mount, and the type parameter showed
up in every hosted user's types.

**Q9. Layer boundaries.** Resolved 2026-09-24 by the user, in two rounds.
First: host volumes wrap `Volume` with `StdMutex`, `host::Error` replaces
`AnyError`, the host FAT code page is `Cp437`, and profiles are modules. Then,
after an architecture pass: raw layers are separate `hadris-<fmt>-raw` crates;
`FatFs` and `ExFatFs` collapse to one type parameter and require `alloc`;
the shared tier ships `sync` and `Send` `async`; firmware gets a separate
embedded API with exFAT read-only in 3.0. Async-only with a `block_on` sync
wrapper was measured and rejected (4.15). The API prototype then replaced
`host::Error` with the crate-wide `PathError` (pass 2).

**Q10. Where `Error<E>` lives.** Resolved 2026-09-24 by the user.
`Error<E>`, `ErrorKind` and `Location`, with the detail-code plumbing
(`DetailCode`), `Errno` and `FsResult`, live in `hadris-io`, the lowest
crate. `hadris-fs` and the umbrella `hadris` re-export them. Decision B makes
the block methods return `Error<E>` and `hadris-fs` depends on
`hadris-storage`, so the type had to sit at or below `hadris-storage`;
merging `hadris-storage` into `hadris-fs` was the rejected alternative.

**Q11. Read refusals by adapters.** Resolved 2026-09-24 by the user.
`BlockDevice::read_blocks` returns `Error<Self::Error>` like `write_blocks`
and `flush`. A read an adapter refuses itself (past the end of a
`Partition` or a memory device) is an `ErrorKind` with a `Location`, so
`StorageError` and `OutOfRange` are gone. The rejected options were a small
adapter error type again, and treating such reads as caller bugs.

**Q12. NTFS in `open`.** Resolved 2026-09-24 by the user. `AnyFs` gets no
NTFS variant in 3.0: `open` fails with `NotRecognized` and the message
`"ntfs"` on an NTFS volume, while `detect` still lists `ImageFormat::Ntfs`.
`AnyFs` is non-exhaustive, so the variant is added in the 3.x minor that
stabilises NTFS. A variant present in every build with a stable payload was
the rejected alternative.

**Q13. Open points from R5.** Resolved 2026-09-25 by the user:

- `hadris_fat::raw` no longer re-exports the whole `hadris-fat-raw` crate, which tied `hadris-fat`'s API to the raw crate's version. `hadris-fat` keeps the `check` functions and the raw types its own signatures use (`FatKind`, `Detail`, `exfat::Detail`, and `Geometry` once `format` returns it); users of the rest depend on `hadris-fat-raw` directly (R12).
- `MountOptions::with_utc_offset` stays fallible. The host's local UTC offset is the default of `host::mount_options()`, not of `MountOptions::new()`, as D10 says.
- A FAT label is decoded through the mount's code page, like short names, so a label with bytes above `0x7F` no longer reads as `Some("")`.

**Q15. Open points from R7.** Resolved 2026-09-25 by the user:

- The `hadris` binary stays in its own `hadris-cli` package (`cargo install hadris-cli`). The umbrella gets no `cli` feature, so the library never pulls in clap and the CLI versions apart. Sections 3 and 5.9 say so.
- Choosing the ISO 9660 namespace through `open` and `AnyFs` is deferred to 3.x. `MountOptions` stays shared across formats, so `open` mounts the most capable tree and `IsoFs::mount_namespace` chooses another; an ISO-specific way to choose the tree can be added later without a break.
- A damaged UDF File Identifier Descriptor keeps failing the listing it is in, since the lengths that locate the next descriptor cannot be trusted. Skipping it would need a way for `readdir` to report a skipped entry.
- `hadris cpio extract` takes `-p/--path` like the other formats: the root is merged into the output, and any other entry or subtree lands at `<output>/<name>`.

**Q14. Open points from R6.** Resolved 2026-09-25 by the user:

- Warning paths and `Report::extents` keys use one form, the one `Tree::insert` takes: no leading, trailing or repeated `/`.
- Serials and GUIDs hash the tree (paths, sizes and times, through `Tree::fingerprint`) together with the seed, or the time without one, as 4.10 says. Output stays reproducible for fixed inputs. `format` alone has no tree, so its serial derives from the seed or the time.
- FAT and exFAT have no `plan` in 3.0. It can be added in 3.x without a break.
- `with_label` keeps taking a `VolumeLabel`, which implements `TryFrom<&str>`, so an invalid label fails where it is built.
- `read_tree` of a single file names it as its directory lists it, at the cost of one directory scan. The FAT CLI's rename step is gone.
