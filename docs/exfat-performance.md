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
