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
