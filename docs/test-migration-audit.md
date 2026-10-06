# Test migration audit

This audit addresses issue #125 against the V3 tree based on `64e5b1e6`.
It compares the tests changed by `55a0e3d` and `8a58ba2` with their parents,
then follows the scenarios into the current crate tests and detached suite.
It assesses behavior and assertions, rather than treating equal test counts
or matching names as proof of equivalent coverage.

## Historical changes

| Commit | Historical file | Test names removed or replaced in that file |
|---|---|---:|
| `55a0e3d` | FAT `comprehensive_fat.rs` | 42 |
| `55a0e3d` | FAT `fat_roundtrip.rs` | 17 |
| `55a0e3d` | NTFS `poc_audit_ntfs.rs` | 2 |
| `55a0e3d` | ISO `benchmark.rs` | 9 |
| `55a0e3d` | ISO `comprehensive_iso.rs` | 50 |
| `55a0e3d` | UDF `comprehensive_udf.rs` | 45 |
| `8a58ba2` | FAT `fat_conformance.rs` | 7 |
| `8a58ba2` | ISO `xorriso_advanced.rs` | 2 |
| `8a58ba2` | ISO `xorriso_basic.rs` | 9 |
| `8a58ba2` | ISO `xorriso_boot.rs` | 7 |
| `8a58ba2` | ISO `xorriso_hybrid.rs` | 3 |

These are changes to function names in the historical files, not counts of
lost behavioral checks. All 21 ISO xorriso test names are discovered in the
current detached test binary. The seven FAT tests are split between current
oracle checks, interoperability checks and manual peer/native reports.

## Assertion and scenario mapping

Paths below are relative to the repository root. A replacement may combine
several historical cases in a table or exercise their behavior through V3's
shared filesystem API.

| Historical scenarios and assertions | Current evidence and disposition |
|---|---|
| FAT invalid boot signatures, zero/invalid sector and cluster sizes, FAT12/16/32 detection, FAT32 version, root cluster and cluster arithmetic | `crates/block/hadris-fat-raw/src/boot.rs`: `check_bpb_rejects_bad_sizes`, `cluster_shift_matches_every_valid_sector_and_cluster_size`, `floppy_is_fat12`, `many_clusters_is_fat16`, `fat32_geometry_and_root_cluster`, `cluster_counts_are_limited_by_the_fat_type`, `ext16_checks_signature_before_fat_count`. Formatter boundaries are checked in `layout.rs` and `fatfs_format.rs`. The old root-cluster-zero test accepted either success or failure; the current test requires rejection of an invalid root cluster. |
| FAT FSInfo signatures and unknown free-count sentinel | Raw `boot.rs::fs_info_signatures` checks all three signatures; `fatfs_format.rs` checks emitted FSInfo and free-space fields. These checks use serialized structures instead of assertions on copied constants. |
| FAT short names, dot entries, labels, deleted/end markers, attributes and FAT12/16/32 entry values | Raw `short_name.rs`, `dirent.rs`, `slot.rs` and `entry.rs` unit tests; `fatfs_read.rs` checks label/dot filtering, names and contents through V3. Old tests of private V2 helper types are superseded. |
| FAT Unicode, spaces, extensions, LFN assembly and maximum-length names | Raw `lfn.rs` checks sequences, checksums and UTF-16 names; `fatfs_read.rs` and `fatfs_write.rs` exercise actual long/Unicode names. The old maximum-length test only checked `"a".repeat(255).len()`, and the escaped-0xE5 test discarded its result; neither established driver behavior. |
| FAT small/multi-cluster files, seek/read offsets, multiple files, root growth, create, rename, delete and truncate | `fatfs_read.rs::reads_at_offsets_across_clusters_and_fragments`, `fatfs_write.rs`, and `tests/suite/fat/{spec,limits}.rs`. `run_spec_matrix` runs FAT12/16/32 and compares every modeled path, kind, stable attribute and content with an independent raw-image oracle. Native formatter/checker tests remain separate. |
| FAT clean images and external fsck after mutations | `tests/suite/fat/native.rs` and `fat/peers.rs::external_tools_interoperate_with_hadris`, with the scenario/oracle suite covering mutation results. Kernel mounts remain explicitly enabled manual tests; they are not implied by ordinary round trips. |
| FAT old error-display strings | Intentionally not preserved as a V3 API requirement. The old test passed every supplied case even when its message did not match. V3 tests assert error kinds and details instead. |
| ISO valid/invalid descriptor identifiers, required primary/terminator, volume IDs, endian pairs and truncated images | `crates/optical/hadris-iso/tests/errors.rs::malformed_images_are_refused`, raw descriptor tests, `tests/suite/iso/volume_descriptors.rs`, and `iso/spec.rs::oracle_rejects_structural_corruption`. The independent oracle checks descriptors and redundant endian fields. |
| ISO directory records, special names, sector padding, path tables, version stripping and name limits | Raw directory/path-table tests, `name.rs` unit tests, crate `roundtrip.rs`/`malformed.rs`, and detached `iso/directory.rs`/`iso/spec.rs`. The old version-number test checked filename construction; it did not establish explicit-version lookup or duplicate-version selection. That behavior remains a known issue. |
| ISO recording/descriptor dates, Joliet escape sequences, Unicode names and maximum lengths | Raw date tests, `name.rs`, `roundtrip.rs`, and detached `volume_descriptors.rs`. This audit strengthens `test_unicode_filenames_joliet` to resolve and read all three Unicode files, rather than merely finding a supplementary descriptor. |
| ISO SUSP/RRIP signatures, PX/NM/SL records and deep trees | `rock_ridge.rs` unit tests, crate `malformed.rs`, and detached `iso/{rock_ridge,relocation}.rs`. The shallow Rock Ridge test now requires successful xorriso extraction and exact paths, kinds and bytes. Existing relocation checks cover deep trees and user relocation-directory collisions independently. |
| ISO boot record/catalog fields, platform/media indicators, section headers and checksum | Raw boot tests, crate `roundtrip.rs::boot_catalogs_read_back`, and detached `iso/boot.rs`. The catalog comparison now asserts boot mode/load fields and the boot payload referenced by each producer, in addition to validation checksums; it previously only printed most differences. Producer-specific block addresses are checked through their payloads, not required to be equal. |
| ISO deep-directory limits, large files, multi-extent flags and volume-space limits | Crate `errors.rs`, `multi_extent.rs`, and detached `iso/{multi_extent,relocation,spec}.rs`. Current sparse-file and raw-oracle tests exercise actual extent chains; old assertions on copied flag values are not retained as behavioral evidence. |
| ISO benchmark and stress test functions | Timing loops are replaced by `crates/optical/hadris-iso/benches/performance.rs` and the detached performance harness. Directory stress behavior is checked by `iso/directory.rs::test_multi_sector_directory` and crate performance tests with I/O bounds. Benchmarks remain explicit commands, not correctness gates. |
| UDF empty/truncated images, invalid VRS and missing anchor | `crates/optical/hadris-udf/tests/errors.rs::malformed_volumes_are_refused`, backup-anchor/reserve-sequence tests, and `roundtrip.rs::anchors_sequences_and_directories_follow_udf`. Unlike the old invalid-VRS fixture, current malformed tests also start from valid volumes. |
| UDF revisions, descriptor/tag identifiers, checksums, anchors, extents and partition fields | Raw descriptor tests, `errors.rs`, `read.rs`, and `roundtrip.rs::anchors_sequences_and_directories_follow_udf`. Tests of literal revision/identifier values are replaced by parsing, checksum rejection and serialized layout checks. |
| UDF ICB types/flags, allocation descriptors, FIDs, filename encoding and alignment | Raw file/name tests and `tests/read.rs` cover allocation forms and FIDs crossing extents; `roundtrip.rs` checks written directory structure and Unicode names. The old FID-padding test tested a local arithmetic closure, while the old OSTA test only asserted three local constants. |
| UDF timestamps, entity identifiers, charactersets and integrity descriptors | `time.rs`/raw descriptor unit tests, `roundtrip.rs::metadata_reads_back` and `identifiers_and_the_serial_follow_the_options`, plus `bridge.rs` layout checks. These exercise encoded records instead of copied sizes/bit values. |
| UDF sparing, metadata partition and extended-attribute constants | Intentional removal of constant-only tests. They did not parse or traverse these structures. Unsupported partition maps and writer features are explicitly rejected in `errors.rs`; full support is not claimed. |
| NTFS exhaustive byte mutations and 20,000 randomized mutations | Intentional removal of expensive generic mutation sweeps from normal CI. `fuzz/fuzz_targets/ntfs_read.rs` provides local mutation exploration; `tests/crafted.rs` and raw unit tests assert specific malformed-record, fixup, index, run-length and bounds failures. Fuzzing is not a CI gate and does not prove exhaustive mutation equivalence. |
| NTFS crafted mount/walk, huge MFT/run bounds, index allocation, corrupt fixups and async behavior retained by the first refactor | V3 `tests/crafted.rs` and `tests/read.rs` cover equivalent operations through `NtfsFs`, including every I/O mode and the shared contract. V2-only `read_to_vec` behavior is replaced by bounded reads and V3 error contracts. |

## Cargo discovery, feature gates and CI

The current detached manifest sets `autotests = false` and explicitly declares
`suite/main.rs`. Its `mod fat`, `mod exfat` and `mod iso` chains discover 64
suite tests; the harness library discovers seven more tests. The historical
ISO test names were checked against Cargo's `--list` output, not just source
searches. The current affected crates also discover their unit and integration
tests with `--all-features`.

`rrip_writer_metadata.rs` was included in ISO's library harness immediately
after both historical commits despite `autotests = false`. V3 subsequently
replaced that harness with the current integration tests, including
`roundtrip.rs` metadata cases; the historical filename is no longer the
current evidence. Sync/async cases are feature-gated explicitly, and the CI
MSRV feature tiers separately compile no-std configurations.

Rust CI runs the detached FAT slice with `fat:: -- --skip exfat::`, the exFAT
slice separately, and the ISO slice with `iso::`. A substring filter `fat::`
alone would include exFAT. Optical CI supplies xorriso and bsdtar and sets
`HADRIS_REQUIRE_EXTERNAL_TOOLS=1`. Rock Ridge now uses the same mandatory-tool
helper as the other interoperability tests. All xorriso inspection calls
require successful exit status, including session and multi-extent checks.

Oracle tests and interoperability assertions are correctness gates, including
the sparse multi-gigabyte extent check. Peer accuracy reports, privileged
native mounts and QEMU boot tests remain manual. A peer scorecard is not
evidence that Hadris passed a correctness gate. Local tests that return early
for a missing tool are not external-tool passes.

## Remaining limits

This audit does not establish whole-workspace correctness or claim that every
historical private API still exists in V3. Missing tests of unsupported UDF
features, exact V2 error strings and generic exhaustive mutation sweeps are
intentional omissions, not silent coverage claims. Mixed-version ISO lookup,
partial Rock Ridge timestamp fallback and relocation-directory visibility
remain runtime concerns tracked in `KNOWN_ISSUES.md`.

The current semantic snapshots compare ordinary directories and file bytes;
they do not qualify every symlink, hard-link identity, permission or native
mount behavior. Those need their dedicated tests. Linux/Windows native
qualification and QEMU execution cannot be inferred from this macOS run.

## Reproduction

```bash
cargo test --manifest-path tests/Cargo.toml -- --list
cargo test -p hadris-fat -p hadris-fat-raw -p hadris-iso -p hadris-udf \
  -p hadris-ntfs --all-features -- --list
nix develop -c env HADRIS_REQUIRE_EXTERNAL_TOOLS=1 \
  cargo test --manifest-path tests/Cargo.toml iso::
```

The ISO check on macOS passes with external tools required. Ignored tests
still require explicit execution; in particular, this command does not run
the QEMU or privileged native-mount reports.
