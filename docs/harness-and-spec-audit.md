# V3 harness and specification audit

Measured on 2026-10-06 at `73f5e615`, using Rust 1.97.1 release builds on an
Apple M3 Pro with 18 GiB RAM, macOS 27.0.1. The worktree was clean during the
measurement matrix. The harness changes start at `81c1dbab`; the baseline
library implementation is main's `71fa18b2`.

The strongest measured opportunity is directory metadata traversal, especially
UDF and exFAT repeated lookup. File payload reads are already batched. Before
optimizing those paths, tighten UDF identifier validation and reconcile the
V3 crash and failure guarantees with their tests. The cache results support
keeping memory budgets configurable: a directory index helps repeated FAT
lookup substantially, but increases first-lookup cost and retained memory.

## Harness changes and measurement boundaries

The shared `FileSystem` benchmark now covers eleven operations on FAT12/16/32,
exFAT, ISO and UDF: mount, listing, cold last-file lookup, missing lookup, warm
last-file lookup, batch lookup, stat, 4 KiB and 64 KiB reads, scattered reads,
and full-file reads. Each operation checks its result outside the timing window.
Batch lookup visits every small file; it does not include the separate payload.

Memory-backed runs measure CPU and requested backend I/O. File-backed runs
launch a fresh worker per operation and sample through the platform time
utility. Fixture generation and independent raw-oracle validation happen in
the parent, outside worker RSS. Each worker performs one discarded warm-up and
one measured operation on separate fresh mounts. Driver-cold lookup means a
fresh mount; the warm lookup primes the driver before measurement. Neither
means a cold OS page cache.

The CSV adds backend, cache policy, directory size, resolved metadata-cache
capacity and optional peak RSS. Compiler provenance is captured at build time;
revision, dirty state, configuration and measurement boundaries accompany each
run. Metadata cache capacities can be varied independently for FAT and ISO.
CI exercises both backends as correctness smoke checks without timing gates.

Peak RSS is the whole worker's high-water mark, including startup, mounting,
setup, warm-up, verification and allocator retention. It is not the operation's
incremental allocation or a measurement of embedded driver state. Counters sit
below driver caches and count requested backend calls/bytes; OS cache misses,
physical reads and syscall counts are not measured. Timings exclude worker
startup, mounting for non-mount operations, buffer setup and result verification.

There are **4,290 retained samples**: 21 memory samples or five file-backed
samples per operation/configuration. All measured windows completed with zero
write calls and zero I/O failures. Raw samples retain timing and RSS ranges;
five RSS samples are insufficient to characterize rare tails. Background
activity and scheduler variation remain visible, especially in the FAT cache
comparison, so the deterministic read counts are the stronger evidence.

- [Raw samples](benchmarks/harness-v3-audit-samples.csv)
- [Medians and ranges](benchmarks/harness-v3-audit-summary.csv)
- [Per-run provenance](benchmarks/harness-v3-audit-metadata.txt)
- [Harness configuration and commands](../tests/README.md#v3-performance-harness)

## Directory scaling

Images contain either 32 or 1,000 short-name, seven-byte files plus a patterned
128 KiB payload. These are flat, contiguous, generated images; long names,
nested directories, fragmentation, mutation and external images need separate
qualification. FAT, exFAT and ISO fixtures pass independent raw-image oracles.
UDF checks expected names and bytes but has no independent oracle in this
detached package.

File-backed batch lookup, with the runner's default policies:

| Format | 32 files: reads / median | 1,000 files: reads / median | 1,000-file median worker peak RSS |
|---|---:|---:|---:|
| FAT32 | 4 / 0.025 ms | 35,640 / 37.89 ms | 2,672 KiB |
| exFAT | 116 / 0.097 ms | 105,751 / 79.69 ms | 2,608 KiB |
| ISO | 64 / 0.069 ms | 13,431 / 29.39 ms | 2,512 KiB |
| UDF | 1,193 / 0.905 ms | 1,050,615 / 775.75 ms | 2,560 KiB |

Increasing the number of lookups by 31.25 times increases backend reads by
about 880 times for UDF and 912 times for exFAT. Their implementation restarts
name scanning for each lookup. This is evidence of quadratic batch work on
these fixtures, consistent with code inspection; it is not a fitted complexity
bound for every directory layout.

UDF's 1,000-file lookup batch requested **514.46 MiB** of backend data for a
small directory. Listing alone made 4,100 reads and requested 4.94 MiB, with a
4.51 ms median. `fid_at` reads fixed and variable identifier fields separately;
`readdir` also resolves entry metadata. A bounded sector window should remove
repeated physical requests before an optional name index addresses repeated
decoding and scans. Preserve validation across allocation-descriptor boundaries
and test both tiny caches and directories larger than the cache.

exFAT's batch requested 51.64 MiB. A bounded metadata cache and an optional
directory index are candidates. Its writable driver needs invalidation on
create, rename and unlink; importing a read-only index without that contract
would be a correctness regression.

ISO listing made 2,027 reads without its optional cache, versus 24 with the
eight-sector/1,001-record policy. Median time fell from 1.487 ms to 0.295 ms.
The mounted reader reconstructs a local directory block window for each
`readdir` call, so repeated sector reads are a concrete target for the uncached
path. Persistent bounded traversal state may benefit sequential enumeration
without requiring every caller to enable a larger cache.

## Cache cost and API implications

These are format-specific policies, not equivalent memory budgets. FAT's
`index` policy enables its directory-prefix index; ISO's policy enables parsed
records and metadata sectors and **does not build a name index**. ISO's default
policy here is uncached. FAT defaults to eight device metadata blocks and
32 chain positions, with no directory index.

File-backed batch lookup of 1,000 files:

| Format / configuration | Reads | Median time | Median worker peak RSS |
|---|---:|---:|---:|
| FAT32, caches disabled | 62,613 | 54.75 ms | 2,640 KiB |
| FAT32, default | 35,640 | 37.89 ms | 2,672 KiB |
| FAT32, 1,001-entry directory index | 72 | 0.829 ms | 2,768 KiB |
| ISO, uncached | 13,431 | 29.39 ms | 2,512 KiB |
| ISO, 8 sectors / 1,001 records | 10,920 | 28.58 ms | 2,592 KiB |
| ISO, 32 sectors / 32 records | 24 | 21.90 ms | 2,640 KiB |
| ISO, 32 sectors / 1,001 records | 24 | 22.15 ms | 2,624 KiB |

The FAT index reduces batch reads about 495 times and median file-backed time
about 46 times. Worker peak RSS increases by 96 KiB versus default in this run;
this difference is an observed process cost, not an exact index allocation.
The initial last-file lookup rises from 68.5 to 125.1 microseconds, while a
subsequent warm lookup falls from 78.2 to 1.0 microseconds and needs zero reads.
Retain opt-in indexing for workloads that amortize construction. Capacity,
fallback behavior and invalidation belong in the public cache contract.

ISO needs 24 metadata sectors for this directory traversal. Eight sectors
thrash during repeated scans; 32 retain the working set. Raising capacity cuts
backend reads about 560 times, but batch time improves only about 1.34 times.
Memory-backed batch time remains around 21–22 ms. Source inspection confirms
lookup still scans and decodes directory records; the remaining cost calls for
a bounded name index or reusable decoded traversal state, not larger sector
caches alone. Increasing parsed-record capacity to 1,001 has no meaningful
benefit here. The RSS results do not establish a monotonic per-record cost.

Mounting remains small and independent of directory count for these fixtures.
At 1,000 files, default mounts made three FAT32 reads, 27 exFAT reads, three ISO
reads and 16 UDF reads, with approximately 2.3–2.4 MiB whole-worker peak RSS.
An eager full-directory scan at every mount would move cost onto callers that
never inspect that directory. Prefer lazy, bounded caches with explicit budgets.

Listing peak RSS rises to roughly 3.5–3.6 MiB at 1,000 files. The benchmark
retains the returned entries for verification; `DirEntry` is 920 bytes on this
target, so its preallocated 1,001-entry vector alone reserves 899 KiB. This
explains a substantial part of the increase and is not evidence of a driver
leak. A future allocation-specific harness should distinguish caller buffers,
live driver allocations and retained allocator pages.

Full-payload reads already use three backend calls on FAT32 and two on exFAT,
uncached ISO and UDF. They request approximately the payload size plus a small
metadata overhead. Fragmented reads and read-ahead workloads remain useful,
but these results prioritize directory work over contiguous payload copying.

## Specification and correctness findings

The clause catalog contains **88 selected requirements** across five crates:
73 labelled verified, 11 partial and four unimplemented after refreshing
evidence. These counts describe the selected catalog, not a whole-format
conformance percentage. APFS and partition formats have no atomic catalog here.
NTFS's catalog explicitly covers a partial profile from Microsoft's MFT
overview rather than a complete NTFS specification.

Eighteen implementation mappings referenced obsolete identifiers or wrong
files. They now point to current code. FAT32 backup-boot recovery was wrongly
listed as missing: `MountOptions::backup_boot()` mounts the validated sector-six
backup read-only. It is an explicit option, not automatic primary-failure
fallback. Its integration test is now included in the recovery-copy evidence.

The checker now rejects mappings whose qualified identifiers are absent from
the named source file. This is a mention check, not Rust symbol resolution or
semantic verification. All cached sources with recorded digests passed the
digest check. Macro-generated tests and feature-gated discovery still require
separate checks.

| Priority | Finding | Evidence and next action |
|---|---|---|
| High | Crash guarantee exceeds implemented recovery | `NF-CRASH-01` promises no cross-links or corrupt entry sets. FAT power-cut tests explicitly allow cross-links; known exFAT limitations include damaged entry sets and recovery state lost on remount. Separate same-mount cancellation/I/O recovery from recovery after power loss and remount. Strengthening the latter requires write-ordering or persistent recovery work. |
| High | Failure atomicity needs an explicit scope | `NF-ATOMIC-01` says a failed operation leaves disk and memory unchanged. `overwrite_error_can_leave_partial_data` documents partial writes, and FAT/exFAT `write` can format before detecting insufficient capacity. Distinguish contract rejection, preflight capacity failure and device failure; implement planning under [#266](https://github.com/hxyulin/hadris/issues/266). |
| Medium | UDF identifier validation is incomplete | A checksum-preserving probe reproduced acceptance of an invalid file version, reserved characteristic bits, nonzero padding, wrong tag location and reserved tag byte. Add independent malformed-image regressions and validate these fields without weakening cross-extent support. |
| Medium | ISO namespace behavior remains incomplete | Explicit versions and duplicate version listings are tracked in [#264](https://github.com/hxyulin/hadris/issues/264); the exposed relocation directory is tracked in [#265](https://github.com/hxyulin/hadris/issues/265). The short-name performance fixture does not cover either case. |
| Medium | Performance guarantees need cache conditions | `NF-PERF-04` promises path cost independent of directory size, which uncached scan-based readers cannot provide. `NF-PERF-03` promises O(1) block-cache access, while the storage cache uses `BTreeMap`/`BTreeSet`. Specify cold/warm behavior, capacity and fallback; tree indexing is O(log capacity), not O(1). |
| Low | The API roadmap is stale in places | `NF-NOALLOC-02` still schedules allocator-free exFAT writing for 3.x, although the embedded writer and tests exist. The shared local-async tier remains a real design question under [#267](https://github.com/hxyulin/hadris/issues/267). |

The UDF checks correspond to ECMA-167 Part 4 descriptor tags and identifier
fields, particularly 4/7.2 and 4/14.4.2, .3 and .9. The tested mutations retain
valid CRCs and tag checksums, so checksum detection does not explain their
acceptance. The catalog's partial FID requirement now records the additional
unchecked fields. The probe diagnoses input validation, not memory unsafety or
an exploit. Its source and observed output are archived in the
[diagnostic evidence](benchmarks/harness-v3-audit-fid-probe.txt).
The authoritative rules are in the
[ECMA-167 third edition](https://ecma-international.org/wp-content/uploads/ECMA-167_3rd_edition_june_1997.pdf).

Relevant repository evidence:

- [V3 requirements](v3/actions.md#3-non-functional-constraints) and
  [known limitations](../KNOWN_ISSUES.md).
- [FAT power-cut tests](../crates/block/hadris-fat/tests/fatfs_check.rs) and
  [embedded power-cut tests](../crates/block/hadris-fat/tests/embedded.rs).
- [UDF identifier reader](../crates/optical/hadris-udf/src/read.rs),
  [ISO lookup and enumeration](../crates/optical/hadris-iso/src/image.rs), and
  [storage cache indexing](../crates/core/hadris-storage/src/cache.rs).
- [Clause catalog and source provenance](../spec/README.md).

## Coverage gaps and recommended sequence

1. Fix UDF FID validation in a separate correctness change, and make the crash
   and rejection requirements precise. Existing ISO namespace and formatting
   preflight issues remain concrete correctness work.
2. Add a bounded UDF directory-sector window, measuring calls, CPU and RSS on
   this fixture plus fragmented identifiers. Then evaluate optional name
   indexing for repeated lookup. Do the same for exFAT with mutation and
   cancellation invalidation tests.
3. Improve ISO's uncached sequential listing; separately evaluate a name index
   for repeated lookups. Preserve caller-selected cache budgets and fallbacks.
4. Extend the harness to long/nested names, fragmented extents, write operations,
   cancellation and complete extraction. Reuse the existing peer runner for
   end-to-end comparisons; its process/host-I/O boundaries differ from these
   isolated `FileSystem` timings. This matrix contains no new peer speed claim.
5. Add APFS and NTFS fixture adapters before prioritizing their micro-optimizations.
   APFS's mounted driver builds an index proportional to volume records, making
   mount RSS on a real macOS volume especially useful. NTFS scans its directory
   index nodes rather than descending by key, a performance candidate requiring
   large native fixtures. CPIO needs stream-specific measurements; its existing
   [benchmark](cpio-performance.md) already distinguishes streaming from owned
   tree loading. None of those formats was timed in this matrix.

The harness remains synchronous; async throughput/cancellation, allocator peaks,
embedded stack/RAM, compressed/encrypted files and physical cold-storage latency
are separate evidence requirements. UDF's independent native qualification
remains in crate interoperability tests; Linux CI provides `udfinfo`/`mkudffs`,
which are unavailable in the local macOS flake.

## Reproduction and validation

Use the commands below with distinct report directories. The archived metadata
names every configuration. Run all six formats at 32 files, then filter each of
FAT32/exFAT/ISO/UDF at 1,000 files. Repeat FAT32 with `none` and `index`, ISO with
`index`, and ISO `default`/`index` with 32 metadata sectors. Each combination
runs all eleven workloads. Memory runs use 21 samples; file runs use five.

```bash
HADRIS_TESTS_PERF_BACKEND=file HADRIS_TESTS_PERF_SAMPLES=5 \
  HADRIS_TESTS_PERF_FILTER=udf HADRIS_TESTS_PERF_FILES=1000 \
  HADRIS_TESTS_REPORT_DIR=output/udf-audit \
  cargo +1.97.1 bench --manifest-path tests/Cargo.toml --bench performance

HADRIS_TESTS_PERF_BACKEND=file HADRIS_TESTS_PERF_SAMPLES=5 \
  HADRIS_TESTS_PERF_FILTER=iso HADRIS_TESTS_PERF_FILES=1000 \
  HADRIS_TESTS_PERF_CACHE=default HADRIS_TESTS_PERF_BLOCKS=32 \
  HADRIS_TESTS_REPORT_DIR=output/iso-cache-audit \
  cargo +1.97.1 bench --manifest-path tests/Cargo.toml --bench performance
```

Validation includes workspace formatting and warning-free checking, detached
harness formatting/Clippy and Rust 1.88 all-target checking, nine harness unit
tests, both benchmark backends, and the full hosted conformance suite through
Nix with external tools required: 54 passed, ten manual tests ignored. Compliance
checker self-tests, cached-source digests, spec annotations, documentation links
and workflow syntax also passed. The diagnostic UDF probe was run separately
and removed from the test source after recording its result. The cited FAT32
backup recovery test and UDF extraction through Nix-provided 7-Zip passed too.
