# FAT bootloader and hosted performance audit

The allocation-backed `FatFs` is shared by `alloc` + `no_std` bootloaders and
`std` applications. There is no separate bootloader driver. The allocator-free
`embedded::Fat` remains useful for bootloaders too; this audit focuses on the
node API and uses the embedded API as a comparison. FAT12/16/32 are covered;
exFAT and boot-media image construction are separate follow-ups.

## Baseline

Driver revision: `d6146394e539c1064d6e138e7e507b89b24d3167` (`next`).
Host: Apple M3 Pro, `aarch64-apple-darwin`. Compiler: Rust 1.97.1
(`8bab26f4f`, LLVM 22.1.6). Seven measured release samples per case, plus a
checked warm-up. Default features (`std,sync,write`), no tracing.
The expanded harness runs 120 cases. The original 78 cases retain identical
successful I/O counts, bytes, flush counts and maximum request sizes.

```sh
RUSTUP_TOOLCHAIN=1.97.1 cargo bench -p hadris-fat --bench performance -- --samples 7 --csv
```

[Full CSV](benchmarks/fat-hosted-baseline.csv) includes every FAT kind and both
drivers, timing ranges, geometry and write-region attribution.
[Methodology](performance.md) defines the workload boundaries and checks.
These are memory-device timings, including counters; they describe driver CPU
cost and device requests, rather than physical SSD or SD-card latency.

| FAT32 hosted workload | Median µs | Reads | Writes | Read bytes | Write bytes |
|---|---:|---:|---:|---:|---:|
| `mount` | 0.92 | 3 | 0 | 1,536 | 0 |
| `boot-load-64k` | 32.67 | 39 | 0 | 1,060,352 | 0 |
| `read-4k` | 33.92 | 274 | 0 | 1,057,792 | 0 |
| `read-64k` | 31.38 | 34 | 0 | 1,057,792 | 0 |
| `read-unaligned` | 44.21 | 1,043 | 0 | 1,319,936 | 0 |
| `read-reverse-4k` | 2,249.46 | 2,435 | 0 | 2,164,224 | 0 |
| `read-shuffled-4k` | 1,127.92 | 1,298 | 0 | 1,582,080 | 0 |
| `read-fragmented-64k` | 63.29 | 2,083 | 0 | 1,066,496 | 0 |
| `lookup-128` | 517.25 | 1,008 | 0 | 516,096 | 0 |
| `list-128` | 19.04 | 16 | 0 | 8,192 | 0 |
| `lookup-long-128` | 904.25 | 4,092 | 0 | 2,095,104 | 0 |
| `list-long-128` | 28.62 | 64 | 0 | 32,768 | 0 |
| `append-512-end-sync` | 6.21 | 131 | 162 | 67,072 | 82,944 |
| `write-4k` | 1,698.79 | 1,092 | 1,314 | 559,104 | 1,590,272 |
| `write-64k` | 262.17 | 132 | 114 | 67,584 | 1,098,752 |

## Findings

### Mount and contiguous boot loading are already lazy

Mount reads one sector on FAT12, two on FAT16 and three on FAT32. It neither
scans every directory nor builds a whole-volume cluster index. Loading the
1 MiB `EFI/BOOT/BOOTAA64.EFI` file, including read-only mount, three component
lookups, reading and close, takes 39 device reads on FAT32. A root-file 64 KiB
read workload takes 34 reads with mount excluded. FAT32 returns 1 MiB with
only 9 KiB of additional bytes read in that root-file workload.

`FatFs::read` already coalesces physically consecutive clusters through
`rawio::run`. Increasing a caller's chunk from 4 KiB to 64 KiB reduces data
requests from 256 to 16. There is no reason to eagerly read the entire FAT or
preload a boot directory tree for this use case.

### Backward seeks repeatedly walk the chain

`Node::hint` stores one `ChainPos`. `rawio::walk` can reuse it only when the
requested cluster index is at or after that hint; backwards requests restart
at the first cluster. The reverse workload takes about 66 times the forward
4 KiB workload's CPU time on FAT32. Shuffled reads show the same problem.
A sector cache could reduce repeated device reads, but would still decode
and follow the repeated links. A bounded index of validated chain positions
or extents should address both costs.

Such an index should be built lazily per file, usable without `std`, and
optional or bounded for bootloaders. Invalidation on growth, truncation and
interrupted-operation recovery must be explicit; cached traversal must retain
chain corruption detection. A full chain index costs memory proportional to
file size, and should not be an automatic mount-time operation.

### Partial data reads evict FAT metadata

4093-byte reads take 1,043 device calls versus 274 for 4096-byte reads on the
same FAT32 file. Data edges use the same single `BlockBuf` as FAT entries and
directory slots; aligned full-block transfers bypass it. Alternating between
partial data and FAT reads loses cached sectors and rereads payload edges.
Separating metadata and partial-data buffering, or adding a bounded metadata
sector cache, is a plausible follow-up. Measure both memory growth and write
invalidation before adopting it.

### Fragmentation places a real limit on transfer coalescing

The fragmented FAT32 chain needs 2,083 reads with a 64 KiB application buffer,
versus 34 for the contiguous file. The fixture keeps the same allocated
clusters but orders even-indexed physical clusters before odd-indexed ones;
contents are relocated and both FAT copies updated. Requests cannot coalesce
across those physical gaps. Metadata caching may reduce the extra FAT reads,
but cannot turn the fragmented payload into 16 contiguous device requests.

### Repeated lookup rescans and converts names

`find_entry` starts at directory slot zero on each lookup, and `names::matches`
rechecks and decodes the short name for each candidate. Looking up and getting
metadata for all 128 short-name files takes 1,008 reads; listing once takes 16.
Long-name lookup takes 4,092 reads versus 64 for listing. The aggregate scans
are quadratic in the number of entries when every name is looked up.

Prepare the query once per lookup before considering a directory index.
This can reduce repeated UTF-16 iteration, short-name display conversion and
length checks without persistent caching. A later bounded directory/name
cache should preserve Unicode folding, short-name aliases, runtime code pages,
pinned-node identity and mutation visibility. Listing already returns metadata
from the entry it just parsed and does not need the ISO metadata-reuse fix.

### Hosted single-cluster growth misses an existing embedded optimization

`FatFs::cover` allocates a new chain and then separately links the old tail.
The embedded driver uses `rawio::allocate_after` for single-cluster extension,
combining both FAT-entry updates when they share a device block on FAT16/32.
For the FAT32 16 KiB log in 512-byte writes, hosted writes 63 sectors to each
FAT copy; embedded writes 32. Total calls are 162 versus 100, respectively.
Porting that path offers a concrete, bounded first change without a new API.

The embedded timing is slower here despite fewer device calls. That reinforces
measuring CPU and I/O separately rather than treating either as a proxy for
the other.

### Write batching has a large effect

Writing 1 MiB in 4 KiB calls needs 1,314 writes and about 1.70 ms on FAT32;
64 KiB calls need 114 writes and about 0.26 ms. FAT-copy writes account for
539,648 bytes versus 48,128 bytes. Most of the difference is repeated small
allocation/link updates, rather than payload transfer. Extending allocation
and tail-link batching is useful after the single-cluster path is proven.
FAT12 retains a separate per-entry allocation path: its 1 MiB write amplification
is about 5 times even with 64 KiB calls. FAT12 packing and mirrored-write
recovery need a dedicated change rather than applying the FAT16/32 algorithm
blindly.

## Memory and proposed order

On this host, driver state excluding the device is 4,400 bytes for `FatFs`
versus 960 bytes for embedded `Fat<_, 4>`. `FatFs` includes a 4,096-byte inline
buffer and a heap-backed node table. `readdir` returns unpinned IDs, so listing
does not allocate a table entry for every file. Heap allocation counts and
peak stack usage were not measured; the driver-state measurement is not total
memory consumption.

Keep optimizations in separate commits:

1. Reuse `allocate_after` for hosted single-cluster growth, with failed-write
   and async cancellation coverage at every affected write/await boundary.
2. Prepare lookup queries once, keeping the current name and code-page rules.
3. Add an optional bounded per-file chain-position or extent index for seeks,
   built as needed instead of at mount.
4. Evaluate separate partial-data buffering or an optional metadata-sector
   cache against unaligned reads and repeated directory access.
5. Extend multi-cluster allocation/tail-link batching; audit FAT12 separately.

Bootloaders that only load contiguous files can retain the present small,
lazy mount path and use large aligned read buffers. Hosted workloads with
random reads and repeated name lookups have more to gain from configurable
caches. API design for those caches should follow measured use cases and
preserve `alloc` + `no_std` and both I/O modes.

## Validation

All 120 warm-ups verify disk structure and fresh-mount contents/counts, including
relocated fragmented payloads and every reverse/shuffled read. All seven
samples per case reproduce the warm-up I/O counts. The FAT sync/async crate
tests, workspace warning check, no-default `alloc,sync,async,write` tier and
format check pass. No driver behavior, public API or unsafe code changed in
this audit. Async scheduling, real-device timings, large-directory scaling,
non-ASCII lookup performance, boot configuration parsing, initrd loading and
heap allocation counts remain unmeasured.
