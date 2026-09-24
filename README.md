# Hadris

**The Rust storage stack.**

Hadris is a collection of pure Rust storage and filesystem libraries for block
devices, GPT and MBR partition tables, FAT12/16/32, exFAT, ISO 9660, UDF,
CPIO, and disk images, plus a read-only NTFS reader in preview. It supports desktop
applications as well as `no_std` bootloaders, operating-system kernels,
firmware, and embedded devices.

Use a focused format crate such as `hadris-fat` or `hadris-iso`, a category
facade such as `hadris-block`, or the `hadris` umbrella crate as an application
grows. Shared I/O, storage, filesystem, feature, and API conventions keep
those layers coherent without hiding format-specific capabilities.

## Stability and Versioning

Hadris follows [Semantic Versioning](https://semver.org/). `2.4.0` is the
current stable release, and the 2.x series continues on `main`. The 3.0 API
described here is developed on the `next` branch and is not released yet; its
design and stability rules are in [`docs/v3-api-design.md`](docs/v3-api-design.md).
Within a major series, breaking changes to the public API require a new major
version; minor releases add backward-compatible functionality, and patch
releases are limited to correctness fixes, interoperability qualification, and
documentation.

In 3.0, exFAT is stable as `hadris_fat::exfat::ExFatFs`, with no feature flag.
The NTFS reader (`hadris-ntfs`, and the `unstable-ntfs` feature of
`hadris-block` and `hadris`) is a preview whose native API may change in 3.x
minor releases. Every stable crate is covered by the public-API snapshots in
[`api-snapshots/`](api-snapshots/).

Problems that are understood but not fixed yet are listed in
[`KNOWN_ISSUES.md`](KNOWN_ISSUES.md).

## Architecture

![Hadris architecture: applications use the umbrella crate over block, optical, and archive formats backed by shared I/O, paths, and storage](website/static/img/architecture.svg)

Every filesystem driver implements the `FsDriver` trait of `hadris-fs`, so
the path helpers, file handles and generic code work on any of them. Each
format keeps a native API for what the trait does not model: formatting,
checking, FAT attributes, ISO namespaces and boot catalogs, NTFS streams.
Image writers share one input tree instead of a trait. The category facades
detect a format and open it behind the same driver trait.

## Why Hadris?

- **Pure Rust** - Inspect, create, and modify storage formats without C library
  bindings.
- **`std`, `alloc`, and allocation-free configurations** - Select the platform
  support and capabilities appropriate for the target.
- **Bootloader and kernel friendly** - Read disk images and filesystems in
  freestanding environments.
- **Embedded ready** - Work with storage used by firmware, SD cards, and USB
  drives through portable I/O abstractions.
- **Desktop capable** - Build image parsers, filesystem tools, and optical-disc
  image generators with synchronous or asynchronous APIs.
- **One ecosystem** - Move from a leaf filesystem crate to category facades or
  the umbrella crate while retaining the same underlying implementations.

## Who is Hadris for?

- Bootloaders and UEFI or Open Firmware utilities reading FAT and ISO images
- Operating-system kernels and experimental filesystems
- Embedded firmware working with SD cards, USB storage, and raw block devices
- Desktop disk-image, recovery, inspection, and authoring tools
- Build systems producing initramfs, bootable ISO, UDF, or hybrid disc images

## Workspace Crates

Crates are grouped by their storage access model. These directories are
organizational only: published package names such as `hadris-fat` are unchanged.

### Core Libraries

- **[hadris-io](crates/core/hadris-io)** - No-std I/O abstraction layer (`Read`, `Write`, `Seek`)
- **[hadris-fs](crates/core/hadris-fs)** - Shared filesystem vocabulary (names, times, metadata, error kinds), the `FsDriver` and `FileSystem` traits, `Volume`, handles, path helpers, and the input tree of the image writers
- **[hadris-common](crates/core/hadris-common)** - Internal endian-aware integer types for on-disk layouts; not for direct use
- **[hadris-storage](crates/core/hadris-storage)** - Format-neutral block devices, geometry, slices, a block cache, and seekable-stream adapters
- **[hadris-macros](crates/core/hadris-macros)** - Proc macros for dual sync/async code generation

### Block Storage

- **[hadris-block](crates/block/hadris-block)** - Detection of FAT, NTFS, exFAT and partition tables on block devices, and `OpenVolume`, which opens FAT12/16/32, exFAT and NTFS behind one `hadris-fs` driver
- **[hadris-part](crates/block/hadris-part)** - Partition table support on block devices
  - MBR with extended and logical partitions
  - GPT with backup-copy recovery and UTF-16 names
  - Hybrid MBR (Combined MBR+GPT for dual BIOS/UEFI boot)
  - `DiskLayout` builder for whole-disk images
- **[hadris-fat-raw](crates/block/hadris-fat-raw)** - FAT12/16/32 and exFAT on-disk layouts and I/O-free codecs, for tools and custom drivers
- **[hadris-fat](crates/block/hadris-fat)** - FAT filesystem implementation
  - FAT12, FAT16, FAT32 support
  - `FatFs`, a node-based `hadris-fs` driver in sync, async and `Send` async modes
  - Long filename support (VFAT/LFN)
  - Formatting and a read-only checker, all without an allocator
  - `ExFatFs`, an allocation-free exFAT driver with its own formatter and checker, in the same three modes; opened by `hadris-block`
- **[hadris-ntfs](crates/block/hadris-ntfs)** - Read-only NTFS reader
  (preview) on block devices, allocation-free in sync, async and `Send`
  async modes, with attribute lists, named streams and `$UpCase` case
  folding; opened by `hadris-block`

### Optical Media

- **[hadris-optical](crates/optical/hadris-optical)** - Detection of ISO 9660, UDF and bridge images on block devices, and `OpenOpticalImage`, which opens one of them behind one `hadris-fs` driver
- **[hadris-iso](crates/optical/hadris-iso)** - ISO 9660 filesystem implementation
  - Allocation-free reader for the primary, Rock Ridge, Joliet and enhanced trees, in sync, async and `Send` async modes
  - Writer and multi-session updates driven by the shared input tree
  - ISO 9660 Level 1-3 and ISO 9660:1999 (long filenames)
  - Joliet extension (UTF-16 Unicode filenames)
  - Rock Ridge (RRIP) and SUSP (POSIX semantics, symlinks)
  - El Torito bootable CD/DVD images and hybrid MBR/GPT boot
- **[hadris-udf](crates/optical/hadris-udf)** - Universal Disk Format (UDF) for DVD/Blu-ray: an allocation-free reader and a writer
- **[hadris-cd](crates/optical/hadris-cd)** - Hybrid ISO+UDF optical disc image creation

### Archives

- **[hadris-cpio](crates/archive/hadris-cpio)** - CPIO archives (initramfs): streaming newc, CRC and odc reader and writer, old binary read

### CLI Tools

| Crate | Binary | Notes |
|-------|--------|-------|
| [hadris-iso-cli](crates/tools/hadris-iso-cli) | `hadris-iso` | ISO create/inspect/extract; legacy alias: `hadris-iso-cli` |
| [hadris-fat-cli](crates/tools/hadris-fat-cli) | `hadris-fat` | FAT12/16/32 and exFAT create/read/extract/check/analyze; legacy alias: `fatutil` |
| [hadris-cpio-cli](crates/tools/hadris-cpio-cli) | `hadris-cpio` | CPIO create/read/extract; legacy alias: `cpioutil` |
| [hadris-udf-cli](crates/tools/hadris-udf-cli) | `hadris-udf` | UDF create/inspect/extract; legacy alias: `hadris-udf-cli` |
| [hadris-cd-cli](crates/tools/hadris-cd-cli) | `hadris-cd` | Create, inspect, and verify hybrid ISO 9660/UDF images |

### Meta-crate

- **[hadris](crates/core/hadris)** - Optional umbrella that re-exports `hadris-io`, `hadris-storage` and `hadris-fs`, and each format crate at a flat path (`hadris::fat`, `hadris::iso`, `hadris::cpio`, ...), with `hadris::block` and `hadris::optical` for detection. One feature per format; the platform (`std`, `alloc`), mode (`sync`, `async`, `async-send`) and `write` features are forwarded to every enabled crate. The defaults are `std`, `sync`, `write`, `fat`, `iso` and `cpio`.

## Key Features

- **No-std compatible** - Use in bootloaders, kernels, firmware, and embedded systems
- **Allocation-free reading** - FAT, exFAT, ISO 9660, UDF, NTFS and CPIO read
  without a heap allocator; FAT and exFAT also write, format and check
- **One driver trait** - Path helpers, handles, `Volume` sharing and host
  import and extraction work on every filesystem
- **Additive features** - Platform, I/O mode and `write` features only add
  items; no feature changes what an existing item does
- **Sync, async and `Send` async** - One source per crate, generated for each
  mode via `hadris-macros`
- **Standards oriented** - ECMA-119, IEEE P1282 / Rock Ridge, El-Torito, Microsoft FAT, ECMA-167 / UDF, CPIO newc

## FAT Interoperability

Hadris-generated images are checked against an independent raw-image oracle
and common FAT implementations. These results cover the same filesystem
operations on every listed FAT variant. The oracle row counts the 18 peer
scenarios and the three geometry-sized limit exercises, all of which pass in
the 2.4.0 hosted suite.

| Consumer | FAT12 | FAT16 | FAT32 |
|----------|-------|-------|-------|
| Hadris specification oracle | Pass (21/21) | Pass (21/21) | Pass (21/21) |
| mtools 4.0.49 reader | Pass (14/16) | Pass (14/16) | Pass (14/16) |
| dosfstools `fsck.fat` 4.2 | Pass (16/16) | Pass (16/16) | Pass (16/16) |
| Rust `fatfs` master (`2aefc2a`) reader | Pass (16/16) | Pass (16/16) | Pass (16/16) |
| macOS `fsck_msdos` | Pass (2/2) | Pass (2/2) | Pass (2/2) |

"Pass" means that the consumer read the expected semantic tree or that the
checker accepted the completed image; the two mtools read failures per width
are names outside the Basic Multilingual Plane, which mtools transliterates.
See the
[FAT compliance profile](docs/compliance/hadris-fat.md#interoperability-results)
for the test method, peer-writer conformance, and known tool defects.

## ISO Interoperability

Hadris-generated ISO images are checked against an independent ECMA-119
raw-image oracle and common ISO readers.

| Consumer | Result |
|----------|--------|
| Hadris specification oracle | Pass (2/2) |
| xorriso/libisofs 1.5.8 | Pass (2/2) |
| Linux kernel ISO driver | Pass (2/2) |
| macOS 26.6.2 built-in ISO reader | Pass (2/2) |
| Windows `Mount-DiskImage` | Available manual target; not yet measured |

See the [ISO compliance profile](docs/compliance/hadris-iso.md#consumers-of-hadris-images)
for the test method, peer-producer conformance, and known tool deviations.

## Quick Start

Choose the narrowest entry point that fits the application:

```toml
[dependencies]
# One filesystem:
hadris-fat = "2.4.0"

# Or the unified storage ecosystem:
hadris = { version = "2.4.0", features = ["block", "optical"] }
```

The umbrella crate re-exports `hadris::io`, `hadris::storage` and
`hadris::fs`, and each format crate at a flat path (`hadris::fat`,
`hadris::iso`, `hadris::cpio`, and `hadris::block` and `hadris::optical` for
detection), so applications can grow into partition detection or additional
disk-image formats without replacing their filesystem implementation. The
umbrella's crate documentation has a quick start, and the compiled programs
in [`examples/`](examples/) show each crate in use.

Each package now owns its version; all current packages target **2.4.0**:

```toml
[dependencies]
hadris-iso = "2.4.0"
hadris-fat = "2.4.0"
hadris-part = "2.4.0"
hadris-fs = "2.4.0"
```

For allocation-free `no_std` ISO reading and FAT reading and writing:

```toml
[dependencies]
# No heap allocator: every ISO tree, Rock Ridge metadata and file reads, and
# FAT reads and writes.
hadris-iso = { version = "2.4.0", default-features = false, features = ["sync"] }
hadris-fat = { version = "2.4.0", default-features = false, features = ["sync"] }
```

Add the `alloc` feature to `hadris-iso` for the writer, sessions and the boot
catalog listing without full `std`.

## Building

```bash
# Build entire workspace
cargo build --workspace

# Run tests
cargo test --workspace

# Build for no-std (example)
cargo build -p hadris-fat --no-default-features --features "sync"
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the test, feature-tier and PR
workflow, and [`docs/v3-api-design.md`](docs/v3-api-design.md) for the 3.0
architecture. The [changelog](CHANGELOG.md) lists the unreleased 3.0 changes
and the [`2.4.0` release](CHANGELOG.md#240---2026-09-08).
The Docusaurus source for the task-oriented documentation site lives in
[`website/`](website/); it includes getting-started, crate-selection, and
FAT, partition, ISO, UDF, CPIO, async, and `no_std` use-case guides.
Runnable application examples live in [`examples/`](examples/) and are compiled
as part of the Cargo workspace.

**MSRV:** Rust 1.88.0 (`rust-toolchain.toml` / workspace `rust-version`).

Fuzz harnesses under [`fuzz/`](fuzz/) are local developer tools and are **not** part of PR CI.

## Development

Install [pre-commit](https://pre-commit.com/) hooks once per clone (runs `cargo fmt` / `cargo clippy` before commits):

```bash
# brew install pre-commit   # or: pipx install pre-commit
pre-commit install
pre-commit install --hook-type pre-push   # also run clippy on push
```

## License

Licensed under the [MIT license](LICENSE-MIT).
