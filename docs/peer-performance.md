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
