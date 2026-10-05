# UDF reader performance

Measured on 2026-10-05 with Rust 1.97.1, Cargo's optimized bench profile and an
Apple M3 Pro. Each row has one warm-up and 21 measured samples, reporting the
median. The final harness was run against baseline `a8d5149b` and the optimized
reader, so before/after cases use the same instrumentation and fixtures.
The [benchmark guide](performance.md#udf-benchmark) describes workload boundaries.

## Identifier reuse

The reader previously fetched the 38-byte identifier header, reread its covered
bytes to validate the CRC, then fetched its name separately. It now computes
CRC over the header already read and captures name bytes while reading the
remaining covered bytes. When CRC coverage ends before the name, it reads the
remaining name bytes as well. Large implementation-use fields remain streamed
through the same 512-byte buffer, and names crossing chunks are assembled in a
bounded 255-byte array. Padding outside both CRC coverage and the name is not
prefetched. Implementation-use bytes outside CRC coverage are skipped. Parent
and deleted records need no separate name read.

The reader still decodes each name only when lookup or listing needs it,
accepts shortened CRC coverage, validates covered bytes, and reads across
allocation boundaries. Mounting does not load directory trees or file payloads.
No heap allocations or persistent fields were added to `UdfFs`. Temporary
identifiers now carry the encoded name; the old separate name-read scratch is
removed. Peak stack usage was not measured.

## Device calls

The fixture contains 1,000 numbered files plus a 1 MiB payload. These rows use a
2,048-byte device. Cached rows use the existing shared storage adapter after
identifier reuse; counts are successful calls reaching the underlying device.

| Workload | Baseline | Identifier reuse | Cache 8 blocks | Cache 32 blocks |
|---|---:|---:|---:|---:|
| list | 5,047 | 4,030 | 1,027 | 1,027 |
| lookup-last | 3,042 | 2,026 | 26 | 26 |
| lookup-miss | 3,045 | 2,028 | 26 | 26 |
| stat-100 | 100 | 100 | 1 | 1 |
| read-4k | 512 | 512 | 257 | 257 |
| read-64k | 32 | 32 | 17 | 17 |
| read-scattered | 512 | 512 | 257 | 257 |

Without caching, listing reduces calls and bytes by 20.2%; last-entry lookup
reduces both by 33.4%. Eight cached blocks reduce listing calls by 79.7% and
last-entry lookup by 99.1% versus the baseline. Listing still reads each child
ICB to obtain metadata; caching avoids repeatedly reading the parent ICB and
directory blocks. The cache removes repeated file-ICB reads from the file
workloads. The first metadata request still needs one device read.

Uncached mount remains 13 calls on a 2,048-byte device and 16 on a 512-byte
device. Uncached payload read counts are unchanged by identifier reuse.

## CPU time

Median microseconds on `MemDevice`, including output validation for file reads:

| Workload | Baseline | Identifier reuse | Cache 8 blocks | Cache 32 blocks |
|---|---:|---:|---:|---:|
| list | 2,293.0 | 2,060.1 | 2,126.4 | 2,152.8 |
| lookup-last | 752.6 | 505.4 | 519.6 | 555.6 |
| lookup-miss | 725.4 | 504.2 | 517.5 | 515.4 |
| read-4k | 536.9 | 502.5 | 528.7 | 540.0 |

Identifier reuse improves last-entry lookup by about 33% and listing by about
10% in this run. Caching has much larger I/O savings than CPU savings: CRC
validation and decoding still run on cached bytes, and LRU bookkeeping adds
work. Scattered file reads with caching are slower in this memory-device run.
These timings do not estimate optical-drive, file-backed or network-device
latency. Small timing changes in unchanged workloads are measurement variation;
device counts are the stronger evidence here.

## Cache capacity and memory

UDF already accepts `hadris_storage::sync::Cache` and its async counterpart.
This change documents, benchmarks and tests that composition rather than adding
a second cache implementation or a UDF cache feature. It requires the storage
crate's `alloc` feature; ordinary mounting still needs no allocator.

Capacity counts physical device blocks. Eight blocks have up to 4 KiB of payload
storage on a 512-byte device or 16 KiB on a 2,048-byte device; 32 blocks have up
to 16 KiB or 64 KiB respectively. LRU entries, tree nodes and allocator overhead
are additional and were not measured. Slots allocate as needed. Large requests
bypass the cache; small payload reads can occupy it alongside metadata.

On a 512-byte device, listing takes 5,163 calls before optimization and 4,100
after identifier reuse. Eight cache blocks increase this to 5,913 through
repeated eviction, despite transferring fewer bytes. Increasing to 32 blocks
reduces it to 1,103 calls. Capacity must therefore be sized in bytes and working
set, rather than assuming an eight-block cache is sufficient for every device.
The default remains uncached.

## Validation and follow-ups

Regression tests bound uncached listing and lookup calls, cover Unicode names
crossing the buffer boundary with short and full CRC coverage, reject covered
byte corruption, and retain the existing cross-extent and continuation tests.
The Unicode/CRC regression passes under Miri. Cache tests cover capacities 1,
8 and 32 on 512- and 2,048-byte devices in both modes, including eviction,
unaligned/backward reads, symlinks and directories. Cold cache failures preserve
the original device error, and retry after clearing the injected failure works.
Strict external interoperability with 7-Zip, `mkudffs` and `udfinfo` passes.

Useful separate follow-ups are a fragmented-allocation benchmark, profiling CRC
and name decoding, and writer allocation/copy reduction. An internal ICB cache
would duplicate some existing cache benefits and add reader state; these results
do not justify enabling one by default. Read-ahead and whole-directory loading
remain unimplemented.

## Data

- [Uncached baseline](benchmarks/udf-reader-before.csv)
- [Identifier reuse](benchmarks/udf-reader-identifiers.csv)
- [Eight cached blocks](benchmarks/udf-reader-cache8.csv)
- [32 cached blocks](benchmarks/udf-reader-cache32.csv)
