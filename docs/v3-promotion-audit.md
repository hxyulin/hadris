# V3 promotion audit against V2

Audit date: 2026-10-06. V2 baseline: `main` at `d94747a8` (2.5.0 source,
including the release-workflow rustfmt fix after the `v2.5.0` tag). V3 baseline:
`next` at `c49966e7`, with audit follow-ups on `audit/v3-promotion`.

## Recommendation

Promote V3 to `main` as a **release-candidate development line**, after the audit
preparation changes pass CI. Keep package versions at `3.0.0-rc.1`, with
`hadris-fat-raw` at `0.1.0`. Promotion does not publish crates, create release
tags, or declare 3.0 stable. The current 2.5.0 release remains available and
its source/history is preserved. If ongoing V2 maintenance is desired, branch
from the existing V2 main tip before switching the main development line.

Do not describe V3 as an API-compatible replacement for V2, universally faster,
or a fully crash-safe writable filesystem. The deliberate major-version API
change is documented; limitations and the measured performance tradeoff below
remain. A stable 3.0 release should resolve or explicitly revisit the remaining
3.0 action-catalog requirements, especially exFAT bad-cluster allocation and
ISO version selection. This audit is a focused promotion check, not an exhaustive
new proof of every parser or every action-catalog requirement.

## Confirmed promotion preparation findings

| Finding | Evidence | Resolution/status |
|---|---|---|
| Release/default-feature tests fail to compile even though all-feature CI is green | `cargo +1.97.1 test --workspace --locked` fails first in CPIO `async_writer`; subsequent builds expose ISO, UDF, NTFS and partition async references without their crate's `async` feature. Standalone default FAT tests fail through an unconditional async path-helper module. The release workflow uses the default suite. | Gate async-only targets/cases/helpers and imports; preserve sync coverage in mixed files. Add a warnings-as-errors default-feature CI job. The corrected default workspace suite passes 1,022 tests/doctests with 11 ignored. |
| Stability/migration guidance names 2.4 as the current stable version or assumes V2 must remain on `main` | Current GitHub release is 2.5.0; `website/docs/stability.md` still names 2.4. Migration introduction describes only 2.4, although an APFS 2.5 appendix exists. | Branch-neutral stability wording; link the V2 tag; describe the 2.4/2.5 migration and newer cache, tracing and encryption configuration. Explicit async close/sync guidance added. |
| The known-issues list incorrectly claims FAT/exFAT chmod/chown are silent no-ops | Both hosted `setattr` implementations refuse owners and unsupported permission modes; regression tests cover mapping supported permissions to the read-only attribute. | Remove the stale issue. Root no-op attribute handling remains separately documented. Clarify that directory performance problems concern unsupported/bounded-cache fallback workloads, not every optimized path. |
| Publication cannot be dispatched with the current notes | `scripts/release-plan.py --notes 3.0.0-rc.1 all` rejects the undated release-candidate section for every crate. | Intentional publication gate. Date and review the release notes when actually publishing; do not date them merely to merge branches. |

## V2 capability comparison

| Area | V2 baseline | V3 outcome and migration implications |
|---|---|---|
| Storage and I/O | Byte-stream APIs, format-specific error enums and category facades | Shared whole-block devices and `FileSystem` trait; backend errors retained. Intentional source incompatibility. Stream adapters and returned devices on failed mounts cover migration use cases. |
| FAT12/16/32 | Read/write, LFN, formatting, checking, sector-cache feature | Coverage retained, with hosted and embedded drivers, bounded optional caches, batching, interrupted-operation recovery, and more independent-oracle/peer coverage. Writes still require explicit durability handling. |
| exFAT | Synchronous preview; fragmented bitmap unsupported in the V2 loader | Hosted/embedded drivers and sync/async support; fragmented system files, vendor entry sets and guarded operation recovery. API becomes stable, but malformed-bitmap/bad-cluster handling and crash limits remain in known issues. |
| ISO | Primary/Joliet/Rock Ridge/enhanced trees, multi-extent files, boot catalogs, hybrid images and modification | Readers/builders/session support retained and audited, with optional caching and fewer repeated reads. Remaining explicit-version, relocation-container and TF timestamp behavior is documented. |
| UDF and bridges | Type-1 mastered images; V2 writer can label output through 2.60 without implementing metadata partitions | Type-1 reader/writer and ISO/UDF bridges retained. V3 deliberately refuses writer revisions 2.50/2.60. This narrows the advertised revision labels; it does not remove an implemented metadata-partition writer. CLI help still advertises those rejected values and should be corrected. |
| CPIO | newc/CRC archives | Streaming newc/CRC/odc read/write, binary read, hard-link handling and native extraction checks. Tree building needs alloc; streaming reading does not. |
| Partitions | MBR/GPT, hybrid MBR, facades | Shared-device scanning/opening, EBRs, backup GPT recovery and disk-layout building retained. GUID generation and CRC behavior migrate to explicit APIs. |
| NTFS | Experimental read-only native reader | Read-only shared driver, streams/attribute lists and geometry/run bounds. Still a preview; compressed/encrypted streams, recovery log replay and full reparse-point semantics remain unsupported. |
| APFS | Experimental native container/volume reader added in 2.5 | Shared read-only driver, explicit volume selection, live object-map resolution and qualified software password unlocking. Still a preview; Apple-silicon hardware FileVault, compressed files and general repair/recovery are not qualified. |
| Tools | Separate FAT/ISO/UDF/CPIO/CD binaries and aliases, plus APFS inspector | Unified `hadris` commands; old binaries/flags/defaults intentionally change. Legacy standalone APFS inspector remains. Safer extraction and atomic regular-file image replacement are useful semantic differences. |

No format present in the V2 workspace disappears in V3. That does not imply
one-to-one API or command compatibility: six facade/utility libraries and five
CLI packages are removed, with successors/mappings in the migration guide.
Published-package count changes from 24 to 16. MSRV remains Rust 1.88.0.
Production Rust source under `crates/**/src/*.rs` changes from 199 files/66,401
physical lines to 228 files/72,172 lines (+8.7% lines); that includes the new
shared filesystem layer, raw FAT layer and APFS crypto implementation. These
counts include comments and tests embedded in source modules, exclude integration
tests/docs/benchmarks, and do not measure complexity or runtime quality.

## Direct V2/V3 FAT extraction measurements

Same M3 Pro/macOS host, Rust 1.97.1 release builds, identical 64 MiB FAT32 image
with 1,000 seven-byte files and a 128 KiB payload. Each case has 21 fresh-process
samples after one discarded warmup, with deterministically shuffled order each
round. Wall time includes startup/exit and excludes validation/cleanup. Peak RSS
comes from `/usr/bin/time -l`; image calls/bytes come from separate untimed Darwin
interposer runs matched by image inode. No instrumentation is present in timing
or RSS samples. Every output name and byte is validated, with zero I/O failures.

| Extractor | Median ms | Peak RSS MiB | Image reads | Requested bytes |
|---|---:|---:|---:|---:|
| V2 `hadris-fat extract` | 59.868 | 6.109 | 2,641 | 204,684 |
| V3 default | 74.254 | 4.141 | 1,134 | 710,656 |
| V3 `--no-cache` | 126.577 | 4.125 | 63,872 | 32,832,512 |
| V3 `--read-ahead-blocks 128` | 72.542 | 4.203 | 90 | 788,992 |

V3 default is about 24.0% slower for this CLI workload, while using 32.2% less
peak RSS and 57.1% fewer image calls. Read-ahead is about 21.2% slower than V2
while cutting image calls 96.6%. V3 uses whole-block I/O and therefore requests
more bytes. Disabling its listing hint makes the repeated directory-scan cost
substantial.

Output semantics differ: V2 streams files to `File::create` without restoring
metadata, whereas V3 builds a lazy tree, safely refuses existing files/symlinks,
and applies permissions and timestamps. Contents are equivalent; metadata and
overwrite semantics are not. These numbers show a user-visible workflow cost,
not an isolated driver regression or a universal ranking. This audit does not
benchmark every format against V2. Follow-up profiling should separate host
metadata/security work, tree construction, lookup and transport before deciding
what to optimize or whether an explicit content-only extraction mode is useful.

[Raw samples](benchmarks/v2-v3-fat-extraction.csv),
[image I/O](benchmarks/v2-v3-fat-image-io.csv), and
[binary/fixture metadata](benchmarks/v2-v3-fat-extraction-metadata.json).
Reproduce on macOS with `tests/peers/compare-fat-generations.py`, passing
`--v2`, `--v3`, `--image`, `--counter` (the Darwin interposer dylib), and
`--output`. Use the same prepared 1,000-file fixture as the peer harness.
Untracked local artifacts
include binaries, SHA-256 metadata, logs, counter output and the runner under
`output/v2-v3/` in the audit worktree.

## Remaining correctness and qualification limits

- FAT16/32 reports the dirty flag but does not set/clear it across writes/sync.
  V2 also exposes read-only status-flag reading without a write lifecycle;
  this is not established as a new V3 regression, but VOL-MOUNT-04 remains unmet.
- exFAT allocation chooses clear bitmap bits and writes FAT_END without first
  refusing an existing bad-cluster marker. V2 bitmap selection has the same
  omission, but it was a preview. Check/repair and mount policy for inconsistent
  allocation metadata should be addressed before claiming CHECK-BAD-01 complete.
- Interrupted-write/cancellation recovery is stronger than V2's synchronous
  exFAT preview, but is held in memory. Real power loss is not a rollback journal;
  spanning exFAT entry sets and TexFAT mirror state have documented limits.
- ISO explicit `;N` lookup, duplicate-version listings, relocation-directory
  visibility and partial Rock Ridge TF timestamps remain documented. These
  items were inspected at code/known-issue level, not newly reproduced in this audit.
- Unstorable root attributes can refuse equivalent values. FAT/exFAT tree writing
  can discover insufficient space after formatting; the API docs must not imply
  a non-destructive size plan where none exists.
- UDF CLI 2.50/2.60 acceptance disagrees with the writer planner's refusal.
  Reader/writer metadata, sparable and virtual partitions remain outside the
  implemented Type-1 scope.
- APFS/NTFS native preview limitations remain; Linux FUSE deployment and internal
  Apple-silicon FileVault are separate qualification work, not promotion evidence.

See [KNOWN_ISSUES](../KNOWN_ISSUES.md) and the [3.0 action catalog](v3/actions.md).
The action catalog is a requirements list, not proof that each marked item has
been implemented. Its individual unmet 3.0 requirements need a disposition
before calling the release stable.

## Validation and promotion sequence

1. Merge the small audit-preparation PR into `next` after CI passes. It fixes test
   gating/CI and documentation, not on-disk algorithms or the public API.
2. Review the `next` to `main` promotion as a major-version transition. Preserve
   V2 tags/history; choose a maintenance branch if continued V2 fixes are planned.
3. Run promotion CI against the `main` base, including semver checks, full API
   snapshots, subset/parity, feature tiers, interoperability, embedded builds,
   Miri and platform tests. Existing #251/#252 CI provides baseline evidence,
   but does not replace the promotion-base check.
4. Merge V3 as the candidate development line. Publish only through the release
   workflow after selecting versions and dating/reviewing notes. Stable 3.0 is a
   separate decision with the remaining requirements explicitly resolved/deferred.

Local audit evidence: warnings-as-errors default workspace tests (1,022 passed,
11 ignored); all 16 publishable crates packaged and verified with
`cargo +stable publish --locked --dry-run --allow-dirty` (no uploads); direct
V2/V3 extraction validation. Async-enabled regression tests across CPIO/ISO/UDF/NTFS/partitions pass
310 tests; standalone default FAT passes 185 tests. All 51 affected MSRV feature
tiers, warnings-as-errors workspace check, both formatter versions and the full
CI Clippy command pass.
