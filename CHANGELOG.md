# Changelog

All notable changes to this workspace are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Each published package owns its version and may be released independently.

## [Unreleased]

## [3.0.0-rc.1] - Unreleased

The first release candidate of the 3.0 API. Every published crate moves to
`3.0.0-rc.1` together; `hadris-fat-raw` is new and starts at 0.1.0. After
3.0.0 each crate versions independently. This section describes 3.0 as a
whole against 2.4. The
[migration guide](docs/hadris-3.0.0-migration.md) maps every 2.4 crate, item
and command to its 3.0 replacement, and
[`docs/v3-api-design.md`](docs/v3-api-design.md) records the design.

### Added

- **hadris-fs:** New crate with the vocabulary every format shares
  (`NodeId`, `Name`, `Metadata`, `DateTime`, `MountOptions`), one
  `FileSystem` trait per mode, `Volume` with `File` and `ReadDir` handles,
  `Walk`, the writer input `Tree`, `Node` and `Content`, `Report`,
  `copy_tree`, `read_tree`, `Finding` and `CheckReport`, a `host` module
  (`read_tree`, `write_tree`, `file`, `source_date_epoch`,
  `local_utc_offset`, `mount_options`) and the `contract` driver test kit.
- **hadris-fat-raw:** New crate, version 0.1.0. FAT12/16/32 and exFAT
  on-disk layouts, codecs that do no I/O, device primitives and checkers
  that need no allocator.
- **hadris-cli:** New package that installs one `hadris` binary with `fat`,
  `iso`, `udf`, `cpio` and `detect` commands.
- **hadris:** `hadris::{sync, r#async}::{detect, open, AnyFs}`,
  `ImageFormat` and `hadris::host::open`. `detect` recognizes FAT12/16/32,
  exFAT, ISO 9660, UDF, ISO/UDF bridges, cpio, MBR, GPT and NTFS and
  reports damage; `open` mounts the first filesystem it finds.
- **hadris-fat:** The embedded API for firmware without an allocator:
  `embedded::{sync, r#async}::Fat<D, FILES>` reads and writes FAT12/16/32,
  and `exfat::embedded::{sync, r#async}::ExFat<D, FILES>` reads exFAT, over
  512-byte blocks in about 1 KiB of state. Sizes per target are in the
  [embedded guide](website/docs/guides/embedded.md).
- **hadris-fat:** exFAT is stable as `ExFatFs`, with reads, writes,
  `format` and `check`. `fat::{sync, r#async}::write` builds a FAT volume
  from a `Tree`.
- **hadris-fat, hadris-iso, hadris-udf:** Format extras on each driver:
  `info()`, file extents, raw records and `read_raw`, El Torito boot
  catalogs, `MountOptions::backup_boot`, and `cluster_chain` for FAT.
- **hadris-iso:** Appended partitions (`Hybrid::with_appended`) so an EFI
  system partition is stored once for El Torito and GPT, `IsoId` and
  `IsoDate`, and ISO 9660 sessions over a mounted image.
- **hadris-storage:** `Partition<D>`, a byte window of a device;
  `host::FileDevice`, which reports the size of disk devices on macOS,
  FreeBSD, Windows and Linux; `Vec<u8>` as a device; `max_block_count`.
- **hadris-io:** `Error<E>` with `ErrorKind` (including `NotRecognized`),
  a static message, `Location`, per-format detail codes and `errno()`.
  Optional `embedded-io` and `embedded-io-async` bridges through
  `FromEmbedded`.
- **hadris-ntfs:** A read-only `NtfsFs` without an allocator, in both
  modes. It stays a preview behind the umbrella's `unstable-ntfs`.
- **Docs:** A versioned documentation site, the migration guide, and
  guides for detection, the embedded API and each format.

### Changed

- **All crates:** Every crate has the `std`, `alloc`, `sync` and `async`
  feature axes, with `std` and `sync` on by default. Features only add
  items and never change behaviour. I/O items live in the mode modules
  (`hadris_fat::sync::FatFs`) and are not re-exported at crate roots.
- **All crates:** One async mode, `r#async`, whose futures are `Send`
  when the device is. The `async_send` modules are gone; non-`Send` async
  uses the `local` device traits or the embedded API.
- **All crates:** One error shape. Format crates return `Error<E>` with a
  `Detail` read by `Detail::of`, mounts return `MountError`, which gives
  the device back, and writers return `PathError` with the failing path.
  Bytes that are not the format fail with `NotRecognized`.
- **All filesystems:** Mount a `hadris-storage` `BlockDevice` with
  `mount(dev, MountOptions)`, implement the one `FileSystem` trait, and
  get paths and handles through `Volume`. `IsoImage` and `IsoView` merge
  into `IsoFs`, `UdfVolume` becomes `UdfFs`, and FAT's `FatVolume`
  becomes `FatFs`.
- **All writers:** ISO 9660, UDF, the ISO/UDF bridge, cpio and FAT take a
  `hadris_fs::Tree`. `plan` does no I/O and returns the `Report`; `write`
  runs in each mode. Writers read no clock and no RNG: `with_time` and
  `with_seed` set the inputs, serials and ids derive from them and the
  tree, and the same input gives the same image.
- **hadris-fat:** Short names use CP437 by default, FAT times are local
  time with a UTC offset from `MountOptions`, and names compare by folding
  UTF-16 units. `format` takes a `Geometry` with `FatOptions` or
  `ExFatOptions`.
- **hadris-iso:** The options follow one shape: `IsoId`, `Hybrid`,
  `ElTorito` with `BootEntry` values, and `Relocation`.
- **hadris-udf:** The ISO 9660 and UDF bridge writer moves here from
  `hadris-cd` as `plan_bridge` and `write_bridge`.
- **hadris-cpio:** `CpioReader` streams entries with configurable name storage,
  `path`/`path_str`, absolute header/data offsets and `next_segment` for
  concatenated archives; oversized names return `LimitExceeded`. `into_parts`
  preserves the byte peeked at a segment boundary. CRC trailers are checked.
  `Writer` appends entries
  and returns its sink from `finish`; `Format` names Newc, Crc, Odc and
  Binary. Odc is new.
- **hadris-part:** Partition tables work on block devices, `open` returns
  a `hadris_storage::Partition`, and GUIDs are passed in.
- **hadris:** Re-exports `io`, `storage` and `fs` in every build and each
  enabled format at a flat path (`hadris::fat`, `hadris::cpio`).
- **hadris-common, hadris-macros:** Internal. `hadris-common` keeps only
  the endian integers.
- **CLI:** `hadris <format> <command>` replaces the per-format binaries.
  `create` takes `-o` and refuses an existing output without `-f`, then
  replaces it atomically; `extract` never replaces an existing file;
  `verify` exits non-zero on findings; `hadris fat` handles exFAT; cpio
  accepts `-` for standard input and output and `extract --path`.
- **Release:** Releases are per crate, tagged `<crate>-v<version>`.

### Removed

- **Crates:** `hadris-block` and `hadris-optical` (use `hadris::detect`
  and `hadris::open`), `hadris-cd` (use the bridge writer in
  `hadris-udf`), `hadris-archive` (use `hadris-cpio`), `hadris-path`
  (merged into `hadris-fs`), `hadris-fixed`, and the five CLI crates.
- **Features:** `read`, `lfn`, `cache`, `tool`, `unstable-exfat`,
  `dirty-file-panic`, `crc`, `rand`, `joliet` and `unstable-streaming`,
  and the umbrella's `block`, `optical`, `cd`, `archive`, `path`, `fixed`
  and `storage`.
- **hadris-io:** The V2 stream traits and `hadris_io::legacy`.
- **hadris-storage:** `WriteError`, `StorageError`, `OutOfRange`,
  `PartitionView`, and `BlockDevice` for `std::fs::File` (use
  `host::FileDevice`).
- **Format crates:** Their own error enums and `Report` types, and every
  V2 reader and writer type; the migration guide lists each with its
  replacement.

### Fixed

- **hadris-iso:** Joliet and enhanced volume descriptors pad their escape
  sequence field with zeros (ECMA-119 8.5.6); `bsdtar` read images with
  an enhanced tree as empty.
- **hadris-iso:** Every directory record of a file over 4 GiB carries its
  Rock Ridge entries, which `bsdtar` and xorriso need.
- **hadris-iso:** Images end in 150 zero blocks counted in the volume
  space size, as xorriso writes; `isoinfo` refused images under 48 blocks.
- **hadris-iso:** Level 1 and 2 identifiers keep the `.` separator of names
  without an extension (ECMA-119 7.5.1), map each character to one byte,
  and keep the extension when a long name is cut. Joliet names replace
  forbidden characters.
- **hadris-iso:** Rock Ridge relocation writes placeholders libarchive
  reads and reuses an existing root `rr_moved` directory. Hard link names
  share one node id in the Rock Ridge view.
- **hadris-iso:** A session that replaces a boot image updates the boot
  catalog's load size and boot information table.
- **hadris-iso:** Boot code over 446 bytes and invalid El Torito load
  sizes fail instead of being cut or written.
- **hadris-udf:** The writer refuses UDF 2.50 and 2.60, which need a
  metadata partition it does not write.
- **hadris-udf:** The volume space size of a bridge image's ISO 9660 side
  covers the whole image.
- **hadris-fat:** Generated short names map non-ASCII characters through
  the code page correctly, and short names that differ only in bytes above
  `0x7F` stay distinct.
- **hadris-fat:** A looping cluster chain fails with `Corrupt` instead of
  listing entries forever.
- **hadris-fs:** Copying or extracting a tree whose directory entries loop
  fails with `Corrupt` instead of running forever.
- **hadris-part:** GPT CRCs are correct with the `write` feature.
- **hadris-macros:** `send_async!` passes malformed input through to rustc
  instead of panicking.
- **CLI:** A failed `create` no longer truncates an existing output, and
  cpio and UDF extraction refuse paths that escape the output directory.

## [2.4.0] - 2026-09-08

### Added

- **hadris-iso (`unstable-streaming`):** `InputEntryKind::Source` streams a
  file's contents from a reader that is opened while the image is written, so a
  large input no longer has to be held in memory. `FileSource::from_path`
  streams a file on disk. The feature is outside the V2 stability promise.
  ([@zone117x](https://github.com/zone117x),
  [#111](https://github.com/hxyulin/hadris/pull/111))
- **hadris-udf (`unstable-streaming`):** `SimpleFile::from_source` streams a
  file's contents from a `FileSource` that is opened while the image is
  written. The feature is outside the V2 stability promise.
  ([@zone117x](https://github.com/zone117x),
  [#115](https://github.com/hxyulin/hadris/pull/115))

### Fixed

- **hadris-fat (`unstable-exfat`):** The exFAT writer now records a file's size
  as the stream extension's `DataLength` instead of its cluster-rounded
  allocation, and an empty file keeps `AllocationPossible` set with
  `NoFatChain` clear so both `fsck_exfat` and `fsck.exfat` accept it.
  ([@zone117x](https://github.com/zone117x),
  [#116](https://github.com/hxyulin/hadris/pull/116))
- **hadris-iso:** Directory iteration now stops after a malformed record or I/O
  error instead of returning the same error on every call.
  ([@zone117x](https://github.com/zone117x),
  [#113](https://github.com/hxyulin/hadris/pull/113))
- **hadris-iso:** Images too large for an MBR partition entry or for the
  32-bit ISO 9660 volume space size now fail with `InvalidInput` instead of
  writing a truncated sector count.
  ([#105](https://github.com/hxyulin/hadris/issues/105))
- **hadris-iso:** Path table iteration now stops after an I/O or parse error
  instead of returning the same error on every call.
- **hadris-udf:** Files of 1 GiB or more are now recorded with several short
  allocation descriptors. The single descriptor written before overflowed its
  30-bit extent length, so such files read back empty or truncated.
- **hadris-udf (`unstable-streaming`):** An interrupted read from a
  `FileSource` is retried instead of failing the image.

## [2.3.0] - 2026-09-02

### Added

- **hadris-iso:** The ISO writer now emits multi-extent files larger than 4 GiB.
  The allocating reader can consume them incrementally with
  `read_file_chunked`.
  ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#96](https://github.com/hxyulin/hadris/pull/96))
- **hadris-part:** Added `MbrPartition::new_iso_partition` for whole-image
  ISO 9660 partitions in isohybrid-style MBRs.
  ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#96](https://github.com/hxyulin/hadris/pull/96))

### Changed

- **Tests:** FAT and ISO now use the same hosted, external-tool, and manual
  test tiers. CI requires mtools and dosfstools for FAT interoperability and
  xorriso for ISO interoperability, while local runs skip unavailable tools.
- **Development:** The pinned Rust toolchain now installs `rust-analyzer` for
  editor support. ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#100](https://github.com/hxyulin/hadris/pull/100))

### Fixed

- **hadris-iso:** Multi-extent directory planning now counts continuation
  records before assigning file data, preventing creation failures when a
  continuation crosses a directory-sector boundary. `WrittenFile` retains its
  v2.2 field layout for source compatibility.
- **hadris-fat:** FAT name handling is now case-insensitive for long names,
  rejects reserved characters, permits case-only renames, prevents moving a
  directory into its own subtree, and generates valid aliases for leading-dot
  names. Readers still accept existing nonconforming names and prefer an exact
  long-name match when an old image contains entries that differ only by case.
- **hadris-fat:** Failed writes caused by a full data region now release their
  uncommitted clusters and persist a valid FAT32 FSInfo count and next-free
  hint instead of leaving orphaned allocations or stale metadata.
- **hadris-fat:** File and directory creation and rename now allocate distinct
  8.3 aliases when long filenames share the same generated short name. Such
  entries previously appeared through their long names but were rejected as
  duplicate directory entries by `fsck.fat`.
- **hadris-iso:** Non-final extents of multi-extent files are now sized as a
  multiple of the configured logical block size instead of always 2048 bytes,
  as required by ECMA-119 6.5.4.

## [2.2.0] - 2026-08-24

### Added

- **hadris-fat:** `FileReader` now supports start-, current-, and end-relative
  seeking in both the synchronous and asynchronous APIs, including buffered
  and cached-chain readers.
  ([@stlankes](https://github.com/stlankes),
  [#89](https://github.com/hxyulin/hadris/pull/89))
- **hadris-fat:** New `Error::StaleEntry` (with the `write` feature) reported by
  the mutating APIs and `FileReader` when a `FileEntry` handle no longer refers
  to the same on-disk directory entry (see below).
- **hadris-fat:** New `Error::WriterConflict` (with the `write` feature) returned
  when a second `FileWriter` is opened for a directory entry that already has one
  open (see below).
- **hadris-iso:** New `HybridBootOptions::efi_boot_partition` field and
  `with_efi_boot_partition` builder to expose an El Torito UEFI boot image as a
  GPT EFI System Partition (the equivalent of xorriso's
  `-efi-boot-part --efi-boot-image`). When unset, the writer automatically uses
  the boot image of the single `PlatformId::UEFI` El Torito entry, if there is
  exactly one, so existing UEFI/hybrid consumers gain the ESP without code
  changes. `PlatformId` now derives `PartialEq`/`Eq`.
- **hadris-iso:** New `SizeBreakdown::backup_gpt` estimator component covering
  the backup-GPT region appended to GPT/Hybrid images, so size estimates keep
  their never-underestimates contract for those schemes.

### Fixed

- **hadris-iso:** GPT and Hybrid images now write a complete GPT. The writer
  appends a backup-GPT region (backup entry array plus backup header) after the
  ISO data, so `alternate_lba` points at a real backup header instead of
  unwritten sectors, and both headers use the standard 128-entry array via
  `hadris-part::GptDisk` (previously 4 or 2 entries were advertised and no
  backup was written). `volume_space_size` covers the appended region, so tools
  that copy `volume_space_size` logical sectors preserve the backup GPT.
- **hadris-iso:** The GPT partition layout is now meaningful: basic-data
  partitions cover exactly the ISO data area (the previous single partition
  spanned LBA 34 to `disk_size - 34`, an extent with no valid filesystem), and
  the EFI System Partition entry covers the El Torito UEFI image's exact
  sectors. Partitions start no earlier than 512-byte LBA 64, where the ISO
  volume descriptors begin; tools that convert ISOHYBRID GPT images to MBR
  (such as `limine bios-install`) embed boot code below sector 63 and reject
  partitions starting earlier. The hybrid MBR mirrors the ESP as type `0xEF` alongside the `0x17`
  ISO partition instead of mirroring only the bogus basic-data range.
- **hadris-part:** `HybridMbrBuilder` no longer undersizes the protective MBR
  entry by one sector. The entry now covers LBA 1 through the sector before the
  first mirrored partition inclusive (33 sectors for a partition starting at
  LBA 34), and `end_chs` is consistent with `sector_count` in every branch.
- **hadris-part:** Corrected the byte values of many well-known partition-type
  `Guid` constants, which did not match their canonical GUIDs (all `FREEBSD_*`,
  `SOLARIS_*`, `NETBSD_*`, and `VMWARE_*` constants, the `LINUX_ROOT_*`,
  `LINUX_HOME`, `LINUX_SRV`, and `LINUX_LUKS` constants,
  `WINDOWS_STORAGE_SPACES`, and several `APPLE_*` constants). All constants are
  now defined from their canonical GUID strings at compile time, with the
  sources referenced in the code and a regression test asserting each
  constant's textual form.

- **hadris-fat:** Overwriting an existing file through `write_file` no longer
  leaks the file's previous cluster chain. The writer now follows and reuses
  the existing chain, and `finish()` frees any tail clusters left over when
  the new contents are shorter than the old, keeping the FAT and the FSInfo
  free-cluster count consistent. Previously every overwrite of a multi-cluster
  file orphaned all but its first cluster, eventually failing with
  `NoFreeSpace` (#90).
- **hadris-fat:** `create_dir` no longer leaks a cluster when it fails after
  allocating the directory's data cluster (e.g. `DirectoryFull` on a full
  fixed root, or an over-long name). The data cluster is now allocated only
  after the fallible name/slot steps succeed, matching `create_file`.
- **hadris-fat:** `FileWriter::new_append` no longer overwrites the last
  cluster of a file whose size is an exact multiple of the cluster size. The
  writer now positions past the full final cluster so appended data extends
  the file instead of clobbering its tail.
- **hadris-fat:** Stale `FileEntry` handles can no longer corrupt an unrelated
  file. If a file is deleted and its directory slot reused, operating through
  the old handle previously freed the new file's chain, cleared its
  `DIRECTORY` bit, moved it, or clobbered its entry. `delete`, `rename`,
  `truncate`, `set_attributes`, `set_times`, and `FileWriter::finish` now
  revalidate the slot's on-disk short name, creation timestamp, and
  not-deleted marker before acting and return `Error::StaleEntry` on mismatch.
  FAT carries no per-entry identity, so this is best-effort: a slot re-taken by
  a same-named file is distinguished only by the creation timestamp, which has
  10 ms resolution and is constant under the `no_std` `EpochTimeProvider`.
  Passing `created` to `set_times` rewrites that field and therefore
  invalidates other handles to the same file. `delete` also marks the
  entry deleted before freeing its chain so a mid-operation error cannot leave
  a live entry pointing at freed clusters.
- **hadris-fat:** A `FileReader` held across a delete and cluster reuse no
  longer discloses the reusing file's data. The reader revalidates its
  directory slot before serving bytes and returns `Error::StaleEntry` instead.
- **hadris-fat:** `create_file` and `create_dir` through a stale `FatDir` (its
  directory was deleted and the cluster reused) no longer write directory
  entries into an unrelated file's data. Both revalidate the directory's own
  entry first and return `Error::StaleEntry` on mismatch. The root directory,
  which cannot be deleted, is always accepted.
- **hadris-fat:** `FileWriter::finish` now reports I/O and FAT-update failures
  instead of masking them. With the `dirty-file-panic` feature, an error
  returned part-way through `finish` used to trip the drop guard on the way
  out, panicking about a writer that had in fact been finished. The guard now
  fires only for a genuinely forgotten `finish()`.
- **hadris-fat:** Opening two `FileWriter`s for the same file no longer lets
  them independently allocate and cross-link one cluster chain (the last
  `finish()` won and orphaned the other writer's clusters). The volume now
  tracks the directory slot of each open writer and returns
  `Error::WriterConflict` for a second writer on the same entry; the slot is
  released when the writer is finished or dropped.
- **hadris-fat (`unstable-exfat`):** Extending a file across a cluster boundary
  onto a fragmented layout now writes the FAT chain links, so data past the
  first cluster is reachable on read-back. The writer linearizes a contiguous
  file's prefix into the FAT when it first becomes fragmented and links each
  appended cluster onto the tail.
- **hadris-fat (`unstable-exfat`):** `allocate_clusters` is now authoritative
  against the allocation bitmap. Its fragmented fallback previously scanned the
  FAT for zero entries and re-handed-out clusters the contiguous (bitmap-only)
  path had already allocated; it now reserves clusters from the bitmap and
  links them in the FAT, rolling the reservations back if either step fails
  and returning `NoFreeSpace` when the volume is full.
- **hadris-fat (`unstable-exfat`):** `create_dir`, `delete`, and `truncate` now
  flush the allocation bitmap. Previously only the file writer persisted it, so
  a directory's allocation or a freed chain was lost on remount.
- **hadris-fat (`unstable-exfat`):** Truncating a fragmented file now frees the
  dropped tail clusters in the allocation bitmap, not only in the FAT, so the
  space is reclaimed and the bitmap and FAT stay consistent.
- **hadris-fat (`unstable-exfat`):** Directory entry sets are no longer placed
  across a cluster boundary (they were written linearly, which is wrong for a
  fragmented directory), and scanning a directory with an unknown size no longer
  walks past its allocation into unrelated data.
- **hadris-fat (`unstable-exfat`):** Overwriting a contiguous, multi-cluster
  file in place now reuses the file's own already-allocated clusters. Crossing a
  cluster boundary during an overwrite previously saw the file's next cluster as
  "already allocated" and allocated a brand-new cluster instead, orphaning the
  old one in the bitmap and needlessly fragmenting the file (the #90 pattern for
  the exFAT path).
- **hadris-fat (`unstable-exfat`):** exFAT formatting computes its cluster-heap
  geometry in 64-bit arithmetic and range-checks the results. A volume larger
  than ~2 TiB previously truncated the sector count to `u32`, silently sizing
  the filesystem to a fraction of the device; oversized geometry now returns
  `VolumeTooLarge` instead of corrupting the layout.
- **hadris-udf:** dstring decoding no longer includes one trailing garbage byte
  (usually a NUL). The dstring length byte counts the compression-ID byte, so
  `PrimaryVolumeDescriptor::volume_id()`, `LogicalVolumeDescriptor::volume_id()`,
  and `FileSetDescriptor::file_set_id()` previously returned values like
  `"LABEL\0"` and equality comparisons against the written label failed. The
  decoder also guards hostile length bytes without panicking.
- **hadris-udf:** 16-bit (UTF-16) dstrings and filenames are now decoded with
  proper surrogate-pair handling. Characters outside the Basic Multilingual
  Plane were previously dropped from names, so files written with such names
  could not be listed or found by name. Unpaired surrogates decode to U+FFFD.
- **hadris-cd:** Creating an image without a name-preserving namespace (Joliet
  disabled and Rock Ridge off) no longer fails with `ISO writer did not produce
  the planned file` for names the ISO 9660 charset sanitizes, and no longer
  produces images whose ISO and UDF namespaces diverge for sanitized directory
  names. The writer now maps ISO names onto the shared tree before writing and
  deduplicates them within each directory.
- **hadris-cd:** Rock Ridge metadata requested with `rock_ridge` creation
  options or the CLI's `-R` flag is now written. The base ISO namespace did not
  advertise RRIP support, so hadris-iso silently skipped the system-use fields.
- **hadris-cd:** The CLI's `verify` command now compares Rock Ridge alternate
  names when present, so `-R` images verify cleanly.
- **hadris-iso:** `PrimaryVolumeDescriptor::new` and
  `SupplementaryVolumeDescriptor::new_evd` no longer panic on volume names
  longer than 32 characters. The constructors truncate or substitute invalid
  input, while the writer rejects an over-long `volume_name` with
  `InvalidInput`.
- **hadris-io:** Converting `ErrorKind::UnexpectedEof` and
  `ErrorKind::WouldBlock` to `std::io::ErrorKind` now preserves the kind instead
  of mapping both to `Other` through the embedded-io conversion.
- **hadris-io:** `Cursor::seek` uses checked position arithmetic. Overflowing
  seeks return `InvalidInput`, and `SeekFrom::Start` accepts offsets above
  `i64::MAX`, matching `std::io::Cursor`.
- **hadris-storage:** Building with neither the `sync` nor `async` feature no
  longer emits dead-code warnings.
- **tools:** All five CLI tools reset `SIGPIPE` to the default disposition on
  Unix, so piping output into commands such as `head` exits without a
  broken-pipe panic.
- **hadris-udf-cli:** `create --revision` now accepts only the supported UDF
  revisions: 1.02, 1.50, 2.00, 2.01, 2.50, and 2.60.

### Documentation

- **hadris-io:** Corrected the claim that the I/O traits re-export from
  `std::io` under the `std` feature. They are Hadris traits with blanket impls
  for standard-library types. The docs now mark `alloc` as a no-op, and the
  package no longer contains the unused `traits.rs`.
- **hadris-cd:** The README quick start now opens the output file for reading
  and writing, as required by `OpticalImageWriter::finish`. The runtime error
  for a write-only target now states that requirement.
- **hadris-fixed:** Documented that `From<&[u8]>` for `FixedBytes` panics on
  over-long slices and that `try_from_slice` is the fallible alternative.
- **hadris-common:** Corrected the `sync` feature's default in the crate-level
  feature table.

## [2.1.0] - 2026-08-18

### Added

- **hadris-fat:** Directory entries and parse results now implement `Clone`,
  allowing parsed metadata to be retained independently of directory iterators.
  ([@stlankes](https://github.com/stlankes),
  [#84](https://github.com/hxyulin/hadris/pull/84))
- **Release automation:** Added a manually triggered GitHub Actions workflow
  that validates and publishes the workspace crates in dependency order, tags
  the release, and creates GitHub release notes from this changelog entry.

### Changed

- **hadris-fat:** `TimeProvider` and `OemCpConverter` now require `Sync`, which
  allows a `FatVolume` that uses them to be moved into a mutex and shared across
  threads. ([@stlankes](https://github.com/stlankes),
  [#83](https://github.com/hxyulin/hadris/pull/83))

### Fixed

- **hadris-common:** Replaced the GPL-licensed `noalloc` dependency with a
  compatibility layer backed by `heapless` while preserving the existing
  fixed-capacity collection API.
- **Build:** Added a `cargo-deny` CI gate that rejects dependencies outside the
  workspace's permissive-license allowlist.
- **hadris-fat:** Rejected FAT32 images whose BPB root cluster is outside the
  data-cluster range at mount time, and treated directory entries whose first
  cluster is below 2 as empty, fixing `attempt to subtract with overflow`
  panics on corrupt images (found by `cargo fuzz run fat_read`).
- **hadris-fat:** Short (8.3) names containing OEM high bytes no longer panic
  `FileEntry::name`, `ShortFileName::matches`, or the `Debug` impl on
  untrusted images; such names are decoded lossily, and a non-panicking
  `ShortFileName::try_as_str` was added.
- **hadris-fat:** `FileReader::read_to_vec` no longer pre-allocates the full
  claimed file size, so a corrupt entry advertising gigabytes on a tiny image
  cannot force an out-of-memory abort.
- **hadris-part:** Partition end-LBA and size computations now saturate
  instead of panicking on corrupt MBR/GPT entries whose start + size exceeds
  the integer range (found by the new `part_read` fuzz target).
- **hadris-part:** GPT reads bound the untrusted partition-entry array
  (entry count and LBAs) against the actual image size before allocating or
  seeking, returning `DiskTooSmall`/`BackupHeaderIo` instead of panicking on
  multiply overflow or allocating hundreds of GiB.
- **hadris-fat (exFAT):** Boot-sector validation no longer panics on shift
  fields whose sum overflows `u8`, and cluster arithmetic (`cluster_to_offset`,
  `is_valid_cluster`, FAT table bounds) saturates instead of over- or
  underflowing on corrupt boot-region values (found by the new `exfat_read`
  fuzz target).
- **hadris-ntfs:** Mount-time record sizes (MFT/index) and per-stream sizes
  (`$BITMAP`, index allocation blocks, `FileReader::read_to_vec`) derived
  from untrusted on-disk fields are now bounded against the actual data
  source or volume capacity, so a corrupt boot sector or attribute cannot
  force an out-of-memory abort (found by `cargo fuzz run ntfs_read`).
- **hadris-ntfs:** Mount rejects a `$UpCase` stream whose declared size is
  not exactly the 128 KiB table before reading it, so a corrupt attribute
  claiming gigabytes (backed by a huge claimed volume) cannot force an
  out-of-memory abort (found by `cargo fuzz run ntfs_read`).
- **hadris-fat:** An LFN entry with the last-entry flag but a sequence count
  of zero no longer starts a long-name sequence, fixing an
  `attempt to subtract with overflow` panic in the sequence countdown
  (found by `cargo fuzz run fat_read`).
- **hadris-fat:** FAT12/16 fixed-root directory iteration no longer stops
  after the first 4 KiB window, which silently dropped entries past slot 128
  of larger roots (e.g. the standard 224-entry floppy root) and could make
  `find`/`open_path` miss existing files (found by manual audit).
- **hadris-fat (exFAT):** Mount rejects an allocation bitmap whose claimed
  `data_length` exceeds the cluster heap, directory iteration and file
  reads/seeks return `ClusterLoop` on cyclic FAT chains instead of looping
  forever, and allocation-bitmap cluster arithmetic saturates instead of
  over- or underflowing (found by manual audit).
- **hadris-fat (tool):** `scan_fat`/`verify` no longer pre-allocate from
  claimed BPB geometry, and the recursive directory walks cap nesting depth
  with a `CorruptFilesystem` error instead of overflowing the stack on cyclic
  directory graphs (found by manual audit).
- **hadris-ntfs:** `attr::parse_index_entries` validates a caller-supplied
  node-header offset with checked arithmetic and returns
  `InvalidIndexEntry` instead of panicking on out-of-range values
  (found by manual audit).
- **hadris-part:** `PartitionInfo::size_bytes`, CHS-to-LBA conversion,
  hybrid-MBR building, `GptDisk` geometry helpers, and the GPT write path
  now use checked or saturating arithmetic, fixing divide-by-zero and
  overflow panics on crafted tables — and, in release builds, potential
  writes to wrapped offsets. Reading a GPT with `block_size` below the
  header size returns `InvalidBlockSize`. The GPT fuzz-regression tests are
  now gated off the `crc` feature, which rejects the crafted images earlier
  with a checksum error (found by manual audit).
- **hadris-iso:** `IsoDir::read_entries` bounds a directory's claimed size
  against the actual image before allocating, matching `read_file`.
  `IsoModifier::open` returns an error on images without a primary volume
  descriptor instead of panicking, rejects cyclic directory graphs (depth
  cap plus extent-visit tracking) instead of overflowing the stack, and
  skips version-only (`;1`) directory names instead of panicking in
  `finish()` (found by manual audit).
- **hadris-iso:** Rock Ridge continuation areas are now written after the
  directory records that reference them, directory extents are assigned
  pre-order (parents before children), and file data is written after the
  whole directory region. Streaming readers only follow CE pointers to
  later positions and ignore directories whose extent is lower than their
  scan position, so libarchive/bsdtar previously rejected hadris-written
  Rock Ridge images outright with "Invalid parameter in SUSP CE
  extension" and still could not list nested directories once the CE
  placement was fixed. bsdtar now lists hadris-written images cleanly.
  The writer plans the full layout (sizes are extent-independent) before
  writing, so emitted images are deterministic and the floor set by
  `create_with_allocation_floor` still reserves the low sectors.
- **hadris-iso:** `pad_align_sector` now zero-fills padding instead of
  seeking past it, so emitted bytes no longer depend on the target
  reading unwritten regions back as zeros (reused buffers, block
  devices). Output on fresh files and memory cursors is unchanged.
- **hadris-udf:** `UdfWriter::write_file_entry` returns
  `TooManyAllocationDescriptors` instead of panicking when the descriptors
  exceed a file entry, and `UdfWriter::create` rejects directory trees
  deeper than 128 levels with `DirectoryNestingTooDeep` instead of
  overflowing the stack (found by manual audit).
- **hadris-udf:** Allocation descriptors and file-identifier ICBs are now
  parsed without requiring pointer alignment, so a corrupt File Entry with an
  odd `extended_attributes_length` (or FIDs at unaligned offsets) no longer
  panics inside bytemuck on the misaligned cast (found by
  `cargo fuzz run udf_read`).
- **hadris-cpio:** `read_entry_data` returns `Error::BufferSizeMismatch`
  before consuming any bytes instead of panicking when the caller's buffer
  does not match the entry's claimed file size (found by manual audit).
- **hadris-fat:** Directory enumeration now skips any entry carrying the
  `VOLUME_ID` attribute bit (except exact LFN components), so a corrupt
  entry combining `VOLUME_ID` with `DIRECTORY` is no longer listed and
  recursed into as a directory, matching the FAT spec and mtools (found by
  differential fuzzing against mdir).

## [2.0.0] - 2026-08-09

First stable release of the V2 API. This entry consolidates the changes made
across the `2.0.0-rc.4` candidate and the subsequent specification-conformance
work; the public API frozen during the release-candidate series is now stable
under Semantic Versioning.

### Added

- **hadris-ntfs:** Added an experimental, read-only NTFS leaf crate with
  validated boot geometry, MFT and directory traversal, resident/non-resident
  file reading, sparse runs, Unicode names, and sync/async `no_std` support.
  ([@aruiz](https://github.com/aruiz))
- **Facade crates:** Added package READMEs for `hadris-archive`,
  `hadris-block`, and `hadris-optical`.
- **Specification compliance:** Added a compliance catalog framework with
  pinned source digests and extracted requirement sets for ECMA-119, ECMA-167,
  UDF 1.02, the ECMA TR/71 bridge format, and source-bounded NTFS MFT
  behavior.

### Fixed

- **hadris-iso:** Enforced ECMA-119 invariants and validated descriptor
  conformance; the aligned image tail is now preserved.
- **hadris-udf:** Corrected anchor and Volume Descriptor Sequence layout, and
  made descriptor validation and decoding portable across targets.
- **hadris-cd:** Conformed the UDF bridge descriptor layout and reserved space
  for the trailing UDF anchor.
- **hadris-cpio:** Enforced `newc` archive invariants and accepted aligned
  trailerless archives.
- **hadris-fat:** Validated BPB geometry, enforced filesystem integrity rules,
  and supported checksum validation across read tiers.
- **hadris-part:** Hardened partition metadata handling.
- **hadris-iso-cli:** Extension filenames are displayed correctly and filename
  namespace semantics are respected.

### Changed

- **Build / MSRV:** The workspace now uses Cargo resolver 3 so dependency
  resolution prefers releases compatible with the declared Rust 1.88 MSRV.
- **Documentation:** Removed superseded V2 planning, migration, performance,
  and internal agent-design notes; consolidated the active specification
  annotation rules in `docs/spec-coverage.md`.
- **hadris-fat:** Expanded cache documentation with the builder, transparent
  routing, explicit flush, capacity, and sync-only behavior.
- **Specification compliance:** Full claims now require runnable test evidence;
  CI checks evidence existence and bidirectional coverage-table parity.
- **hadris-ntfs:** Public API documentation is now enforced with
  `deny(missing_docs)`.
- **hadris-cpio:** The optional parser is now crate-internal.

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
- Recorded caching and performance findings deferred out of 2.0.
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

- Workspace and crate READMEs updated for API accuracy and version `1.2.1`.

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
  ([@aruiz](https://github.com/aruiz),
  [#21](https://github.com/hxyulin/hadris/pull/21))
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
  ([@Jozefpodlecki](https://github.com/Jozefpodlecki),
  [#29](https://github.com/hxyulin/hadris/pull/29))
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

### Added

- **hadris-fat:** Added configurable FAT table readahead and reduced fragmented
  file reads to one I/O operation when `FileReader` uses a cached chain.
  ([@aruiz](https://github.com/aruiz),
  [#13](https://github.com/hxyulin/hadris/pull/13),
  [#15](https://github.com/hxyulin/hadris/pull/15))

### Fixed

- **hadris-fat:** Corrected FAT32 entry endianness and fixed cached builds to use
  the volume's cluster bounds. ([@aruiz](https://github.com/aruiz),
  [#11](https://github.com/hxyulin/hadris/pull/11),
  [#12](https://github.com/hxyulin/hadris/pull/12))
- **Build:** Disabled `thiserror` default features so the workspace builds as
  `no_std`. ([@aruiz](https://github.com/aruiz))

[Unreleased]: https://github.com/hxyulin/hadris/compare/v2.4.0...HEAD
[2.4.0]: https://github.com/hxyulin/hadris/compare/v2.3.0...v2.4.0
[2.3.0]: https://github.com/hxyulin/hadris/compare/v2.2.0...v2.3.0
[2.2.0]: https://github.com/hxyulin/hadris/compare/v2.1.0...v2.2.0
[2.1.0]: https://github.com/hxyulin/hadris/compare/v2.0.0...v2.1.0
[2.0.0]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.4...v2.0.0
[2.0.0-rc.3]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.2...v2.0.0-rc.3
[2.0.0-rc.2]: https://github.com/hxyulin/hadris/compare/v2.0.0-rc.1...v2.0.0-rc.2
[2.0.0-rc.1]: https://github.com/hxyulin/hadris/compare/v1.2.1...v2.0.0-rc.1
[1.2.1]: https://github.com/hxyulin/hadris/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/hxyulin/hadris/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/hxyulin/hadris/releases/tag/v1.1.0
