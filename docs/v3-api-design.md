# Hadris principles and features

This is the canonical statement of the rules the Hadris API follows from 3.0
on. The rustdoc describes each item; this document says why the items are
shaped the way they are and what a change must preserve. The
[stability page](../website/docs/stability.md) is the user-facing summary of
the semver promise, and the [action catalog](v3/actions.md) lists the
operations each format supports.

Rules carry stable IDs (R1 to R12) because code, scripts and CI cite them.
The full design record that preceded this document, including the rejected
alternatives and the prototype passes, is in the git history
(`git show ee4e60f2:docs/v3-api-design.md`).

Contents:

1. [Purpose](#purpose)
2. [Principles](#principles)
3. [Layers and crates](#layers-and-crates)
4. [Tiers and modes](#tiers-and-modes)
5. [Errors](#errors)
6. [Options and policies](#options-and-policies)
7. [Features](#features)
8. [Semver and previews](#semver-and-previews)
9. [Planned additions](#planned-additions)

## Purpose

Hadris reads, writes and creates disk and archive formats (FAT12/16/32,
exFAT, ISO 9660, UDF, CPIO, MBR and GPT, with NTFS and APFS as previews) in
Rust, from firmware without an allocator up to hosted tools. It serves three
kinds of users, none of them optional:

- Kernels, VFS layers and FUSE: `Send` drivers, stable node IDs, positional
  I/O, resumable directory reads, and no cache or lock they did not ask for.
- Bootloaders and firmware: reads and writes without an allocator, small
  code and stack, and async on single-threaded executors.
- Image-mastering tools: tree builders with bounded memory, boot options,
  reproducible output, and partitioned disk images in one flow.

The 3.0 shapes are the extension points. New features land in 3.x minors
because the types, traits and error kinds they need already exist.

## Principles

**Every tier does every job.** A user picks a tier for its cost, never for
reach. The bare driver can open, list, walk, copy trees and call format
extras; the shared `Volume` reaches the same extras through `vol.lock()`.

**One closed core trait.** `FileSystem` is the set of operations every
mounted filesystem has: look up, list, read and write at an offset, change
metadata, create and remove. Generic code, the conformance suite and FUSE
target it. It never gains a method only one format can answer. Format
specifics are inherent methods with the same name on every driver (`info`,
`extents`, `records`, `read_raw`) or free functions in the format module
(`format`, `check`, `plan`, `write`); shared helpers over the trait are
free functions or extension traits, not new trait methods.

**no_std and sync/async parity.** Every library crate builds without `std`.
Reading needs no allocator where the format allows it. The sync and async
APIs are generated from one source and expose the same items, apart from the
documented mode differences in [Tiers and modes](#tiers-and-modes). Shared
library code uses `hadris-io` and `hadris-storage`, never `std::io`.

**Policies are values.** Anything that chooses behaviour (read-only, caching,
clocks, code pages, path resolution) is a runtime value the caller passes.
Features add items; they never decide what an item does
([Options and policies](#options-and-policies)).

**Reproducible output.** Writers take a fixed time and an optional seed, and
derive serials and GUIDs from them plus the tree. There is no hidden clock or
RNG; `Guid::random` exists only with `std`.

### Stability rules

- **R1. User-facing enums are `#[non_exhaustive]`.** Errors, kinds, options,
  entry kinds and partition types. Closed on-disk code sets are the
  exception and live in raw layers (R4).
- **R2. User-facing structs have private fields.** Options implement
  `Default`, take consuming `with_*` setters, and offer presets as associated
  functions. Results and entries expose getters. Complete plain values
  (`Guid`, `DateTime`, `NodeId`) keep private fields with `const fn`
  constructors.
- **R3. No feature switches behaviour.** A feature may add types, functions,
  modules and impls. It never adds or removes a variant, field or trait
  bound of an existing item, and never changes what an existing item does.
  `ErrorKind::NoSpace` exists in every build. The one exception is
  `unstable-apfs`, which adds `AnyFs::Apfs` and makes `detect` and `open`
  handle APFS for the whole build.
- **R4. On-disk layouts live in raw crates.** `#[repr(C)]` structs, bitflags
  mirroring disk bytes and specification constants are public and exhaustive
  there. Format crates re-export only the raw items their own signatures
  use.
- **R5. No glob re-exports at crate roots.** Roots re-export a curated list
  and never change meaning with features. I/O types are named through their
  mode module (`hadris_iso::sync::IsoFs`).
- **R6. No panics on disk data or user input.** `expect` and `unwrap` only
  cover invariants the same function established. No feature turns an error
  into a panic. There are no `try_*` twins.
- **R7. Fallible iterators fuse.** An iterator yielding `Result` yields at
  most one `Err`, then `None`, and is `FusedIterator`. The same applies to
  async `next_entry` methods.
- **R8. Sizes and offsets are `u64`** on every target. On-disk narrowing is
  checked and fails with `LimitExceeded`.
- **R9. No bool parameters.** Use an enum, flags or an options struct.
  Functions with more than four parameters take a struct.
- **R10. User-implemented traits can grow.** `FileSystem`, `BlockDevice`,
  `Read`, `Write`, `Seek`, `Clock` and `CodePage` are not sealed. A method
  added in 3.x has a default body that returns `Unsupported` (`ReadOnly` for
  writes) or composes existing methods, with a matching `Capabilities` flag
  that defaults to off. `AnyFs` forwards every method, enforced by a test.
  Traits only Hadris implements are sealed.
- **R11. CI enforces the rules.** `cargo semver-checks`
  (`scripts/check-semver.sh`), the non-exhaustive and raw re-export lint
  (`scripts/check-non-exhaustive.py`), the feature matrix, and sync/async
  contract tests.
- **R12. Crates version independently.** See
  [Semver and previews](#semver-and-previews).

## Layers and crates

```
hadris-io          byte streams: Read, Write, Seek in sync, async_ and local;
                   Error<E>, ErrorKind, Location, DetailCode
hadris-storage     BlockDevice and its adapters: Partition, Cache, ReadAhead,
                   ByteView, MemDevice, StreamDevice, host::FileDevice
hadris-fs          shared vocabulary, FileSystem, Volume, Walk, Tree, Content,
                   Report, PathError, MountOptions, copy_tree, read_tree, host
hadris-<fmt>-raw   I/O-free codecs and on-disk layouts; FAT also has raw::io
                   device primitives
hadris-<fmt>       drivers (FatFs, ExFatFs, IsoFs, UdfFs, NtfsFs, ApfsFs),
                   format extras, format, check, plan, write
hadris             umbrella: flat re-exports, detect, open, AnyFs, host::open
hadris-cli         the `hadris` binary, versioned apart from the library
```

Each layer depends only on the layers above it in this list. A device
implementation depends on `hadris-storage` alone; a format crate exposes only
`hadris-io`, `hadris-storage` and `hadris-fs` types, plus `hadris-iso`
options in the UDF bridge writer.

The `*-raw` crates are a supported foundation, not internals. They hold the
specification knowledge (layouts, checksums, name codecs, entry sets, Rock
Ridge and descriptor parsing) so that someone building their own tool, a
fsck, a forensic reader or a different driver model, can use correct
encodings without reading the specifications. They need no allocator and do
no I/O, except the FAT device primitives.

`hadris-common` and `hadris-macros` are internal support crates.

Image writers do not implement `FileSystem`. Every writer (ISO, UDF, the
ISO/UDF bridge, CPIO, and FAT and exFAT through `format` plus `copy_tree`)
takes one shared `Tree` and returns one `Report`, through `fmt::plan` (no
I/O) and `fmt::{sync, async_}::write`, which plans first so option errors
never touch the device. Streaming archives (CPIO) get a forward-only entry
reader, not random access.

## Tiers and modes

| Tier | Entry point | Needs | Gives |
|---|---|---|---|
| Bare `FileSystem` | `FatFs::mount(dev, options)` | `alloc` for `FatFs`, `ExFatFs`; nothing for `IsoFs`, `UdfFs`, `NtfsFs` | Node API on `&mut self`, no lock; format extras, `Walk`, `copy_tree` |
| `Volume` | `Volume::new(fs)` | sync: `std`; async: `alloc` | Path methods named after `std::fs`, `File` and `ReadDir` handles, cheap clones across threads and tasks |
| Embedded | `Fat::mount_with(dev, &mut token, options)` | nothing | FAT and exFAT handle API on the raw layer: fixed slots, 512-byte blocks, ASCII folding by default |
| Host | `host::open(path)`, `host::mount_options()` | `std` | Host files as devices, host trees, the host clock and UTC offset |

Drivers own their device, take `&mut self` and hold no lock. `Volume<F>` adds
exactly one lock, held for one call. Each tier adds only its own cost.

Each mode module name means one thing in every crate:

- `sync`: blocking API. Default on.
- `async_`: futures with `Send` bounds where the device and its operation
  state are `Send`. Block drivers use one type for local and `Send` devices.
- `local`: only in `hadris-io` and `hadris-fs`, the same contracts without
  `Send` bounds, for single-threaded executors.

Intended mode differences: `ReadDir` is an `Iterator` in sync and has
`next_entry` in async; sync handles implement `std::io` with `std`; the host
tree functions are sync-only. Sync code is generated from the async source,
not wrapped in `block_on`. The async device contract is described in the
[async device guide](../website/docs/guides/async-devices.md).

## Errors

- Every operation returns `Error<E>` (`FsResult<T, E>`), generic over the
  device error, defined in `hadris-io` and re-exported by `hadris-fs` and
  `hadris`. Device errors must be `core::error::Error + Send + Sync +
  'static` and survive unchanged without `alloc`.
- `ErrorKind` is a closed vocabulary with one errno each; `NotRecognized`
  ("not this format") is distinct from `Corrupt`.
- Context is static and allocation-free: a message, an optional `Location`
  and a `DetailCode` within a per-crate domain, read back with
  `Detail::of(&err)`. Detail numbers are pinned by tests and shared with
  `check` findings.
- `Error<E>` is `Clone` when `E` is, never `Copy`.
- Drivers that take the device by value fail to mount with `MountError`,
  which gives the device back.
- Code that carries paths or mixes devices returns `PathError` (`alloc`).
  With `std`, both convert to `std::io::Error` without losing context.
- An error shows its source's text in `Display` or returns it from
  `source()`, never both.

## Options and policies

`MountOptions` is the one shared mount options bag for every format:
`read_only`, `with_clock`, `with_utc_offset`, `with_code_page`,
`with_node_limit`, `backup_boot`. Formats ignore what does not apply, and
the rustdoc lists which driver honours which option. `MountOptions::new()` is
the same on every target (read-write where possible, UTC, CP437, a fixed
clock); `host::mount_options()` is the host default. Format-specific mount
choices are format-specific entry points or options, never a feature.

These stay runtime values in every crate: durability and sync behaviour,
caches (`Cache`, `CacheOptions`), read-ahead (`ReadAhead`), read-only
mounts, code page, case folding, long file names, FAT kind, node limits and
path resolution (`Resolve`). Features unify across the dependency graph, so a
policy chosen by a feature would change behaviour for unrelated crates in the
same build. A value affects only the code that passes it.

Writer options (`FatOptions`, `IsoOptions`, `UdfOptions`, `CpioOptions`)
follow R2 and take `with_time` and `with_seed` for reproducible output.

## Features

### Feature rules

- A feature only adds whole items. It never changes what an existing item
  does (R3).
- Capability features are default-on. Disabling default features is how a
  user opts out of code, not how they opt in to behaviour.
- `alloc` means "has a heap" and `std` implies `alloc`. Neither selects an
  I/O mode.
- `write` means "can create images" in every crate that has it: formatters,
  tree writers, `plan`, sessions and their option types. Reading is always
  compiled; there is no `read` feature.
- `sync` and `async` select mode modules and may be enabled together.
- Anything that dispatches across formats (the umbrella's `detect`, `open`
  and `format`) enables the formats and capabilities it needs itself, so what
  it recognizes never depends on what else is enabled.
- `tracing` adds spans and does not imply `std`.
- `unstable-*` features add previews outside semver.

### Per-crate features

| Crate | Default | Other features |
|---|---|---|
| `hadris-io` | `std`, `sync` | `alloc`; `async`; `embedded-io` (`FromEmbedded` and embedded-io conversions) |
| `hadris-storage` | `std`, `sync` | `alloc` (`Vec<u8>` devices, `Cache`, `ReadAhead`); `async` |
| `hadris-fs` | `std`, `sync` | `alloc` (`Tree`, `PathError`, async `Volume`, `copy_tree`); `async`; `unstable-contract` (driver test kit) |
| `hadris-fat` | `std`, `sync`, `write` | `alloc` (`FatFs`, `ExFatFs`); `write` (`format`, and with `alloc` the tree writer `write`; needs no allocator itself); `async`; `defmt`; `tracing` |
| `hadris-fat-raw` | none | `sync`, `async` (device primitives); `defmt` |
| `hadris-part` | `std`, `sync` | `alloc` (`Disk`, the table types, `DiskLayout`, `read`, `write`, `create`); `async`; `tracing` |
| `hadris-iso` | `std`, `sync`, `write` | `alloc` (heap only: bounded reader caches, `CacheOptions`); `write` (`IsoOptions` and related option types, `plan`, `write`, `Session`; implies `alloc` and the `hadris-part` dependency); `async`; `tracing` |
| `hadris-udf` | `std`, `sync`, `write` | `alloc` (heap only); `write` (`UdfOptions`, `plan`, `write`; implies `alloc`); `bridge` (off by default; `plan_bridge`, `write_bridge`; implies `write` and the `hadris-iso` dependency); `async`; `tracing` |
| `hadris-cpio` | `std`, `sync`, `write` | `alloc` (heap: `read_tree` into a `Tree`); `write` (the streaming `Writer`, `plan`, `write`); `async`; `tracing` |
| `hadris-iso-raw`, `hadris-udf-raw`, `hadris-cpio-raw` | none | none |
| `hadris-ntfs` (preview) | `std`, `sync` | `alloc`; `async`; `tracing` |
| `hadris-apfs` (preview) | `std`, `sync` | `alloc` (mounting); `encryption` (software unlocking); `async`; `tracing` |
| `hadris` | `std`, `sync`, `write`, `fat`, `iso`, `cpio`, `detect` | `alloc`; `async`; `tracing`; `fat`, `part`, `iso`, `udf`, `cpio` (one per format); `write` (forwards `write` to the FAT, ISO, UDF and CPIO crates and `bridge` to UDF); `detect` (`detect`, `open`, `AnyFs`, and `format` with `write`; enables `fat`, `part`, `iso`, `udf`, `cpio`); `unstable-ntfs`; `unstable-apfs`; `apfs-encryption` |

`hadris-common` (default `bytemuck`) and `hadris-macros` are internal. The
platform, mode and `tracing` features forward to the crates below.

## Semver and previews

- Every stable crate shipped 3.0.0 together. After that each crate has its
  own version and release tag (`<crate>-v<version>`) and bumps its major only
  for its own breaking changes.
- A crate exposes another Hadris crate's types only where that crate is a
  deliberate public dependency: `hadris-io`, `hadris-storage` and `hadris-fs`
  for every format crate, and `hadris-iso` for the UDF bridge. A major
  version of a public dependency is a major version of the crate exposing
  it.
- The umbrella bumps its major when a stable crate it re-exports does.
- Raw crates stay at 0.x and are never re-exported wholesale. Items a format
  crate re-exports are frozen for that crate's major, and the format crate
  stays on one 0.x line of the raw crate for its whole major. The rest of a
  raw crate may break in any 0.x minor. Each raw crate has one path per
  item.
- `hadris-common` keeps a 3.x version but is exempt from semver; stable
  crates never expose its types.
- Preview crates (`hadris-ntfs`, `hadris-apfs`) carry 3.x versions and are
  exempt from semver: a break bumps the minor, patch releases stay
  compatible, and users pin them with `~3.y`. A stable crate depends on a
  preview only behind an `unstable-*` feature. A preview that becomes stable
  follows semver from its next major.
- Everything an `unstable-*` feature adds is outside semver and may change in
  any release. Users who enable one pin `hadris` with `=` or `~`.
- Removing a feature is a breaking change, so new features are added only
  when they gate whole items users can opt out of.

## Planned additions

Each of these fits an existing 3.0 shape (a defaulted trait method, a new
entry point, a new variant of a non-exhaustive enum, or a new option), so
none needs a break.

| Area | Addition |
|---|---|
| `FileSystem` | `pin` for ids returned by `readdir`; `symlink`, `link`, `mknod`; `RenameMode::Exchange`; extended attributes and named streams; a `lookup` that returns the stored name; orphan tracking so unlinking an open node succeeds |
| Vocabulary | `FsStats` available blocks and free file slots; per-field time resolution in `Capabilities`; `DateTime` and `SystemTime` conversions; `Errno::darwin()` |
| Shared tier | `Volume::with_lock` for `no_std` sync users; `Content::source` for user-supplied lazy content; `copy_tree` merge and overwrite policies; `write_planned` |
| FAT and exFAT | `repair` and analysis after `check`; embedded exFAT write through a new entry point; an embedded FAT-or-exFAT type |
| ISO 9660 | `check`; Joliet beyond the BMP and long Joliet names; zisofs read and write; RRIP SF and RR; Apple Partition Map in hybrid images; `iso::SystemArea`; a two-pass `write_stream` to non-seekable sinks; `Session::plan`; raw record, Rock Ridge and descriptor decoders in `hadris-iso-raw` |
| UDF | `UdfFs::mount_writable` (`mount` stays read-only); VAT, sparing tables, metadata partitions, stream directories; `check`; UDF 2.50 and 2.60 |
| CPIO | write without `alloc`; `Entry::link_id()` |
| NTFS | Promotion once its read side passes a conformance slice, then write support and an `AnyFs::Ntfs` variant |
