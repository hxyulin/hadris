# Changelog

All notable changes to this workspace are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each published package owns its version and may be released independently.

## [Unreleased]

## [2.0.0-rc.3] - 2026-07-22

### Added

- **hadris-iso:** Added an explicit ISO interchange `BaseIsoLevel::Level3` and
  corrected the CLI `--level 3` mapping.
- **hadris-iso:** Allocation-free readers now discover and explicitly select
  the ISO 9660:1999 enhanced namespace, including its root directory.

### Fixed

- **hadris-udf:** Directory FID extents are planned from exact encoded record
  lengths instead of an unsafe per-entry estimate.
- **hadris-udf:** OSTA CS0 filenames now select compression ID 8 or 16 from
  their Unicode contents, enforce the 255-byte encoded limit, and decode
  8-bit values as one-byte Unicode code points.
- **hadris-iso:** `has_evd()` now reports an ISO 9660:1999 Enhanced Volume
  Descriptor rather than implying that UDF is present.

### Changed

- **hadris-io / hadris-common:** `std` no longer activates `sync`; hosted
  support and I/O mode selection are independent.
- Broken Markdown links now fail the documentation build.

## [2.0.0-rc.2] - 2026-07-19

### Added

- **hadris-iso:** Added a zero-allocation `IsoReader` for sync and async
  `no_std` builds, including ISO 9660/Joliet namespace selection, nested path
  lookup, caller-buffered reads, and multi-extent file streaming.
- **hadris-fat:** Added `NtCaseFlags` and `FileEntry::nt_case` /
  `ShortFileName::with_nt_case` so lowercase 8.3 short names round-trip in their
  original case.
- **hadris-iso:** `EmulationType` now names the El Torito boot media types
  (1.2/1.44/2.88 MB floppy and hard-disk emulation) with an `is_emulated`
  helper, so bootable images can request emulated media. Emulated boot entries
  default their load size to one virtual sector.
- **hadris-iso:** Rock Ridge `TF` timestamp entries now include the creation
  time when the input entry carries one (new `RripBuilder::add_tf`), alongside
  the existing modify and access times.
- **hadris-fat:** `Fat12` and `Fat16` now expose `allocate_chain` and
  `extend_chain`, matching `Fat32`, for callers doing manual multi-cluster
  allocation.
- **hadris-iso:** The allocation-free `IsoReader` can now resolve Rock Ridge
  alternate names: `IsoDirEntry::rrip_name_into` decodes the `NM` field into a
  caller buffer and `rrip_name_matches` compares it, both without allocating.
  (Inline system-use areas only; `CE`-continued names are not followed.)

### Removed

- **hadris-iso:** Removed the unused, always-empty `read::SupportedFeatures`
  bitflags stub.

### Fixed

- **hadris-iso:** `IsoImage::open` now rejects images that declare a logical
  block size other than 2048 with a clear `Unsupported` error, instead of
  silently misreading their extents. (The allocation-free `IsoReader` honors the
  declared block size; full non-2048 support in `IsoImage` remains future work.)
- **hadris-iso:** Joliet file identifiers are now encoded as conformant UCS-2:
  characters outside the Basic Multilingual Plane are substituted with `_`
  instead of leaking UTF-16 surrogate pairs into the field, and names are capped
  at the Joliet 64-character limit (previously up to 103). `encode_joliet_name`
  is likewise BMP-safe.
- **hadris-iso:** Directory records are now written in ascending File Identifier
  order (ECMA-119 9.3) instead of input/tree order, so images validate against
  strict readers.
- **hadris-iso:** Writing a file larger than 4 GiB now fails with a clear error
  instead of silently truncating its length to 32 bits. Multi-extent records
  (which would lift the limit) are not yet emitted.
- **hadris-fat:** Lowercase 8.3 names (e.g. `readme.txt`) are now stored as a
  single short entry with the Windows NT `DIR_NTRes` case flags and read back in
  their original case, instead of being uppercased on read or spending a
  long-file-name entry. `rename` now records case flags for the new name rather
  than carrying over the source entry's.

### Documentation

- Expanded the `@hadris-spec` compliance annotations and `docs/spec-coverage.md`
  to cover more ISO volume descriptors, path tables, and El Torito section
  entries, the FAT FSInfo sector, and a new `hadris-part` (MBR/GPT) section.
- Completed the in-repo ISO 9660 specification notes (volume descriptors, path
  table, directory record, and an extensions index) and documented the ISO
  writer's known limitations.
- Added `docs/hadris-2-perf-notes.md` recording the caching/performance findings
  deferred out of 2.0.
- Added a Docusaurus documentation site with getting-started, crate-selection,
  migration, release-candidate, and task-oriented FAT, partition, ISO, CPIO,
  and `no_std` guides.
- Added GitHub Pages build and deployment automation for the documentation
  site.
- Added runnable workspace examples for listing FAT images and partition
  tables, detecting optical formats, and creating CPIO archives.

### Changed

- Removed the unused top-level `tests` and `resources` placeholders; tests and
  fixtures remain colocated with their owning crates.

## [2.0.0-rc.1] - 2026-07-16

### Added

- **hadris-cd-cli:** New `hadris-cd` utility for creating, inspecting, and
  verifying ISO 9660/UDF bridge images, including Joliet, Rock Ridge, El Torito,
  and hybrid MBR/GPT options.
- **hadris-cd / hadris-iso:** Direct non-empty bridge qualification through
  both concrete readers and an ISO allocation-floor API for collision-free
  composition with other on-disc metadata.
- **hadris-fat-cli:** `cat`, selective and recursive extraction, and recursive
  FAT image creation with automatic or explicit sizing.
- **hadris-part:** `read` is now a default feature; I/O extension traits
  (`MasterBootRecordReadExt`, `GptDiskReadExt`, `DiskPartitionSchemeReadExt`,
  and write counterparts) are re-exported at the crate root.
- **hadris-part:** Explicit `crc` / `rand` feature flags; docs.rs builds with
  all features.
- **hadris-part:** I/O roundtrip integration tests for MBR read/write and
  scheme detection.
- **hadris-macros:** Dual sync/async integration guide in the crate README.
- **CI:** `check-features` tiers for `hadris-io` async and `hadris-part`
  async-read / crc.
- **hadris-udf:** Public `UdfFs::read_file`; directory listings populate
  `UdfDirEntry::size` from each file ICB.
- **hadris-udf-cli:** `cat` and `extract` subcommands.
- **Async integration coverage:** Direct leaf-level runtime tests for FAT
  traversal and multi-cluster reads, GPT detection/opening, ISO descriptor and
  file reads, and UDF nested traversal/file reads.

### Changed

- **CLI tools:** Canonical installed binaries now form the `hadris-fat`,
  `hadris-iso`, `hadris-udf`, and `hadris-cpio` family. Existing executable
  names remain compatibility aliases, and CPIO standardizes on `ls` with
  `list` retained as an alias.
- **Workspace cleanup:** Removed the unpublished `hadris-cli` FAT debug stub;
  the supported V2 command-line surface is the specialized `hadris-*` family.
- **Project positioning and package metadata:** Reframed Hadris as a layered
  Rust storage stack, documented its architecture and target environments, and
  refreshed the `hadris`, FAT, ISO, UDF, block, and storage crate descriptions
  and search keywords.
- **Public API documentation:** Completed and now enforce missing-doc coverage
  for the fixed-capacity, I/O, common, storage, block facade, optical facade,
  FAT, partition, ISO, UDF, CPIO, and hybrid optical writer crates.
- **Release process:** Removed shared workspace versioning and the obsolete
  `cargo-release` configuration. Every current package now declares version
  `2.0.0-rc.1` in its own manifest.

- **hadris-part:** `PartitionError::Io` now wraps `hadris_io::Error` (with
  `std::error::Error::source` under `std`) instead of discarding context.
- **hadris-cd:** Missing/unreadable source paths during ISO tree conversion
  now return `CdError` instead of silently writing empty files.
- **hadris-iso / hadris-fat / hadris-udf:** Documented known limitations in
  crate-level rustdoc.
- **CI / process:** MSRV pinned to Rust 1.88.0 (required for `let`-chains in
  `hadris-macros`; `rust-toolchain.toml`, workspace `rust-version`, CI
  toolchain); CLI `--help` smoke job; workspace `cargo doc` job; Dependabot
  for Cargo and GitHub Actions; CONTRIBUTING.md. Fuzz harnesses remain
  local-only (not PR CI).

## [1.2.1] - 2026-07-09

### Added

- **Fuzzing:** Coverage-guided fuzz harnesses for `cpio_read`, `fat_read`,
  `iso_read`, and `udf_read`, with a committed seed corpus (including CPIO
  allocation-DoS regressions).
- **SECURITY.md:** Project security policy.

### Fixed

- **hadris-fat / hadris-iso / hadris-udf / hadris-cpio:** Bound untrusted length
  fields before allocating; reject inputs that previously panicked readers.
- **hadris-udf:** Validate File Entry allocation window before slicing.
- **hadris-fat:** Skip volume label entries when listing directories.

### Documentation

- Workspace and crate READMEs updated for API accuracy and version `1.2.1`
  (ongoing professionalization; see
  `docs/superpowers/specs/2026-07-09-professionalization-review.md`).

## [1.2.0] - 2026-06-13

### Added

- **hadris-fat:** `FatFs::builder` / `FatFsBuilder` for configuring a volume
  before mount, with pluggable providers:
  - `with_time_provider` — custom clock (`TimeProvider`) for directory-entry
    timestamps.
  - `with_oem_converter` — custom OEM codepage (`OemCpConverter`) for short
    (8.3) filename encoding.
  - `with_fat_cache` — optional LRU FAT-sector cache (requires the `cache`
    feature; sync API only).
- **hadris-fat:** Long filename (VFAT/LFN) **write** support — create and
  delete entries with names up to the 255 UTF-16 code-unit spec cap, including
  supplementary-plane (surrogate-pair) characters.
- **hadris-fat:** Volume timestamp, label, and status-flag (`dirty`,
  `io_errors`) read APIs, sourced from the FAT-resident status word (`FAT[1]`).
- **hadris-fat:** `cache` feature — LRU FAT-sector cache reducing redundant
  seek+read I/O for FAT entry access; dirty entries flush to all FAT copies on
  eviction. `cache` implies `sync`.
- **hadris-fat:** `defmt` support for error types in embedded/no-std contexts.
- **CI/safety:** Miri jobs covering historically-unsafe code paths — LFN
  UTF-16 / surrogate-pair handling and LFN write encoding / union access.

### Changed

- **hadris-fat:** Errors now carry I/O context (`IoContext`) describing the
  failed operation instead of a bare I/O error.
- **hadris-part:** MBR LBA and GPT on-disk fields use `endian-num` typed
  endian fields instead of manual byte handling.
- **Workspace:** endianness types moved to `zerocopy`; `alloc` error types
  supported in no-std builds.

### Fixed

- **Soundness:** Eliminated UTF-8 undefined behavior when converting disk
  bytes to `&str` in LFN, `IsoStr`, and `IsoString`; removed unsoundness in
  `FixedFilename::as_str`.
- **hadris-fat:** Guard against infinite loops on corrupt cluster chains
  (cluster-loop / out-of-bounds / bad-cluster markers now return errors).
- **hadris-fat:** FAT32 `FSInfo` was not flushed on some write paths.
- **hadris-udf:** Fixed errors when parsing Windows 11 ISO images.
- **hadris-iso:** Auto-convert lowercase in PVD string fields instead of
  panicking.
- **Docs:** Fixed broken rustdoc intra-doc links (`OemCpConverter`, `FAT[1]`);
  docs.rs builds `hadris-fat` with the full stable sync feature set while the
  unstable exFAT preview remains opt-in.
- **hadris-fat:** The `tool` feature now implies `sync` and is emitted only
  in the sync slice — the analysis/verify utilities iterate directories
  synchronously, so `--features async,tool` previously failed to compile.
- **hadris-fat:** All sync-only cache code (`with_cached_fat`,
  `with_fat_cache_locked`, `fat_cache`, internal `*_via_cache` helpers) is
  now confined to the sync slice, so `--features async,cache` and
  `--all-features` compile (the cache is simply bypassed under async).
- **hadris-fat:** Long-filename entry runs may now cross directory cluster
  boundaries, including maximum-length names and directory extension.

### Known limitations

- **async + cache:** The FAT-sector cache is sync-only. Driving a volume
  through the async API silently bypasses the cache (async-aware caching is
  deferred — see the `cache` feature note in `hadris-fat/Cargo.toml`).
- **exFAT:** Available only as the leaf-crate `unstable-exfat` preview. It is
  outside the V2 API stability promise and unified block opener; fragmented
  system metadata, directory growth/general cross-cluster entry placement,
  async operation, TexFAT, and repair workflows remain unsupported.

## [1.1.0] - 2026-03-12

Baseline for this changelog. See the git history for changes at and before this
tag.

[Unreleased]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.3...HEAD
[2.0.0-rc.3]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.2...v2.0.0-rc.3
[2.0.0-rc.2]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.1...v2.0.0-rc.2
[2.0.0-rc.1]: https://github.com/hxyulin/hadris/compare/v1.2.1...v2.0.0-rc.1
[1.2.1]: https://github.com/hxyulin/hadris/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/hxyulin/hadris/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/hxyulin/hadris/releases/tag/v1.1.0
