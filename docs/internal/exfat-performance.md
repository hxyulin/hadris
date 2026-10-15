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

## Hosted append tail hints

`docs/benchmarks/exfat-tail-hint.csv` records the final 21-sample run. Growth of
a chained file reuses its existing complete ChainPos, including its cycle
guard, when its index is within the file allocation. It still follows the FAT
to EOF; a hint is not treated as proof of an end marker. Shrinking and recovery
reset hints, and NoFatChain conversion retains its existing path.

| Cluster size | Write request | Baseline reads | Final reads | Baseline median | Final median |
|---|---|---:|---:|---:|---:|
| 512 B | 512 B | 23,846 | 8,198 | 15.13 ms | 0.368 ms |
| 512 B | 64 KiB | 255 | 135 | 208.25 us | 96.71 us |
| 4 KiB | 512 B | 1,164 | 1,026 | 391.46 us | 121.46 us |
| 4 KiB | 64 KiB | 83 | 75 | 43.88 us | 28.92 us |

Write counts and bytes do not change. The first row's saved reads total
8,011,776 bytes. Failure at every append write and cancellation at every
append await cover 512-byte and 4 KiB clusters, initialized-to-zero gaps, and
NoFatChain conversion. Every recovered image passes the structural checker
and remounted exact-content comparison. A read-count regression bounds a
warmed-tail append on a 2,048-cluster file, and exercises shrink then append.

The three optimizations add no public API or driver-state cache. They are
independent of the optional FAT12/16/32 hosted caches. Remaining opportunities
include exFAT directory scan costs and batched NoFatChain-to-FAT conversion;
this change does not redesign allocation or name handling.

## Validation and firmware budgets

The final change passes 344 FAT/raw tests and doctests, including the new
regressions, the sync/async/local contract tests, and native macOS formatter
and kernel-reader tests. All 15 FAT/raw MSRV feature tiers, workspace warnings
checking, formatting and clippy pass. No public API, unsafe block, name decoding
logic, or allocation-write ordering changes.

The pinned nightly firmware budget check passes for all three bare-metal
targets. The exFAT reader retains 1,016-byte driver state and zero static RAM.

| Target | exFAT flash | Mount stack | Largest Hadris frame |
|---|---:|---:|---:|
| thumbv6m-none-eabi | 14,128 B | 1,928 B | 688 B |
| thumbv7em-none-eabihf | 13,956 B | 1,856 B | 688 B |
| riscv32imc-unknown-none-elf | 16,372 B | 1,856 B | 688 B |

These are compiler estimates of direct-call stack usage; indirect callees and
compiler builtins are excluded by the firmware script. The existing unrelated
20 KiB FAT logger flash target remains an open goal, while the enforced 44 KiB
ceiling passes.
