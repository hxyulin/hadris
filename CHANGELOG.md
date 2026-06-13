# Changelog

All notable changes to this workspace are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Crates share a single workspace version.

## [Unreleased]

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
  docs.rs now builds `hadris-fat` with the full sync feature set so cache /
  tool / exfat APIs are documented.
- **hadris-fat:** The `tool` feature now implies `sync` and is emitted only
  in the sync slice — the analysis/verify utilities iterate directories
  synchronously, so `--features async,tool` previously failed to compile.
- **hadris-fat:** All sync-only cache code (`with_cached_fat`,
  `with_fat_cache_locked`, `fat_cache`, internal `*_via_cache` helpers) is
  now confined to the sync slice, so `--features async,cache` and
  `--all-features` compile (the cache is simply bypassed under async).
- **hadris-fat:** Creating a file whose long name needs more directory
  entries than fit in one cluster now returns the specific
  `FatError::DirEntryRunTooLong { entries_needed, entries_per_cluster }`
  instead of the misleading `DirectoryFull`.

### Known limitations

- **async + cache:** The FAT-sector cache is sync-only. Driving a volume
  through the async API silently bypasses the cache (async-aware caching is
  deferred — see the `cache` feature note in `hadris-fat/Cargo.toml`).
- **LFN cross-cluster runs:** Directory entry runs that would span a cluster
  boundary during LFN write are not yet supported; such a name is rejected
  up front with `FatError::DirEntryRunTooLong` (`hadris-fat/src/write.rs`).
- **exFAT:** Work in progress; gated behind the `exfat` feature.

## [1.1.0] - 2026-03-12

Baseline for this changelog. See the git history for changes at and before this
tag.

[Unreleased]: https://github.com/hxyulin/hadris/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/hxyulin/hadris/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/hxyulin/hadris/releases/tag/v1.1.0
