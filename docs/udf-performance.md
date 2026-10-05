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

The follow-up measurements below cover fragmented allocation, CRC and name
decoding. Writer allocation/copy reduction remains a separate investigation. An internal ICB cache
would duplicate some existing cache benefits and add reader state; these results
do not justify enabling one by default. Read-ahead and whole-directory loading
remain unimplemented.

## Data

- [Uncached baseline](benchmarks/udf-reader-before.csv)
- [Identifier reuse](benchmarks/udf-reader-identifiers.csv)
- [Eight cached blocks](benchmarks/udf-reader-cache8.csv)
- [32 cached blocks](benchmarks/udf-reader-cache32.csv)

## Fragmented allocation follow-up

The expanded harness has 108 cases per cache capacity, with 21 measured samples
on the same host/toolchain. These are measurements of the current reader, not
additional production optimizations. All three layouts contain the same names
and logical payload. The inline layout holds 128 file extents in the file entry;
the chained layout has 127 file AEDs and, for 1,000 entries, 25 directory AEDs.

Successful backend calls on the 1,000-entry, 2,048-byte-device cases:

| Workload | Contiguous | Fragmented inline | Fragmented AED | AED + cache 8 | AED + cache 32 |
|---|---:|---:|---:|---:|---:|
| list | 4,030 | 4,030 | 7,847 | 1,052 | 1,052 |
| lookup-last | 2,026 | 2,026 | 5,839 | 51 | 51 |
| read-4k | 512 | 512 | 1,143 | 384 | 384 |
| read-scattered | 512 | 512 | 11,292 | 10,980 | 10,932 |

Of the 11,292 scattered-read calls, 10,780 fetch AED blocks: repeated traversal
is the dominant I/O amplification. The uncached workload transfers 23,650,304
bytes to return 1,048,576 bytes. A 32-block cache still fetches AEDs 10,480 times;
it reduces total calls by only 3.2%. In this memory-device run its median rises
from 1.271 ms to 2.122 ms, about 67%, because it retains too little of the chain
and adds bookkeeping. In contrast, it reduces listing AED reads from 3,817 to
25 and last-entry lookup AED reads from 3,813 to 25.

Inline fragmentation demonstrates a separate CPU cost: sequential 4 KiB reads
still make 512 calls, but take 0.971 ms versus 0.262 ms for contiguous allocation.
The file entry contains substantially more CRC-covered allocation bytes and is
validated for every read. That is a likely contributor based on the code and
isolated CRC measurements; this run does not quantify its exact share.

Every measured sample checks names and payload content and reproduces its
case's device counts. 7-Zip 26.02 independently extracts all names and the exact
payload from both directory sizes in the contiguous and inline layouts. It
rejects the AED layouts: its [UDF parser](https://github.com/ip7z/7zip/blob/main/CPP/7zip/Archive/Udf/UdfIn.cpp)
collects continuation descriptors as extents, while its
[extent checks](https://github.com/ip7z/7zip/blob/main/CPP/7zip/Archive/Udf/UdfIn.h)
require recorded extents and their lengths to sum to file size. Independent
interoperability coverage is therefore limited to the four non-AED images.
The fixture padding keeps closing anchors within 7-Zip's bounded scan after
used extents. No writer behavior was changed for this measurement.

## Native CPU profiles and isolated kernels

Native macOS `sample` captured six seconds at one millisecond intervals for each
uncached workload, after `PROFILE_READY`. These optimized builds include debug
symbols. Percentages below are disjoint top-of-stack samples, not inclusive
call-tree percentages; the output omits individual functions with fewer than
five samples. Copies combine `memmove`/`memcpy`; clearing combines
`memset`/`bzero`. `check_tag` includes validation and inlined CRC work, so its
percentage is not an exact CRC percentage.

| Workload | Copy | Clear | Tag validation | Name decoding |
|---|---:|---:|---:|---:|
| Contiguous list | 37.0% | 11.3% | 36.0% | 1.6% |
| AED list | 36.4% | 13.1% | 34.7% | 1.4% |
| Contiguous lookup-last | 46.1% | 32.0% | 0.1% | 6.9% |
| AED scattered reads | 54.8% | 1.2% | 23.7% | 0.0% |

The counted device wrapper also appears in the samples and can include inlined
device work. Profiling and runtime therefore include instrumentation overhead;
these are directional measurements, not a precise cost model of an unwrapped
reader on a physical device. Copy samples alone do not identify each buffer's
contribution. Code inspection identifies `Walk::resume`, which returns a walker
containing a 4 KiB AED buffer, and file-entry movement as candidates to measure
with separate changes.

Isolated CRC medians are 98.6 ns for 52 bytes, 416.9 ns for 168 bytes, 2.748 us
for 1,024 bytes and 5.529 us for 2,048 bytes. Short ASCII decoding takes 9.3 ns;
241-byte ASCII and UTF-16 inputs take 160.7 ns and 192.1 ns respectively.
These kernels compile the existing implementations, but their call context and
inlining may differ from the driver. Native sampling supports prioritizing
buffer handling and validation ahead of name-decoder changes.

The next proposed order is:

1. Initialize/resume the allocation walker in place to reduce movement of its
   4 KiB buffer, then measure whether buffer initialization can be reduced
   safely. Preserve fully initialized Rust storage and cancellation behavior.
2. Reduce continuation replay for backward/scattered reads, potentially with
   an optional bounded position index. Preserve traversal limits and malformed
   chain handling when resuming from learned positions.
3. Evaluate CRC acceleration after the buffer work, including flash-size and
   `no_std` costs. Keep all current CRC validation.
4. Revisit name decoding only if later profiles make it a meaningful cost.

Each production optimization should have its own commit, correctness coverage
and before/after run of this expanded harness. No new cache, reader API or
production driver changes are included in this follow-up.

Follow-up data:

- [Fragmented workloads, uncached](benchmarks/udf-fragmented-uncached.csv)
- [Fragmented workloads, cache 8](benchmarks/udf-fragmented-cache8.csv)
- [Fragmented workloads, cache 32](benchmarks/udf-fragmented-cache32.csv)
- [CRC and name kernels](benchmarks/udf-kernels.csv)
- [Aggregated native top-of-stack samples](benchmarks/udf-profile-samples.csv)


## In-place allocation walker

The first follow-up initializes `Walk` inside `read_stream` and resumes it
through a mutable reference. Previously, `Walk::resume` constructed the walker
and returned it together with the logical position through `Result`; that
return value included its 4 KiB AED buffer. The new return value is only the
logical position. The buffer remains fully initialized, temporary and bounded;
there is no new reader field, allocation or public API.

A fresh paired run uses baseline `6ccda20b` and this change, with the same Rust
1.97.1 toolchain, optimized profile, host, fixtures and 21 samples. Uncached
1,000-entry, 2,048-byte-device median microseconds:

| Workload | Before | After | Reduction |
|---|---:|---:|---:|
| Contiguous list | 2,098.0 | 1,898.8 | 9.5% |
| Contiguous lookup-last | 532.1 | 339.0 | 36.3% |
| Inline-fragmented lookup-last | 517.3 | 343.8 | 33.5% |
| AED lookup-last | 917.0 | 705.1 | 23.1% |
| Contiguous read-4k | 263.9 | 237.0 | 10.2% |
| AED read-4k | 346.0 | 303.8 | 12.2% |
| AED scattered reads | 1,296.1 | 1,274.1 | 1.7% |

Device calls, bytes and AED counts match the baseline exactly across all 108
uncached cases. This is a CPU optimization; the scattered workload's chain
replay remains. Runtime varies between runs, particularly in short cases and
unchanged mount/metadata operations, so small differences are not evidence of
an improvement. An earlier run also showed the larger lookup improvement.

Regression coverage exercises two continuation blocks, forward and backward
reads, reads spanning a continuation boundary, switching files and EOF in sync
and async modes. The async test cancels while reloading each saved AED, then
retries and verifies exact bytes. Existing malformed-chain and traversal-limit
tests remain in place.

- [Fresh uncached baseline](benchmarks/udf-walker-before.csv)
- [In-place walker](benchmarks/udf-walker-after.csv)


Cached runs at capacities 8 and 32 also reproduce all baseline I/O counts,
for 324 checked cases in total. Their CSVs are
[cache 8](benchmarks/udf-walker-cache8.csv) and
[cache 32](benchmarks/udf-walker-cache32.csv).

A repeat of the native profiles shows contiguous lookup's copy share falling
from 46.1% to 18.7%. Its clearing share rises from 32.0% to 47.9% as total runtime
falls; that percentage increase does not establish increased clearing cost.
Contiguous listing's copy share falls from 37.0% to 30.8%. Scattered AED reads
remain dominated by copying and validation. The same sampling and
instrumentation limits apply to the
[post-change samples](benchmarks/udf-walker-profile.csv). Buffer clearing is a
separate candidate and remains unchanged in this commit.
