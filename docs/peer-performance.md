# Peer performance and reference coverage

The shared V3 runner measures driver operations on memory. The peer runner
measures host workflows on regular image files, using the same source tree for
writers and exactly the same input image for readers. These answer different
questions and their timings must not be pooled.

## Running comparisons

```bash
cargo bench --manifest-path tests/Cargo.toml --bench peers
nix develop -c env HADRIS_REQUIRE_EXTERNAL_TOOLS=1 \
  cargo bench --manifest-path tests/Cargo.toml --bench peers

HADRIS_TESTS_PERF_FILTER=fat HADRIS_TESTS_PERF_SAMPLES=1 \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
HADRIS_TESTS_PERF_FILTER=iso HADRIS_TESTS_PERF_FILES=1000 \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
HADRIS_TESTS_PERF_FILTER=fat32 HADRIS_TESTS_PERF_FILES=1000 \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
```

The default fixture has 32 small files and a 128 KiB patterned payload. Both
runners share this fixture. `HADRIS_TESTS_PERF_FILES` changes the small-file
count. Large directories use FAT32, exFAT, ISO or UDF filters with
`performance`, and FAT32 or ISO with `peers`. The FAT12/16 benchmark fixtures
deliberately cap the count at 128 to stay
within their fixed roots. The FAT size/type is pinned to 2 MiB/FAT12,
16 MiB/FAT16, or 64 MiB/FAT32. Formatters choose their own remaining geometry.

`performance/peers.csv` under the harness report root contains 21 samples per
workload by default, after one discarded warm-up. The format filter and report
root have the same meanings as in the V3 runner. Missing external tools skip
locally and fail under `HADRIS_REQUIRE_EXTERNAL_TOOLS=1`. mkisofs is preferred;
genisoimage is used and labelled as such if it is the installed candidate.

| Format | Workload | Implementations |
|---|---|---|
| FAT12/16/32 | Format an empty volume | Hadris, rust-fatfs with and without stream buffering, dosfstools `mkfs.fat` |
| FAT12/16/32 | Create a populated image from host files | Hadris, both rust-fatfs variants, `mkfs.fat` plus one batched `mcopy` |
| FAT12/16/32 | List the root and extract every file | Hadris, both rust-fatfs variants, `mdir`/`mcopy` |
| ISO | Create a Level 1 image from host files | Hadris, xorriso/libisofs, mkisofs or genisoimage |
| ISO | List the root and extract every file | Hadris, xorriso/libisofs, bsdtar/libarchive |

Every populated output must have the expected names and every file byte.
FAT outputs pass the independent raw FAT oracle and ISO outputs pass the raw
ECMA-119 oracle. Extraction results are checked through a host-tree snapshot.
Read workflows share a validated Hadris-generated input rather than letting
writers choose a favorable layout for their own readers. This is a performance
fixture, not proof of a peer's complete format support.

The `host-workflow` window includes file opening, source discovery for image
creation, mounting, allocation, host source/output I/O, and teardown. External
commands include startup, image parsing, stdout/stderr capture, and output
parsing. mtools configuration is prepared before timing. Verification and
cleanup are excluded. Output images and extraction directories are fresh for
each sample. Formatting and image creation include the same final `sync_data`
barrier on the output image for every implementation; writer-internal flushes
remain part of its workload too. Extraction has no added durability barrier.
There is no OS-cache eviction, and containing directories are not synced;
results describe repeated host workflows, not cold-disk speed or crash recovery.

`image_bytes` records the input/output image size: ISO padding and metadata
choices differ between writers. The I/O columns count requested calls and bytes
at Hadris's block-device boundary or rust-fatfs's stream boundary. Buffered
rust-fatfs places the counter below `fscommon::BufStream`; the other variant
has no stream buffer. Stream seeks are counted too. Calls at these different
API granularities are not equivalent physical disk transactions. Source and
extracted-output I/O and the common final image barrier are outside these counters. CLI counters are empty because
the harness does not instrument those processes; an empty field is not zero.

Record tool versions, Rust version, machine, revision and fixture size with
saved results. Do not run competing benchmarks simultaneously. Compare medians
and sample spread; avoid interpreting a short command's startup cost as its
filesystem algorithm cost. This runner produces timings and API I/O metrics,
not CPU stack profiles. Sustained API loops and OS-specific process profiling
are separate follow-ups.

## Choosing more complete references

Completeness is specific to an operation and feature set. A useful comparison
set includes independent libraries, specialized tools and native drivers,
while the specification oracles remain the ground truth.

| Reference | Coverage and role | Harness status |
|---|---|---|
| ChaN FatFs in C | FAT/exFAT, configurable long names, Unicode, sector sizes and embedded operation; a close peer for Hadris's embedded tier | Implemented optional native helper for basic FAT workflows; broader exFAT/Unicode qualification follows |
| Native Linux/macOS drivers | Native filesystem behavior and caching; kernel comparisons need their own mount/cache boundaries | Existing manual native qualification infrastructure; performance follow-up |
| rust-fatfs | FAT12/16/32 and long names; useful independent Rust API peer, with upstream-recommended buffering reported separately | Implemented; does not stand in for exFAT qualification |
| mtools and dosfstools | DOS file manipulation plus a separate formatter/checker; specialized workflows rather than a kernel filesystem interface | Implemented FAT workflow comparison |
| xorriso/libisofs | ISO authoring, Rock Ridge and session manipulation, plus extraction | Implemented basic creation/list/extraction; extension workloads follow separately |
| mkisofs and libarchive | Independent ISO authoring and archive-style reading; libarchive documents ISO extension support with limitations | Implemented mkisofs creation and bsdtar reads |
| 7-Zip | Read/extract peer across FAT, ISO, UDF, NTFS and APFS; format-specific limitations still need qualification | Existing UDF interoperability use; shared performance follow-up |
| exfatprogs and udftools | Format-specific creation/checking tools, distinct from filesystem drivers | Existing qualification; shared formatting/checking performance follow-up |

Primary references:

- [ChaN FatFs features](https://elm-chan.org/fsw/ff/) and
  [configuration](https://elm-chan.org/fsw/ff/doc/config.html); use the
  [published patches](https://elm-chan.org/fsw/ff/patches.html) when pinning R0.16.
- [rust-fatfs features and buffering recommendation](https://github.com/rafalh/rust-fatfs).
- [GNU mtools manual](https://www.gnu.org/s/mtools/manual/mtools.html).
- [GNU xorriso manual](https://www.gnu.org/software/xorriso/man_1_xorriso.html)
  and [cdrtools programs](https://cdrtools.sourceforge.net/private/cdrecord.html).
- [libarchive formats](https://github.com/libarchive/libarchive/wiki/LibarchiveFormats)
  and [ISO support](https://github.com/libarchive/libarchive/wiki/FormatISO9660).
- [7-Zip supported formats](https://sevenzip.sourceforge.io/index.html).
- [Linux VFAT](https://www.kernel.org/doc/html/latest/filesystems/vfat.html),
  [ISO](https://www.kernel.org/doc/html/latest/filesystems/isofs.html), and
  [UDF](https://www.kernel.org/doc/html/latest/filesystems/udf.html).
- [exfatprogs](https://github.com/exfatprogs/exfatprogs).

The next useful additions are sustained ChaN API loops, qualified native
kernel performance runs, and broader fixtures (fragmentation, deep trees, long
names, Unicode and ISO extensions). A larger peer list alone does not establish
coverage of those cases.

## ChaN FatFs helper

Build without global installation, then opt into the peer:

```sh
python3 scripts/build-fatfs-peer.py
HADRIS_TESTS_CHAN_FATFS="$PWD/tests/target/chan-fatfs/chan-fatfs" \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
```

The builder verifies SHA-256 hashes for upstream R0.16 and both published
patches before compiling with the host C compiler at `-O2`. Downloaded sources
and the executable stay in `tests/target/chan-fatfs`; upstream code is not
vendored. Configuration enables dynamic long-name buffers, UTF-8 API names,
CP437 short names, exFAT, 64-bit LBAs, formatting and labels. Sectors are fixed
at 512 bytes, with one volume and no RTC or reentrancy. FAT12/16/32 creation
uses two FATs, 1024-byte FAT12 allocation units and 512-byte FAT16/32
allocation units; fixed roots have 512 entries. FAT12 needs the larger unit
to keep the 2 MiB fixture below the FAT16 cluster-count threshold.
Enabling exFAT and UTF-8 does not qualify those features: this runner currently
exercises only FAT12/16/32 with flat ASCII 8.3 names.

The helper uses `pread`/`pwrite` sector callbacks and reports requested image
I/O, errors and sync callbacks. Host source/extraction I/O is excluded from
these counters. Time includes native process startup, captured output and
parsing; it is not directly comparable to a sustained in-process API loop.
Read workflows open the common Hadris image read-only. Creation uses the
same final host `sync_data` barrier as every other peer.

FatFs `f_setlabel` updates the root label but leaves the boot-sector label
at the formatter's `NO NAME` value. The helper therefore creates a `NO NAME`
root label, keeping the existing strict label-consistency oracle satisfied
without modifying the upstream writer or generated bytes. Other formatters
use `HADRISCONF`; label values are not part of the file-content comparison.
This deliberate metadata difference should be considered when interpreting
formatting timings. The reference remains unmodified apart from official
patches and configuration.

`HADRIS_TESTS_PERF_PEERS` accepts comma-separated implementation names, such as
`hadris,chan-fatfs/helper,dosfstools+mtools`, to restrict comparisons to those
peers. An unset variable runs every available peer.

## Profiling FAT32 root operations

The peer runner has a diagnostic loop separate from its CSV comparison mode:

```sh
CARGO_PROFILE_BENCH_DEBUG=2 cargo bench --manifest-path tests/Cargo.toml --bench peers --no-run
HADRIS_TESTS_PERF_FILES=1000 \
HADRIS_TESTS_PROFILE_WORKLOAD=extract-tree \
HADRIS_TESTS_PROFILE_SECONDS=10 \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
```

Attach a platform sampler to the PID printed by `PROFILE_READY`. On macOS,
`sample PID 5 1 -fullPaths -file profile.txt` collects five seconds of stacks.
Fixture construction is before that marker; final output validation is after
it. Source images pass the raw FAT oracle before read profiling. Creation
and extraction validate the final output, and recorded operations reject I/O
failures. Extraction cleanup between operations appears in sampled stacks but
is outside each `PROFILE_SAMPLE` timer. Do not treat sampled operation timings
as an unprofiled performance baseline.

Workloads are `create-image`, `extract-tree`, `lookup-all`, and `copy-tree`.
`HADRIS_TESTS_PROFILE_PEER` selects `hadris` (default),
`rust-fatfs/unbuffered` or `rust-fatfs/buffered`; lookup and copy-tree probes
are Hadris-only. Setting `HADRIS_TESTS_PROFILE_CACHE` enables 256 chain
positions and enough directory-index entries for this fixture.
`HADRIS_TESTS_PROFILE_BLOCKS` selects the metadata-block bound (default 16).
`HADRIS_TESTS_PROFILE_POSITIONS` selects the chain-position bound (default 256),
and `HADRIS_TESTS_PROFILE_DIRECTORY_ENTRIES` selects the directory-prefix bound
(default: the fixture entry count). Each bound accepts zero to disable that
component; `HADRIS_TESTS_PROFILE_CACHE` must still be set.
`HADRIS_TESTS_PROFILE_SAMPLES=N` replaces the duration limit with one warm-up
and exactly `N` measured operations. Discard the first `PROFILE_SAMPLE` when
summarizing that mode; every operation, including warm-up, is logged.
Caching is available for lookup, extraction and copy-tree. The copy-tree
probe formats, mounts and copies a previously prepared host tree, allowing
cache configuration before copying. It does not reproduce the public writer's
stamping, serial seeding and preflight work, so compare cached/uncached probes
to each other rather than directly to the public `write` workflow.

A 2026-10-05 M3 Pro run with 1,000 small files plus the 128 KiB payload found:

| Probe | Cache disabled | Cache enabled | Requested image reads |
|---|---:|---:|---|
| Lookup every name after a fresh mount | 49.50 ms | 0.53 ms | 62,741 -> 75 |
| Extract every file | 111.24 ms | 58.05 ms | 63,916 -> 1,181 |
| Copy-tree probe, 16 blocks | 85.55 ms | 122.42 ms | 72,353 -> 38,996 |
| Copy-tree probe, 256 blocks | 85.55 ms | 96.90 ms | 72,353 -> 5,325 |

These are medians from short sequential unprofiled loops, not the earlier
fixed-21-sample dataset. Every iteration remounts; the enabled-cache read probes
learn entries lazily during that iteration. The larger copy-tree cache reduced
reads without a corresponding elapsed-time improvement in this run. Cache
lookup/maintenance and repeated scans still do work.

Five-second optimized/debug-symbol stack samples show about 55% of creation
samples inside `FatFs::plan` and about 60% inside `create_node` (inclusive,
so these percentages overlap). In extraction, lookup accounts for about 44%
of operation samples and host file writes about 54%. Cleanup accounts for
about 22% of all sampled stacks and is excluded from those operation shares.
The separate lookup loop spends most of its time in repeated directory scans
and image reads/seeks. Prioritize the existing directory index for host
extraction, then investigate bulk insertion planning; cache sizing alone is
not a demonstrated creation optimization.

For rust-fatfs, approximately 99% of sampled creation stacks are split between
name lookup and insertion-space scans. A standalone in-memory reproduction
produces 25,122,520 read calls and 25,157,141 seeks for 1,000 seven-byte files,
with no formatting, validation, unmount or host disk work in that counter
window. Directory records are deserialized using small field reads, and
`File::read` seeks before reading. `fscommon::BufStream` invalidates its read
buffer on every seek, explaining why that buffering variant barely reduces
this workload's read-call count. The reproduction, scaling results and
possible improvements are reported in
[rust-fatfs issue #123](https://github.com/rafalh/rust-fatfs/issues/123).

## FAT bulk creation after profiling fixes

A fresh 2026-10-06 Apple M3 Pro comparison used 21 measured samples per case,
1,000 seven-byte files and one 128 KiB payload in a 64 MiB FAT32 image.
Each configuration used the same host-workflow timer and raw-oracle validation.
The original library was `d1eeb6f8`; insertion-position reuse was `109730ae`;
bounded short-name planning was `c53490f7`.

| Implementation / configuration | Create median | Requested image reads | Image writes |
|---|---:|---:|---:|
| Hadris before these fixes | 94.82 ms | 72,353 | 7,593 |
| Hadris insertion-position reuse only | 92.16 ms | 67,017 | 7,593 |
| Hadris both fixes | 40.48 ms | 4,528 | 7,593 |
| dosfstools + mtools | 36.48 ms | Not instrumented | Not instrumented |
| ChaN FatFs helper | 133.55 ms | 127,393 | 6,739 |

The combined change is 2.34 times faster than the fresh baseline, with 93.7%
fewer requested image reads. It is about 11% slower than mtools and 3.30 times
faster than the FatFs helper in this run. These are separate sequential sample
sets with warm OS caches, not simultaneous trials or cold-device measurements.
The older 150.30 ms Hadris result above came from a different run; use the fresh
94.82 ms baseline for this change's improvement. rust-fatfs was excluded from
this focused rerun, so its earlier measurements are not new comparison data.

Insertion now resumes from the chain position reached during planning. Bulk
writing also retains a sorted, bounded set of short names and the next insertion
slot for a dense ASCII 8.3 directory. Long names, non-ASCII short names, deleted
slots, directory switches and capacity overflow retain the regular planning
path. Namespace mutations and interrupted-operation recovery discard the set.
File contents and metadata updates can retain it. Ordinary mounts leave this
set disabled; directory-cached mounts enable it within their requested entry
bound, capped at 2048. The bulk writer enables a 2048-name set internally,
using at most 22 KiB of name storage and no additional metadata-block cache.

Uncached extraction did not improve: 117.08 ms before versus 115.24 ms after,
with 63,916 reads in both configurations. The current mtools and FatFs helper
extraction medians were 70.64 ms and 86.24 ms. The existing directory-index
read probes above remain the evidence for pursuing that separate gap.

To reproduce the final focused comparison:

```sh
HADRIS_TESTS_PERF_FILTER=fat32 HADRIS_TESTS_PERF_FILES=1000 \
HADRIS_TESTS_PERF_SAMPLES=21 \
HADRIS_TESTS_PERF_PEERS=hadris,chan-fatfs/helper,dosfstools+mtools \
HADRIS_TESTS_CHAN_FATFS="$PWD/tests/target/chan-fatfs/chan-fatfs" \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
```

The helper must first be built with `python3 scripts/build-fatfs-peer.py`, and
mtools and dosfstools must be on `PATH`. Each case validates names and file
contents outside its timer; counters reported zero I/O failures throughout.

### Additional first-write I/O reduction

A follow-up avoids reading a partial device block when the entire block belongs
to a file's newly allocated chain and the write starts at that block's beginning.
The driver builds a zero-padded block in its existing buffer and writes it once.
Existing allocations and clusters smaller than the device block retain the
read/patch/write path, preserving neighboring data. Sparse first writes still
zero the gap.

Two alternating 21-sample before/after trials on 2026-10-06 produced:

| Trial | Before | After | Requested image reads before / after |
|---|---:|---:|---:|
| 1 | 39.08 ms | 39.33 ms | 4,528 / 3,528 |
| 2 | 40.10 ms | 39.13 ms | 4,528 / 3,528 |

This removes one 512-byte read for each small file, or 1,000 calls and 500 KiB
of requested image reads for the fixture. Image writes remain at 7,593. The
alternating trials do not establish a meaningful elapsed-time improvement;
this change reduces I/O without allocating another buffer. Validation includes
small clusters sharing a 4096-byte device block, overwrites and sparse writes.

An eight-block metadata-cache experiment was not retained: alternating medians
were 39.70/39.64 ms and 39.76/39.44 ms without/with that cache, and reads fell
only from 4,528 to 4,401. The small elapsed differences did not justify adding
a cache to the bulk writer.

Review also found that the on-disk `0x05` first-byte escape must be excluded
from the ASCII insertion index: it represents OEM byte `0xE5`, which a custom
code page can decode to an ASCII character. Such directories now use regular
planning, preserving case-insensitive duplicate rejection. The regression
fails before the fix and passes with it, including a targeted Miri run.


## FAT extraction profiling after the bulk-write fixes

The [per-trial measurements](benchmarks/fat-extraction-profile.csv) from the
2026-10-06 follow-up measured the merged `695a5d3f` drivers on the same
Apple M3 Pro with Rust 1.88.0, optimized builds and debug symbols. Only the
profiling runner changed: independent cache bounds and a fixed sample count.
The FAT32 image contains 1,000 seven-byte ASCII 8.3 files and one 128 KiB
payload. Every operation remounts the image and extracts to a fresh host
directory. Timing includes mount, root listing, lookup/open/read/close/forget
and host file creation/writes; destination cleanup and final validation are
outside the timer. OS caches remain warm. The source image passes the raw FAT
oracle, extracted paths and contents match the fixture, and all recorded
operations reject I/O failures. This is the root-only peer extraction workflow,
not the public lazy-tree extraction API or a nested-directory measurement.

Two unprofiled trials used 21 measured operations plus one discarded warm-up
per configuration; the second trial reversed configuration order:

| Cache configuration | Trial 1 median | Trial 2 median | Image reads | Requested image bytes |
|---|---:|---:|---:|---:|
| Disabled | 112.30 ms | 112.64 ms | 63,916 | 32,839,680 |
| Full directory index only, 1,001 entries | 59.82 ms | 60.51 ms | 2,226 | 1,254,400 |
| Directory index plus 16 blocks and 256 chain positions | 59.55 ms | 59.57 ms | 1,181 | 719,360 |
| Directory index only, bounded to 256 entries | 88.97 ms | 88.60 ms | 36,220 | 18,659,328 |

The full directory index alone removes 96.5% of image reads and makes this
workflow about 1.86 times faster. Adding the other caches roughly halves the
remaining reads but changes elapsed time by less than 1 ms in these trials.
A 256-entry prefix helps less once the directory exceeds its bound.

Separate two-second diagnostic runs, repeated in reverse order, isolated all
three cache components and varied the number of small files. Their image-read
counts matched between trials:

| Small files | Disabled | Directory index only, full | 16 blocks only | 256 chain positions only | All caches, full index |
|---|---:|---:|---:|---:|---:|
| 32 | 148 | 110 | 73 | 148 | 73 |
| 256 | 4,488 | 600 | 845 | 4,488 | 331 |
| 1,000 | 63,916 | 2,226 | 35,317 | 63,916 | 1,181 |
| 4,000 | 1,005,540 | 8,790 | 572,396 | 1,005,540 | 4,607 |

With 4,000 files, a 256-entry directory index still requests 881,844 reads.
The diagnostic runs had variable sample counts, as few as one measured
operation for the largest uncached configurations, and substantial timing
variation. Use them as I/O scaling evidence; the fixed-count table above is
the elapsed-time comparison. Caching chain positions alone does not reduce
reads in this sequential fixture; that says nothing about random access or
fragmented large files.

Five-second macOS stack samples, collected separately from the timing trials,
show name lookup at about 46% of uncached operation samples and host file
creation/writing at about 51%. With the full directory index alone, lookup
falls to about 2.5% and host creation/writing rises to about 95%. Directory
cleanup appears in sampled stacks but is excluded from these operation shares
and the timer. These are inclusive sampled-stack shares, not tracing spans or
an exact CPU accounting. A separate indexed lookup-only sample over 4,001
entries spends most samples in `lookup`; the index still searches its parsed
entries linearly in memory. It does not dominate the cached extraction run.

The extraction loop first lists entries and then calls `lookup` for each name.
Without the optional index, `find_entry` starts a new directory scan on every
lookup, explaining the approximately quadratic image-read growth. `readdir`
already parsed the entry but currently does not feed that information into
lookup. A focused next experiment should retain enough information from the
most recent listing to accelerate an immediate lookup, preserving normal
name/alias matching, fallback, mutation invalidation and corruption checks.
This could benefit sequential traversal with a small fixed memory bound.
Enabling a full-directory index internally remains an alternative, with
memory proportional to the configured bound. Nested trees, long names,
fragmentation and cache overflow need measurements before choosing a default
or changing the driver. No production behavior changed in this profiling pass.

To reproduce the directory-only fixed-count trial:

```sh
HADRIS_TESTS_PERF_FILES=1000 HADRIS_TESTS_PROFILE_WORKLOAD=extract-tree \
HADRIS_TESTS_PROFILE_SAMPLES=21 HADRIS_TESTS_PROFILE_CACHE=1 \
HADRIS_TESTS_PROFILE_BLOCKS=0 HADRIS_TESTS_PROFILE_POSITIONS=0 \
HADRIS_TESTS_PROFILE_DIRECTORY_ENTRIES=1001 \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
```

Unset `HADRIS_TESTS_PROFILE_CACHE` for the uncached trial. For the combined
trial set blocks to 16 and positions to 256; for the bounded-prefix trial
set directory entries to 256 while leaving the other bounds at zero.
