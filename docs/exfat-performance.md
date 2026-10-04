# exFAT performance

Run `cargo bench -p hadris-fat --features async --bench exfat_performance -- 7`.
The optional positional argument is the number of samples. The benchmark emits
CSV and measures successful device calls, bytes, flushes, and median CPU time.
Mounting, image cloning, allocation of scratch buffers, and read-file lookup are
outside timed regions. Write-file creation is also outside the measured region;
final synchronization is included. Timed reads do not compare payloads. Every
workload first runs a checked warmup, validates the complete image with the exFAT
checker, and verifies file contents after remounting; timed samples must have
identical device counters to that warmup.

The fixtures use 512-byte device blocks and 512-byte or 4 KiB clusters. Read
fixtures cover FAT chains, native-style contiguous `NoFatChain` allocations, and
fragmented chains with every other cluster relocated. Reads transfer 1 MiB in
64 KiB requests. Hosted writes transfer 1 MiB in 512-byte or 64 KiB requests.
The allocator-free embedded driver is read-only. Hosted measurements apply to
both std users and alloc-enabled bootloaders. CPU timings use a memory device,
not physical disk latency.

## Baseline

`docs/benchmarks/exfat-baseline.csv` records seven-sample Rust 1.97.1 measurements
on the development Mac. At 512-byte clusters, embedded contiguous reads need
2,048 payload reads, versus 16 with the hosted driver. Embedded chained reads
need 2,065 reads, versus 33 hosted. Fragmentation prevents payload coalescing and
both need 4,095 reads.

Small hosted writes also stand out: 512-byte requests with 512-byte clusters
need 23,846 reads and 8,195 writes, with a median CPU time of 15.13 ms. The tail
linker walks the entire existing chain for each growth operation. This provides
a separate candidate for reusing the node's already guarded chain position.

## Embedded read runs

`docs/benchmarks/exfat-embedded-runs.csv` captures the first optimization.
At 512-byte clusters, contiguous reads drop from 2,048 calls to 16 (29.04 to
16.08 microseconds), and adjacent FAT chains drop from 2,065 to 33 calls
(40.79 to 27.83 microseconds). At 4 KiB clusters the same counts drop from
256 to 16 and from 259 to 19. Fully fragmented reads retain their original
counts, and their CPU timings vary slightly; there is no claimed fragmented
read speedup. Device bytes stay identical in all cases.

The reader coalesces only initialized bytes, validates the final contiguous
cluster against the heap, and advances the full chain guard for chained runs.
Regression tests cover partial clusters, seeks, short ValidDataLength, unchanged
bytes beyond EOF, fragmentation, cycles, heap overflow, and cancellation at
every await while preserving file position. Driver state is unchanged.

## Hosted contiguous run sizing

The hosted driver now sizes NoFatChain runs arithmetically, checking the final
cluster and index instead of iterating every intermediate cluster. Chained
reads retain the same guarded traversal. The 21-sample results are in
`docs/benchmarks/exfat-hosted-runs.csv`: device counts and bytes remain identical;
512-byte-cluster contiguous reads measure 15.42 microseconds against the
17.58-microsecond original baseline. Timings are small enough that memory-device
noise matters, so this change primarily removes work proportional to the
number of clusters within each request. Regressions exercise unaligned starts,
ValidDataLength transitions, EOF and a run exceeding the cluster heap.
