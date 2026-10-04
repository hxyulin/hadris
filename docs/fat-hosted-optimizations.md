# FAT hosted optimizations

Baseline: [audit](fat-hosted-performance-audit.md), Rust 1.97.1 on Apple M3 Pro,
seven release samples against a memory device. Each step has its own commit.

## 1. Single-cluster growth

`FatFs` reuses `allocate_after` with the existing pending-tail recovery state.
The raw bitmap iterator now visits set bits and skips words above the highest
member, avoiding a 2,048-bit scan for single-cluster updates.

FAT32 `append-512-end-sync`: device writes fall from 162 to 100, reads from
131 to 69, and bytes written from 82,944 to 51,200. Median CPU time is 5.50 µs
versus 6.21 µs in the baseline. [Measurements](benchmarks/fat-hosted-step1.csv).

Validation: a direct I/O-count regression on FAT12/16/32, failures at every
append write, cancellation at every append await, hosted write/async/contract
tests, raw-layer unit tests, and the allocator-only sync/async feature tier.

## 2. Prepared lookup queries

Node-driver lookup and creation scans fold the query once. ASCII 8.3 queries
compare a prepared padded name directly; Unicode and code-page names retain
the display conversion rules. Queries use 24 inline UTF-16 units and bounded
heap storage for longer names, keeping async future sizes within their
existing limits.

FAT32 `lookup-128`: median 283.96 µs versus 517.25 µs in the baseline.
I/O counts are unchanged. [Measurements](benchmarks/fat-hosted-step2.csv).
Validation covers Unicode and supplementary characters, the 255-unit limit,
short aliases, case flags, CP437/ASCII escape behavior, existing read/write/
async/contract tests, async future sizes, targeted Miri and the allocator-only
sync/async feature tier.

## 3. Optional bounded seek index

`CacheOptions` and `FatFs::with_cache` enable a global bound on learned chain
positions. Mount defaults to zero positions; configuring a cache reads no
I/O. A position keeps its original chain guard. Checkpoints span the file's
cluster count, are learned on demand and are discarded before mutation or
interrupted-operation recovery. `clear_cache` also resets traversal hints.

With 32 positions, FAT32 reverse 4 KiB reads fall from 2,435 to 320 device
calls and from 2,249.46 µs to 138.42 µs. Shuffled reads fall from 1,298 to 541
calls and from 1,127.92 µs to 149.21 µs. Sequential/fragmented reads retain their
I/O counts and pay extra CPU for index maintenance; enable it for seek-heavy
workloads. [Measurements](benchmarks/fat-hosted-step3.csv).

Validation covers bounded replacement, separate file identities, backward
reads with byte comparisons on FAT12/16/32, fragmented reads and mutation/slot
reuse across every test geometry, cyclic chains, dropped read futures,
existing hosted sync/async/contract tests and the allocator-only feature tier.
Public API snapshots include both modes and the umbrella reexport.

## 4. Optional metadata-block cache

`CacheOptions::with_blocks` bounds a read-through LRU cache of device blocks.
`CacheOptions::new` configures eight metadata blocks and 32 chain positions;
plain mount still enables neither cache. File payload and raw-device reads
bypass the metadata cache. Writes invalidate overlapping cached blocks before
awaiting the device, without deferring writes or changing flush behavior.
Both caches are usable with `alloc` + `no_std`.

With eight metadata blocks and no chain index, FAT32 unaligned reads fall from
1,043 to 787 calls. Short-name lookup falls from 1,008 to 129 calls. Median
CPU time is 44.42 µs for unaligned reads and 256.46 µs for short-name lookup;
metadata caching reduces I/O but does not remove repeated name scans.
[Measurements](benchmarks/fat-hosted-step4.csv).

Validation covers LRU bounds, range invalidation and clock wrap, payload bypass,
metadata reuse after partial payload reads, failed mirror writes, cancellation,
mutation/slot reuse, existing read/write/async/contract tests and the allocator-
only tier. The async resource caps allow 64 additional bytes for the metadata
adapter (rename/label 3,648 bytes, create 2,304 bytes, extents 576 bytes); they
remain tested rather than ignored. Embedded resource limits are unchanged.

## 5. Batched growth and tail linking

`allocate_run_after` combines the old tail link with the final FAT16/32
allocation group when both occupy the same device block in every FAT copy.
Recovery recognizes a tail pointing at either the completed head or the
in-progress group, mirrors interrupted entries, detaches the old tail and
reclaims the allocation. FAT12 retains its packed-entry path. Disabled
metadata caches bypass lookup and invalidation bookkeeping immediately.

Without optional caches, FAT32 writes for a 1 MiB file fall from 1,314 to 804
with 4 KiB requests, and from 114 to 84 with 64 KiB requests. Bytes written
fall from 1,590,272 to 1,329,152 and from 1,098,752 to 1,083,392 respectively.
Median CPU times after all five changes are 138.54 µs and 89.96 µs, versus
1,698.79 µs and 262.17 µs in the original baseline. These cumulative runtime
improvements also include the bitmap iteration fix in step 1; the batching
change accounts for the write-count reductions. FAT12's 64 KiB workload
retains 8,225 writes and measures 305.12 µs versus the baseline's 273.79 µs;
the series is not a CPU improvement for every workload.
[Measurements](benchmarks/fat-hosted-step5.csv).

Validation covers allocation across multiple FAT blocks on FAT12/16/32 with
one and two FAT copies, FAT32 reserved bits and active-copy selection, invalid
inputs, direct hosted write counts, and
failure at every write/cancellation at every await during multi-cluster
growth with both caches enabled and disabled. The failure fixtures cover
512-byte and 4 KiB device geometries and independently validate the resulting
allocation graph and file contents. Existing hosted and embedded tests,
feature tiers, API snapshots, firmware budgets and the FAT conformance suite
provide the final series checks.

## Configuring both caches

The [combined run](benchmarks/fat-hosted-combined.csv) uses 32 chain positions
and eight metadata blocks after all five changes. FAT32 reverse and shuffled
4 KiB reads use 283 and 296 device calls respectively (baseline: 2,435 and
1,298), with medians of 140.33 µs and 145.38 µs. Short-name lookup uses 129
calls (baseline: 1,008), measuring 291.12 µs. The boot-load case retains 39
reads and measures 36.54 µs versus the baseline's 32.67 µs: the configured
caches offer no I/O gain for this forward-only workload. Choose cache bounds
for the application's access pattern rather than enabling them universally.

Validation of the original five optimizations: 335 passing crate tests/doctests across the full dual-mode
suite and the final added raw-layer regression, all 15 FAT/raw CI feature tiers
on Rust 1.88 with warnings denied, workspace check with warnings denied,
formatting, targeted clippy, public API snapshots and sync/async/local parity.
The firmware budget checker passes on thumbv6m, thumbv7em and riscv32imc.
The FAT conformance suite passes nine tests; four optional peer/native tests
remain ignored. No physical-media timing or native mounted-image run is
claimed by these measurements.

## 6. Reuse invalidated metadata buffers

Invalidation and `clear_cache` mark block slots unavailable without dropping
their buffers. Misses refill invalid slots before allocating new buffers or
replacing live LRU entries. The bound and device I/O are unchanged; cache
storage remains allocated until the cache configuration is replaced or the
filesystem is dropped. Clock wrap preserves the distinction between invalid
slots and live entries, including block index `u64::MAX`.

The reuse regression retains the same owned buffers across 32 invalidation
cycles and explicit clearing for 512-byte and 4 KiB blocks. Existing failed
mirror writes, cancellation, payload bypass, mutation and async resource
checks also pass, along with the allocator-only sync/async/write tier.
The [seven-sample run](benchmarks/fat-hosted-step6.csv) uses 32 positions and
eight blocks. FAT32 4 KiB writes measure 152.88 µs versus 164.92 µs in the
previous combined run; FAT12 64 KiB writes measure 457.92 µs versus 606.92 µs.
I/O counts are identical. Short-name create/remove measures 167.75 µs versus
162.88 µs, so these CPU timings do not establish improvement in every case.

## 7. Cheaper indexed forward reads

Reads whose hint already identifies the requested cluster return that hint
without computing checkpoint spacing or searching the index. Checkpoints
store their bucket at insertion, avoiding repeated division while comparing
cached positions. Mutations still clear the index before file size changes,
so a node's checkpoint spacing remains consistent. Chain guards and
replacement policy are unchanged.

The new `read-128` benchmark reads 1 MiB in 8,192 calls, exercising repeated
reads within each cluster. Paired 21-sample runs use the same harness and
32-position/eight-block configuration, with the previous commit as baseline.
Median CPU times are:

| Variant | Before | After | Device reads (unchanged) |
|---------|-------:|------:|------------------------:|
| FAT12 | 197.83 µs | 169.00 µs | 2,056 |
| FAT16 | 189.33 µs | 145.04 µs | 2,054 |
| FAT32 | 199.42 µs | 171.54 µs | 2,066 |

[Before](benchmarks/fat-hosted-small-read-before.csv),
[after](benchmarks/fat-hosted-small-read-after.csv).
The [existing-workload run](benchmarks/fat-hosted-step7.csv) retains every
I/O count from step 6 and shows mixed CPU changes: FAT32 4 KiB reads are
41.42 µs versus 41.33 µs, while unaligned reads are 47.67 µs versus 52.54 µs.
The improvement is primarily for small reads; it is not a universal
sequential-read speedup.

Validation includes small forward reads across every test geometry followed
by backward/scattered seeks and cache clearing, with metadata caching on and
off. Existing cyclic-chain detection, cancellation, reverse-read counts,
async resource guards and payload tests also pass.

## Further work

Directory indexing and packing-safe FAT12 allocation batching remain separate
follow-ups. A 64-block metadata cache reduces FAT32 long-name lookup reads to
33 versus 2,001 with eight blocks, but CPU remains about 915 µs versus
947 µs: removing repeated entry scanning needs a directory-level index.
FAT12 bulk allocation still updates packed entries individually and requires
its own block-boundary and interruption-recovery design before batching.

The two follow-up commits pass the full 337-test FAT/raw dual-mode suite,
allocator-only sync/async/write checks with warnings denied, workspace
checking with warnings denied, formatting and clippy. All 126 benchmark
smoke cases validate their output; all 60 existing hosted cases retain their
device-call, byte and flush counts after the forward-read change.
