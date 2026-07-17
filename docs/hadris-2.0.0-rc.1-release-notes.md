# Hadris 2.0.0-rc.1 Release Notes

Hadris 2.0.0-rc.1 is the first release candidate for Hadris V2. It is intended
for downstream testing before the stable 2.0 release. Please try it against
real disk images, storage devices, boot workflows, and constrained targets,
especially where your application depends on public API details or unusual
on-disk layouts.

## The Rust storage stack

Hadris V2 presents the workspace as one layered, pure Rust storage ecosystem:

```text
Applications, bootloaders, kernels, firmware, and embedded systems
                              |
                    hadris umbrella crate
                              |
          hadris-block | hadris-optical | hadris-archive
                              |
        FAT + GPT/MBR  |  ISO/UDF  |  CPIO
                              |
              shared I/O, paths, and storage
                              |
           files, disk images, and block devices
```

Applications may depend directly on a focused format crate, use a category
facade for detection and opening, or adopt the `hadris` umbrella crate. The
facades preserve each format's concrete capabilities rather than forcing
unlike filesystems and archives behind a lowest-common-denominator trait.

The stable V2 surface covers FAT12/16/32, MBR and GPT partition tables,
ISO 9660, UDF 1.02, CPIO newc/SVR4 archives, and ISO/UDF bridge-image
creation. exFAT is available only as a separately gated unstable preview.

## Highlights

### Consistent APIs and feature tiers

- Lifecycle operations now consistently distinguish detection, opening an
  existing object, creating a new object, formatting a complete volume, and
  finishing a writer.
- Category facades provide non-destructive format detection and policy-driven
  opening while retaining format-specific errors and handles.
- Platform support (`std`, `alloc`, or allocation-free configurations),
  capabilities (`read` and `write`), and I/O modes (`sync` and `async`) are
  selected independently where supported.
- Mode-specific APIs live under matching `sync` and `async` modules. Writers
  that are genuinely synchronous remain explicitly sync-only.
- Public APIs are documented and checked with missing-documentation lints and
  public API snapshots.

### FAT12, FAT16, and FAT32

- Read, write, modification, and formatting support use the canonical
  `FatVolume` and `FatVolumeBuilder` names.
- VFAT long filenames support names up to 255 UTF-16 code units, including
  supplementary-plane characters and entry runs spanning directory-cluster
  boundaries.
- Corrupt cluster-chain handling, FSInfo flushing, I/O context, timestamps,
  volume labels, status flags, and optional synchronous FAT-sector caching
  have been strengthened.
- Direct and facade-level async tests cover opening, traversal, nested
  directories, multi-cluster reads and writes, truncation, and source recovery.

### Partition tables and block opening

- MBR, GPT, and hybrid layouts have normalized sync and async lifecycles.
- GPT opening validates primary and backup headers, CRCs, geometry, and
  reciprocal locations.
- Bounded partition views avoid manual offset arithmetic and can be passed
  directly to filesystem openers.
- Detection restores the caller's stream position.

### ISO 9660

- The new `IsoReader` provides allocation-free sync/async ISO 9660 and Joliet
  traversal, path lookup, and caller-buffered multi-extent file streaming for
  bootloaders, firmware, and kernels.
- Reading and writing cover ISO 9660 Levels 1-3, ISO 9660:1999, Joliet, SUSP,
  Rock Ridge, and El Torito workflows.
- Rock Ridge writing now includes POSIX metadata, timestamps, symlinks, device
  nodes, and deep-directory relocation.
- Namespace roots can be enumerated and selected explicitly.
- Async readers support descriptor access, directory traversal, lookup, and
  file reads; image writing remains synchronous.
- Allocation-floor support permits safe composition with other on-disc
  metadata without overlapping ISO file allocations.

### UDF and hybrid optical images

- UDF 1.02 images can be opened, traversed, read, and created.
- File reads and directory-entry sizes are available through the public API.
- The optical facade detects ISO-only, UDF-only, and bridge images and supports
  explicit opening policy in both sync and async modes.
- `hadris-cd` creates ISO 9660/UDF bridge images whose shared files are
  independently reopened and verified byte-for-byte through both readers.

### CPIO

- CPIO newc/SVR4 archives support sequential reading and archive writing in
  both sync and async modes.
- The owning `CpioArchiveWriter` returns its target from `finish`.
- Malformed-header, truncated-input, and untrusted-allocation paths have
  dedicated regression coverage.

### Command-line tools

V2 provides a specialized, consistent command family:

- `hadris-fat`
- `hadris-iso`
- `hadris-udf`
- `hadris-cpio`
- `hadris-cd`

The tools share familiar operations such as `info`, `ls`, `cat`, `extract`,
`create`, and `verify` where those operations fit the format. Existing
executable names remain compatibility aliases. The former unpublished
`hadris-cli` debug stub is not part of V2.

## Breaking changes and migration

V2 standardizes names and ownership across the workspace. Important examples
include `FatFs` becoming `FatVolume`, `DiskPartitionScheme` becoming
`PartitionTable`, field-style builder setters replacing many `with_*` methods,
and explicit `sync`/`async` module organization. Several mechanical aliases and
deprecated forwarding methods remain for the prerelease migration period, but
new code should use the canonical V2 names.

Feature selection is also more explicit. Do not assume that `std` selects an
I/O mode, or that enabling `async` provides asynchronous writers for formats
whose writer is currently synchronous.

See the [Hadris 1.x to 2.0 migration guide](hadris-1-to-2-migration.md) for
actionable upgrade instructions. The
[public API audit](hadris-2-api-audit.md) provides the detailed compatibility
table.

## Unstable exFAT preview

The `hadris-fat` crate exposes exFAT only through the opt-in
`unstable-exfat` feature. This preview:

- is outside the Hadris V2 API stability promise and public API snapshot;
- is not exposed by the `hadris` umbrella crate or the unified block opener;
- is synchronous and allocation-backed;
- is not recommended for irreplaceable data.

Basic formatting, reading, traversal, and simple mutation work on conventional
layouts. Fragmented allocation-bitmap or up-case metadata, directory growth,
general cross-cluster directory entry-set placement, async operation, TexFAT,
and repair workflows are not supported.

## Known limitations

- FAT-sector caching is synchronous. Async FAT volumes bypass the cache.
- ISO, UDF, and hybrid optical image writing are synchronous; their read APIs
  provide the supported async surfaces.
- UDF write support targets UDF 1.02. Later UDF revisions and their specialized
  media workflows are not part of this release candidate.
- A UDF Volume Recognition Sequence identifies an NSR revision family, so the
  reported revision is not guaranteed to be the exact revision used by the
  medium in every case.
- CPIO remains a sequential archive API rather than a filesystem-volume API.
- The in-repository ISO specification notes are incomplete developer material;
  they do not define the supported public API.

## What prerelease testers should exercise

We especially welcome testing of:

- API migrations from Hadris 1.x, including feature configurations;
- `no_std`, allocation-constrained, bootloader, firmware, and kernel builds;
- async FAT, partition, ISO, UDF, block-facade, and optical-facade reads;
- FAT mutation, maximum-length Unicode filenames, fragmented files, and
  cross-cluster directory entry runs;
- primary/backup GPT recovery and corrupt or truncated partition tables;
- ISO namespace selection, Rock Ridge metadata, deep directories, and
  BIOS/EFI El Torito images;
- UDF 1.02 images from third-party tools and ISO/UDF bridge images;
- CPIO archives used as initramfs images;
- images exchanged with `fsck.fat`, `xorriso`, `udfinfo`, `mkudffs`, `cpio`,
  `7z`, operating-system mount tools, firmware, and virtual machines.

When reporting a problem, include the crate and exact prerelease version,
enabled Cargo features, Rust version and target, sync or async mode, the
operation performed, and the smallest reproducible image or code sample that
can be shared. For parsing or interoperability failures, tool output and image
provenance are particularly useful.

Please report bugs and migration problems through the
[Hadris issue tracker](https://github.com/hxyulin/hadris/issues). Security
issues should follow the private reporting process in
[SECURITY.md](../SECURITY.md).
