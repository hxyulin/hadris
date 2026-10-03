# Hadris 2.5.0 release preparation

This branch prepares a coordinated 2.5.0 release of the V2 workspace. It is
intended to be the final planned V2 minor release, without ruling out future
correctness or security fixes. The stable V2 APIs remain compatible. exFAT,
streaming, NTFS, and APFS retain their existing experimental status.

The release has not been tagged or published. The latest published V2 release
is 2.4.0. See [the changelog](../CHANGELOG.md#250---2026-10-03) for all changes
since that release, including the experimental APFS reader already on `main`.

## V3 backport review

The backports adapt the on-disk fixes to V2; they do not introduce V3's device,
filesystem, mount, or error APIs.

| V3 change | V2 outcome |
| --- | --- |
| `8533aab`: reject unaddressable FAT geometries | Reject FAT12/16 layouts at 65,525 clusters and above, missing fixed roots, and FAT32 layouts above 0x0FFFFFF5 clusters. Regression fixtures cover oversized FAT16 and FAT32 layouts. |
| `8d696b1`: report looping cluster chains as corruption | Cached chain reads now return `ClusterLoop` when their walk budget is exhausted and validate the byte range before decoding an entry. A healthy chain ending at the limit still succeeds. |
| `5fff445`: maximize exFAT cluster count | Binary-search the largest layout that fits both FAT copies and the data region. Also reject invalid cluster sizes before multiplication. |
| `bcfc451`: zero-pad supplementary escape sequences | Joliet and enhanced descriptors use zero padding. Detection accepts the recognized Joliet prefix, retaining compatibility with old space-padded V2 images in readers and modifiers. |
| `82c25a4`: refuse empty El Torito boot images | Reject an empty boot image with `InvalidInput`. |
| Rock Ridge relocation fixes | Already present on V2 `main`, including reuse of recognized relocation containers and external extraction coverage. |
| V3 exFAT cancellation, vendor allocation, and TexFAT changes | The replacement V3 `ExFatFs` and raw I/O layer are not API-compatible backports to the synchronous V2 preview. They are not included in this release. |
| Embedded FAT and exFAT changes | V3-only APIs; not included. |
| V3 session, hard-link node identity, and Volume publication changes | Depend on the V3 filesystem/session contracts; not included as part of this selected backport set. |

This is a selected backport review, not a claim that every V3 fix applies or
that all V2 corruption and interrupted-write cases have been resolved.

## Native macOS qualification

APFS tests build and populate temporary images with `hdiutil`, detach them,
and verify them with `fsck_apfs -n` before reading them through Hadris. Coverage
includes both case-sensitive and case-insensitive volumes, a 1,500-file
directory, nested paths, large files, sparse extents, symlinks, hard links, and
explicit rejection of compressed data.

The native path comparison exposed two reader defects: a trailing slash on a
regular file was accepted, and `.` / `..` components were not resolved. Both
sync and async readers now handle these paths and reject intermediate regular
files, including `file/../other`. Traversal above the root stays at the root.
Symlinks are returned without following them. Root-only paths continue to
return no entry; callers use the root-directory APIs.

Running the existing exFAT writer round trips through macOS `fsck_exfat -n`
exposed active orphaned secondary entries after deletion. Deletion now clears
the InUse bit on the primary and every secondary before reclaiming the file's
clusters; slot allocation recognizes all inactive entry types. A raw-entry
regression verifies both deactivation and reuse. Native checker tests attach
only temporary images read-only and detach them on exit.

APFS remains an experimental reader. Unicode normalization, compressed data,
encrypted volumes, snapshots, and following symlinks are outside its supported
scope. These limitations do not expand the stable V2 support promise.

## Local verification

- Workspace all-feature hosted tests on macOS, plus affected-crate Clippy.
- Warning-denied workspace compilation at Rust 1.88, including all features.
- Every block and optical CI feature tier at Rust 1.88, checked independently.
- Standalone conformance suite: 41 passed and 9 explicitly ignored tests.
  Linux-only peer tools are absent locally; strict external-tool coverage runs
  in Linux CI.
- Native FAT formatter/checker qualification, macOS exFAT writer round trips,
  APFS native-image tests, and an enhanced ISO read by macOS `bsdtar`.
- The exFAT deletion/reuse regression under nightly Miri. The test uses
  `MIRIFLAGS=-Zmiri-disable-isolation` because the production timestamp provider
  reads the host clock.
- Formatting, documentation consistency, specification annotations, and
  compliance catalog checks.
- `scripts/release-check.sh 2.5.0` passed on Rust 1.97.1 with warnings denied,
  including workspace tests and doctests plus packaging verification of
  `hadris-fixed`, `hadris-io`, `hadris-path`, and `hadris-macros`. Full workspace
  packaging follows prerequisite publication, as described in the release script.

## Release procedure

1. Review and merge this branch into `main` after CI passes on Linux, Windows,
   and macOS. Confirm the changelog date at release time.
2. Run the `Release` workflow from `main` with version `2.5.0` and mode
   `dry-run`. `scripts/release-check.sh 2.5.0` provides the local release gate.
3. Run the workflow in `publish` mode after its dry run passes. It publishes
   in dependency order, creates the tag and release, and rebuilds versioned docs.

No registry publication or release workflow is triggered by preparing this
branch. The documented Linux peer-tool qualification remains a CI gate; local
macOS tests complement it rather than replace it.
