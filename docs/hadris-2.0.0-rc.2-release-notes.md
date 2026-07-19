# Hadris 2.0.0-rc.2 Release Notes

Hadris 2.0.0-rc.2 is the second release candidate for Hadris V2. It continues
the V2 feature and public-API freeze established by
[2.0.0-rc.1](hadris-2.0.0-rc.1-release-notes.md): changes are limited to
correctness fixes, interoperability qualification, additive spec features,
documentation, and release engineering. This candidate contains no breaking
changes to the frozen public API — every API change below is additive, except
the removal of one unused, always-empty stub type.

For the full V2 overview (the storage stack, feature tiers, per-format
capabilities, exFAT preview, and migration guidance), see the
[2.0.0-rc.1 release notes](hadris-2.0.0-rc.1-release-notes.md). This document
covers only what changed since rc.1.

## Highlights since rc.1

### FAT: lowercase 8.3 names round-trip correctly

Lowercase short names such as `readme.txt` are now stored as a single directory
entry carrying the Windows NT `DIR_NTRes` case flags (new `NtCaseFlags`,
`FileEntry::nt_case`, `ShortFileName::with_nt_case`) and read back in their
original case. Previously they were either uppercased on read or forced to spend
a long-file-name entry. This matches how Windows and the Linux `vfat` driver
present the same names, verified against `mtools` as an external reference. As
part of the fix, `rename` now records case flags for the *new* name rather than
carrying over the source entry's.

`Fat12` and `Fat16` also gained `allocate_chain` / `extend_chain`, matching
`Fat32`, for callers performing manual multi-cluster allocation.

### ISO: conformance and interoperability fixes

- **Joliet is now conformant UCS-2.** Characters outside the Basic Multilingual
  Plane are substituted with `_` instead of leaking UTF-16 surrogate pairs, and
  names are capped at the Joliet 64-character limit (previously up to 103).
- **Directory records are written in File Identifier order** (ECMA-119 9.3)
  instead of input/tree order, so images validate against strict readers.
- **Non-2048 logical block sizes are rejected on read.** `IsoImage::open` now
  returns a clear `Unsupported` error instead of silently misreading extents.
  (The allocation-free `IsoReader` already honors the declared block size.)
- **Files larger than 4 GiB fail with a clear error** instead of silently
  truncating their length to 32 bits. Multi-extent *write* (which would lift the
  limit) is not yet emitted and remains a documented limitation.

### ISO: additive boot and metadata features

- **El Torito emulation media types.** `EmulationType` now names the 1.2/1.44/
  2.88 MB floppy and hard-disk emulation modes with an `is_emulated` helper;
  emulated boot entries default their load size to one virtual sector.
- **Rock Ridge creation timestamps.** `TF` entries now include the creation time
  when the input entry carries one (new `RripBuilder::add_tf`), alongside the
  existing modify and access times.
- **No-alloc Rock Ridge name resolution.** The allocation-free `IsoReader` can
  resolve Rock Ridge alternate names via `IsoDirEntry::rrip_name_into` /
  `rrip_name_matches`, decoding and comparing the `NM` field without allocating.
  (Inline system-use areas only; `CE`-continued names are not followed.)

### Compliance annotations and specification notes

- Expanded the `@hadris-spec` compliance annotations and `docs/spec-coverage.md`
  to cover more ISO volume descriptors, path tables, and El Torito section
  entries, the FAT FSInfo sector, and a new `hadris-part` (MBR/GPT) section.
- Completed the in-repo ISO 9660 specification notes (volume descriptors, path
  table, directory record, and an extensions index) and documented the ISO
  writer's known limitations.

## Removed

- **hadris-iso:** the unused, always-empty `read::SupportedFeatures` bitflags
  stub. It carried no variants and was not referenced by any public API, so its
  removal does not affect working code.

## Known limitations

The rc.1 known-limitations list still applies. Newly documented in this
candidate:

- **ISO multi-extent write.** Files larger than 4 GiB cannot yet be written;
  the writer emits a single extent and rejects oversized inputs rather than
  producing a truncated image. Multi-extent *reading* is supported.
- **Non-2048 block sizes in `IsoImage`.** Reading such images returns
  `Unsupported`; the allocation-free `IsoReader` reads them.
- Additional niche ISO features (Volume Partition Descriptor bodies, interleaved
  files, zisofs, Extended Attribute Record contents, Rock Ridge `SF`/`RR`, and
  spec-valid backup GPT for hybrid images) remain out of scope for 2.0 and are
  recorded in the `hadris-iso` crate documentation.

## Performance and caching

No performance code landed in this candidate. The caching and performance
findings deferred out of the 2.0 line — the sync-only FAT sector cache, the
unbuffered allocation-free `IsoReader`, the unused `hadris-storage` block-device
traits, and the absence of FAT benchmarks — are recorded in
[`docs/hadris-2-perf-notes.md`](hadris-2-perf-notes.md) so the decisions are not
re-derived later.

## What prerelease testers should exercise

In addition to the rc.1 testing focus, this candidate especially benefits from:

- FAT images with lowercase short names, cross-checked against `mtools`,
  Windows, and the Linux `vfat` driver;
- ISO images validated by strict readers for directory-record ordering and
  Joliet name conformance;
- BIOS boot workflows using El Torito floppy and hard-disk emulation entries;
- Rock Ridge images carrying creation timestamps and alternate names read
  through the allocation-free `IsoReader`.

Please report bugs and migration problems through the
[Hadris issue tracker](https://github.com/hxyulin/hadris/issues). Security
issues should follow the private reporting process in
[SECURITY.md](../SECURITY.md).
