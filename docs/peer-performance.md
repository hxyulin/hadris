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

Workloads are `create-image`, `extract-tree`, `extract-lazy-tree`, `lookup-all`,
and `copy-tree`. The public lazy-tree extraction probe is documented below.
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


## Configurable listing hint for hosted FAT extraction

The driver now offers `CacheOptions::sequential()`: one lazily allocated
listed-entry hint, with the directory-prefix, block and chain caches disabled.
`with_directory_hint(bool)` is independent of those bounds. Direct mounts
remain uncached. FAT CLI extraction selects the sequential preset by default
and exposes explicit bounds plus `--no-directory-hint` and `--no-cache`.
The hint reuses the existing name/alias matcher, retains one parsed short entry
and at most 255 UTF-16 units, and is cleared before mutations, during recovery,
on explicit cache clearing, and when listing fails or ends. Nonmatching names
and other directories retain regular lookup. It does not bypass directory
walking or its cycle checks.

On this 64-bit Mac, compiler layout output reports 88 bytes for the heap-allocated
hint. A short name needs no additional name allocation; a maximum-length long
name adds 510 bytes, for 598 bytes excluding allocator overhead. The allocation
for the hint itself is reused between successful listings. Long names are copied
into separate bounded allocations. No hint storage is allocated until listing
an entry with the option enabled. This cost is independent of directory size.

The [per-trial measurements](benchmarks/fat-extraction-cache.csv) below use the
public lazy-tree workflow: `read_tree` on a mounted `Volume`, then host
`write_tree`. This includes tree construction, metadata reads, lazy file reads,
and host creation, permissions and timestamp restoration. It differs from the
root-only peer extraction loop above, so compare configurations within this
workflow rather than treating these timings as a regression against that loop.
The counter uses an `Arc<Mutex<IoCounts>>` because lazy volume content requires
a `Send` driver; its lock overhead is included consistently in every configuration.
Fixture creation, destination cleanup and final validation remain outside the
timer. Each input passes the independent raw FAT oracle; every output matches
all paths and contents, and every recorded operation rejects I/O failures.

On 2026-10-06, the same M3 Pro/Rust 1.88.0 configuration ran two trials per
layout, with 21 measured operations after one discarded warm-up. Configuration
order was reversed for the second trial. Every layout has 1,000 seven-byte files
and a 128 KiB payload. Nested layouts distribute them across eight directories,
each containing a `nested` child; the long-name layout uses Unicode long names.
OS caches remain warm.

| Layout | Configuration | Trial 1 median | Trial 2 median | Image reads |
|---|---|---:|---:|---:|
| Root, short names | Disabled | 130.52 ms | 129.66 ms | 63,871 |
| Root, short names | Listing hint only | 77.66 ms | 77.21 ms | 1,133 |
| Root, short names | Directory index only | 78.18 ms | 78.26 ms | 1,257 |
| Root, short names | All caches and listing hint | 77.67 ms | 77.53 ms | 1,079 |
| Nested, short names | Disabled | 86.65 ms | 86.14 ms | 9,113 |
| Nested, short names | Listing hint only | 79.09 ms | 79.67 ms | 1,154 |
| Nested, short names | Directory index only | 79.79 ms | 79.74 ms | 1,266 |
| Nested, short names | All caches and listing hint | 79.87 ms | 79.81 ms | 1,090 |
| Nested, long names | Disabled | 108.81 ms | 108.68 ms | 33,265 |
| Nested, long names | Listing hint only | 81.07 ms | 81.65 ms | 1,537 |
| Nested, long names | Directory index only | 81.94 ms | 82.51 ms | 2,281 |
| Nested, long names | All caches and listing hint | 81.57 ms | 82.09 ms | 1,300 |

Directory-only trials configure 1,001 entries; combined trials add 16 metadata
blocks, 256 chain positions and the hint. The hint removes 98.2% of root image
reads and makes root extraction about 1.68 times faster without allocating the
roughly 94 KiB full index. Nested directories need fewer uncached rescans, so
the elapsed gain is smaller. The full caches do not show a material additional
time benefit for these sequential fixtures. They remain useful options for
other patterns; fragmentation and random-lookup workloads need their own trials.

To reproduce a fixed-count nested long-name hint trial:

```sh
HADRIS_TESTS_PERF_FILES=1000 HADRIS_TESTS_PROFILE_WORKLOAD=extract-lazy-tree \
HADRIS_TESTS_PROFILE_SAMPLES=21 HADRIS_TESTS_PROFILE_DIRECTORIES=8 \
HADRIS_TESTS_PROFILE_LONG_NAMES=1 HADRIS_TESTS_PROFILE_CACHE=1 \
HADRIS_TESTS_PROFILE_DIRECTORY_HINT=1 HADRIS_TESTS_PROFILE_BLOCKS=0 \
HADRIS_TESTS_PROFILE_POSITIONS=0 HADRIS_TESTS_PROFILE_DIRECTORY_ENTRIES=0 \
  cargo bench --manifest-path tests/Cargo.toml --bench peers
```

Unset `HADRIS_TESTS_PROFILE_CACHE` for the baseline. Set directories to zero and
unset long names for the root fixture. Directory/long-name fixture controls are
restricted to `extract-lazy-tree`. An unset directory-hint control disables
that component even when the other caches are enabled.


## Isolated extraction RSS and peer comparison

The 2026-10-06 resource follow-up uses the same 64 MiB FAT32 root fixture:
1,000 seven-byte ASCII 8.3 files and a 128 KiB patterned payload. The fixture
is prepared and checked by the raw FAT oracle in a separate process. Every
measured process opens that common read-only image and extracts once; parent
validation compares every output name and byte afterward. Fixture construction,
validation and destination cleanup do not enter the extraction process's RSS
or timer. The image remains unchanged. mcopy requires its empty destination to
exist, so that directory is prepared outside its timer; other implementations
include creating it. Output files are closed without a durability barrier. No physical disk or
OS-cache eviction is performed.

The machine is the same Apple M3 Pro, macOS 27.0.1, Rust 1.88.0. Peers are
GNU mtools 4.0.49 from Nix, rust-fatfs at the pinned commit
`2aefc2a027ce94ed0671752814dac203f0450e11` with fscommon 0.1.1 for buffering,
and the existing unmodified/patched ChaN R0.16 helper configuration above.
Hadris production code is `6410c6ac`, with diagnostic worker changes only.
The before-hint driver is `695a5d3f`, compiled with the same worker and release
settings, adapting only unavailable cache configuration methods. Baseline and
current executable hashes are distinct and recorded with the raw artifacts.

Each case has two trials of 21 fresh processes plus one discarded warm-up;
the second reverses configuration order. Time is parent `perf_counter_ns` around
`/usr/bin/time -l` and the extraction child, including process startup and exit.
Peak RSS is the child's macOS maximum resident set size in bytes, measured
from process start to exit. It includes touched executable/runtime pages, node
state, buffers and trees, and is not a measurement of cache allocation alone.
These timings are separate from the in-process loops above.

### Cost when optional caching is disabled

With a `FileDevice`, `size_of::<FatFs<FileDevice>>()` increases from 4,600 to
4,608 bytes: eight bytes of fixed state, with no optional cache allocation at
mount or during uncached reads. Existing driver buffers and node allocations
still exist. The independent compiler layout check with a memory device also
reports an eight-byte increase. The bounded hint is never allocated unless
explicitly enabled and an entry is listed.

[Paired before/after measurements](benchmarks/fat-extraction-disabled-baseline.csv)
use the correctly identified baseline/current executables:

| Uncached workflow | Before median | After median | Peak RSS before / after | Image reads before / after |
|---|---:|---:|---:|---:|
| Public lazy-tree extraction | 124.70 ms | 123.87 ms | 3.000 / 3.016 MiB | 63,871 / 63,871 |
| Streaming content extraction | 115.94 ms | 113.24 ms | 2.484 / 2.469 MiB | 64,840 / 64,840 |

These runs do not show a slowdown with caching disabled. The small timing
differences are not evidence that the hint improves disabled execution.
RSS medians shift by one 16 KiB page in opposite directions between workflows,
with overlapping sample ranges; do not interpret that as an eight-byte heap
measurement. An initial mislabeled baseline copy was caught by executable hash
verification, excluded, and replaced by these paired trials.

### Peer extraction results

[Per-process timing and RSS samples](benchmarks/fat-extraction-peer-resources.csv)
produce the following combined medians over 42 measured processes per case:

| Implementation / workflow | Time | Peak RSS | Image read calls | Requested image bytes |
|---|---:|---:|---:|---:|
| Hadris public lazy tree, disabled | 124.47 ms | 3.016 MiB | 63,871 | 32,832,000 |
| Hadris public lazy tree, hint only | 73.37 ms | 3.016 MiB | 1,133 | 710,144 |
| Hadris public lazy tree, full index only | 74.45 ms | 3.109 MiB | 1,257 | 773,632 |
| Hadris streaming content, disabled | 111.00 ms | 2.469 MiB | 64,840 | 33,312,768 |
| Hadris streaming content, hint only | 57.79 ms | 2.469 MiB | 2,102 | 1,190,912 |
| Hadris streaming content, full index only | 58.55 ms | 2.578 MiB | 2,226 | 1,254,400 |
| rust-fatfs, unbuffered | 63.55 ms | 2.375 MiB | 13,650 | 172,464 |
| rust-fatfs + fscommon buffering | 63.78 ms | 2.375 MiB | 13,616 | 6,971,392 |
| ChaN FatFs helper | 79.95 ms | 1.719 MiB | 64,126 | 32,832,512 |
| mtools mcopy | 63.09 ms | 3.164 MiB | 13 | 741,376 |

The public Hadris path constructs a lazy tree, retains file pins and restores
host permissions and times. The streaming helper interleaves listing, lookup,
whole-file reading and host writing, without tree construction or metadata
restoration. It is a diagnostic content-only extraction path, not a new public
API. Peer helpers likewise validate content; their metadata policies are not
qualified here. Compare the public path's extra work explicitly when assessing
its elapsed time or memory against peers. Rust peers share the benchmark
executable with Hadris; mtools and ChaN run directly in their own processes.

For public extraction the hint reduces reads by 98.2% and gives about a 1.70x
elapsed improvement, with no observable peak RSS increase in this fixture.
The full 1,001-entry index adds 96 KiB to public-path peak RSS; the streaming
index's peak is 112 KiB higher. These differences include allocator, paging and
code-path effects rather than directly measuring heap allocation. A single hint remains much
smaller than either. Streaming hint extraction is competitive with the other
content helpers here, while public extraction still spends extra time on its
tree and metadata work. These figures do not establish a universal peer ranking.

### Counting image reads in external processes

Read counts and bytes are requested image I/O, not physical device reads. Hadris,
rust-fatfs and ChaN counters are recorded in every timing run. mtools is measured
in three separate instrumented extractions using
`tests/peers/image-read-counter-macos.c`. The Darwin interposer counts `read`,
`pread` and `readv` only when the descriptor's device/inode matches the input
image, and records seeks, delivered bytes and failures. It is absent from all
speed/RSS trials. The same instrumentation cross-checks every library/helper;
all counters match their internal read counts and requested bytes. Every count
is identical across three instrumented runs, and all extractions have zero I/O
failures. See [the syscall samples](benchmarks/fat-extraction-image-io.csv).

mcopy imports the instrumented read/seek functions and performs 13 large image
reads, averaging about 55.7 KiB. Its much smaller call count despite comparable
bytes suggests image read-ahead/batching as a further investigation. The buffered
rust-fatfs variant requests about 40 times more bytes than the unbuffered variant
without materially reducing calls or elapsed time in this workload.

To reproduce one isolated Hadris extraction, first build the release runner
and prepare an image outside the measured process:

```sh
cargo bench --manifest-path tests/Cargo.toml --bench peers --no-run
HADRIS_TESTS_PEER_WORKER=prepare HADRIS_TESTS_PERF_FILES=1000 \
HADRIS_TESTS_PEER_IMAGE="$PWD/common.img" "$PEERS_BIN"
/usr/bin/time -l env HADRIS_TESTS_PEER_WORKER=hadris/lazy \
HADRIS_TESTS_PEER_CACHE=hint HADRIS_TESTS_PEER_IMAGE="$PWD/common.img" \
HADRIS_TESTS_PEER_DESTINATION="$PWD/out" "$PEERS_BIN"
```

`PEERS_BIN` is the executable path printed by the build. Select `hadris/stream`,
`rust-fatfs/unbuffered` or `rust-fatfs/buffered` as the worker. Cache modes are
`none`, `hint`, and `index`; index defaults to 1,001 entries and accepts
`HADRIS_TESTS_PEER_DIRECTORY_ENTRIES`. Always validate outputs in the parent and
use a fresh destination. The worker supports the qualified root-file fixture;
streaming content and the current ChaN helper do not accept nested directories.

On macOS, build and calibrate the image counter before instrumenting a peer:

```sh
cc -dynamiclib -O2 -Wall -Wextra -Werror \
  tests/peers/image-read-counter-macos.c -o image-read-counter.dylib
cc -O2 -Wall -Wextra -Werror \
  tests/peers/image-read-counter-check.c -o image-read-counter-check
DYLD_INSERT_LIBRARIES="$PWD/image-read-counter.dylib" \
HADRIS_IO_IMAGE="$PWD/common.img" ./image-read-counter-check common.img
```

Calibration reads through three APIs, excludes a second descriptor and tests a
failed image read while preserving `errno`; its expected report is
`IMAGE_IO,4,160,96,1,1`. Apply the same environment directly to the peer binary
for counting. Do not run the interposer through Apple's protected `time` binary,
which can strip `DYLD_*` variables, or use instrumented elapsed/RSS numbers as
performance measurements. The helper is diagnostic instrumentation, not part
of the filesystem drivers.
