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
