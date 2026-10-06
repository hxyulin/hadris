# FAT directory-prefix indexing

The optional FAT node-driver index caches parsed names from the prefix of one
hot directory. It addresses repeated directory traversal and LFN assembly,
which remain CPU costs even when metadata blocks fit in the existing cache.
Normal mounting and `CacheOptions::new()` leave this index disabled.

```rust,ignore
let fs = FatFs::mount(device, options)?
    .with_cache(CacheOptions::new()
        .with_directory_entries(128)
        .with_blocks(64));
```

A lookup scans only as far as its first matching entry. The index saves each
validated visible entry encountered, its original LFN code units, its short
entry, and folded-name hashes. Hashes only narrow candidates: the existing
name matcher verifies every candidate and retains exact-case information for
rename. Short aliases remain usable. The index saves the original `DirWalk`
and slot after its last entry, retaining cycle detection across resumed scans.
A later miss resumes beyond that prefix without parsing it again. There is no
negative-result cache and no upfront full-directory scan.

The entry bound limits the prefix; overflow scans its suffix normally. Changing
directories replaces the prefix. Mutation preparation, directory insert/clear,
metadata writes (including dirty-node flushes), and interrupted-operation
recovery invalidate it before writes. An interrupted lookup only leaves fully
validated entries and their corresponding cursor. It cannot skip an unfinished
LFN sequence. `clear_cache()` discards names while retaining vector storage.

Fixed entry storage is reserved when indexing is explicitly configured. Each
entry can additionally own at most 510 bytes of LFN code units. The index adds
one optional heap pointer to driver state (4488 bytes excluding the device on
this 64-bit host, up eight bytes); allocator-free embedded drivers and
existing async future-size limits are unchanged. It supports `alloc` without
`std`, with sync, async and local I/O modes.

## Measurements

These are 21-sample memory-device benchmarks on the Apple M3 Pro using Rust
1.97.1 release builds. The workload looks up 128 different entries in order,
including metadata retrieval and pin release. Every sample starts from a fresh
mount; construction and image validation are outside the operation timer.
Metadata caching is fixed at 64 blocks and chain-position caching at zero.
Thus these measurements include prefix construction and its amortized benefit,
not merely warm hits. Timings describe CPU work on a memory device, not SSD
latency. Device-call counts are deterministic and identical across settings.

| Variant / workload | Index disabled | 16 entries | 128 entries | Reads |
|---|---:|---:|---:|---:|
| FAT12, 128 long names | 696.75 µs | 570.54 µs | 79.46 µs | 32 |
| FAT12, 128 short names | 237.50 µs | 198.21 µs | 44.50 µs | 8 |
| FAT16, 128 long names | 689.83 µs | 591.96 µs | 80.29 µs | 32 |
| FAT16, 128 short names | 235.75 µs | 200.46 µs | 37.00 µs | 8 |
| FAT32, 128 long names | 916.17 µs | 756.08 µs | 85.96 µs | 33 |
| FAT32, 128 short names | 283.58 µs | 241.71 µs | 47.50 µs | 9 |

FAT32 median CPU time falls by about 91% for long names and 83% for short
names at a 128-entry bound. A smaller bound provides a smaller benefit because
lookups beyond its prefix still scan the suffix. A nested boot path switches
directories, so this one-directory index does not retain all its components.
The 21-sample cold EFI boot-load check retained exactly 27/26/39 device reads
for FAT12/16/32 with the index enabled or disabled. Median CPU time changed
31.67→35.21 µs, 24.42→24.38 µs and 34.71→35.88 µs respectively. The index
provided no boot-path I/O benefit and can add CPU/allocation work. Keep it
disabled for single-path boot loading. CSV inputs are [boot disabled](benchmarks/fat-directory-index-boot-disabled.csv)
and [boot indexed](benchmarks/fat-directory-index-boot-128.csv).

CSV inputs: [disabled](benchmarks/fat-directory-index-disabled.csv),
[16 entries](benchmarks/fat-directory-index-16.csv),
[128 entries](benchmarks/fat-directory-index-128.csv).

```sh
cargo +1.97.1 bench -p hadris-fat --bench performance -- \
  --filter hosted/lookup --metadata-blocks 64 --directory-entries 128 \
  --samples 21 --csv
```

## Verification

Regression tests cover all five FAT test geometries, bounds 0/1/4/48, lowercase
BMP and supplementary names, short aliases, warmed hits, missing names,
directory switching, dirty pins, create/rename/unlink and slot reuse. Warmed
lookups whose directory fits the index require zero device reads with metadata
caching disabled. A FAT32 directory cycle still reports corruption on repeated
resumed missing-name scans with bounds 1/48/128. Async lookups are cancelled at
every await before retrying every name. A separate cache invalidation regression
cancels renames of an empty file at every await, compares cached and cleared
lookup results, and checks the resulting image with the raw oracle. Existing
mutation failure/cancellation and hosted async resource tests also pass.

The new short-name hashing conversion has targeted Miri coverage for ASCII
case flags, CP437, ASCII escape decoding and the escaped first-byte form.
The decoder regression asserts ASCII bypasses the OEM decoder. Public API
snapshots and sync/async/local parity include the new cache builder.

## Pre-existing cancellation finding

The additional cancellation audit found that renaming a nonempty file can
leave both old and new entries owning the same cluster if cancelled after the
new entry is written. This reproduces with directory indexing disabled. It is
retained as an explicitly ignored regression, with the raw oracle still
asserting that no cross-link may remain:

```sh
cargo +1.97.1 test -p hadris-fat --features async --test fatfs_cache \
  cancelled_nonempty_rename_can_leave_a_cross_link_without_directory_indexing \
  -- --ignored
```

The minimal case mounts the FAT32 fixture through `YieldDev`, warms a lookup
of `OLD.TXT`, cancels its rename to `A replacement long name.txt` at await
budget five, syncs and unmounts. The oracle reports both entries owning cluster
three. This transaction-recovery bug requires a separate correctness change;
the index neither causes nor repairs it.
