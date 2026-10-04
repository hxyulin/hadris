# FAT12 allocation batching

## Prerequisite: interrupted split-entry recovery

FAT12 entries occupy twelve bits in a two-byte window. When that window crosses
an underlying device-block boundary, writing it requires two transfers. Before
this change, interruption after the first transfer could leave a partial link:
for example, allocating cluster 341 as end-of-chain could temporarily encode
cluster 15. Mirroring that partial value and reclaiming the held allocation
could follow an unrelated live file's chain.

The raw FAT state now records the intended value and original allocation status
while writing a split entry in the active FAT. `mirror` completes that entry
before copying secondary FATs. The record survives failed or cancelled repairs,
clears only after the active entry completes, and preserves exactly-once
free-cluster accounting. Recovery is also tracked on volumes with one FAT copy.
This is recovery within the running driver; it does not add a persistent journal
or guarantee recovery following process termination or power loss.

The focused regression fails against the original code with a partial link to
cluster 15. Its independent packed-entry reader checks every data-cluster entry
in every FAT copy, including the neighbors sharing nibbles and unrelated live
clusters 15/255. It exercises allocation, linking and freeing on 512/4096-byte
sector/device-block combinations with one and two FAT copies, injects each
write failure, and cancels both the update and its repair at every await.

Existing hosted async future caps pass unchanged. All three firmware targets
pass the repository's flash/state/stack budgets. The split-entry record adds
8 bytes to the measured bare-metal embedded driver state: the bare-metal four-slot FAT state is
1,048 bytes including the rename-recovery prerequisite. The thumbv7em FAT
logger occupies 41,948 bytes after batching, below its 44 KiB
regression ceiling; the existing 20 KiB target remains tracked separately.

## Packed-entry allocation groups

FAT12 now uses the existing reverse-group allocator for entries whose complete
two-byte windows fit in the same device block in every written FAT copy. The
packed encoder preserves the neighboring nibble while the group is linked in
memory, then writes the active copy before the other FAT. An entry crossing a
block boundary uses the individual path with the split-entry repair above.
Group membership is limited by the existing 2,048-cluster bitmap, even when a
4 KiB device block could hold more FAT12 entries. No allocation bitmap or
additional I/O buffer is allocated.

Single-cluster append also combines allocation and tail linking when both
entries fit in the same block. The raw primitives scan for sufficient space
before changing a multi-cluster FAT12 allocation, matching FAT16/32 behavior.
`Held.head` and `Held.extra` retain their existing recovery meanings. The public
API and feature matrix are unchanged; FAT12 freeing remains per entry.

Measurements used Rust 1.97.1 on Apple M3 Pro, seven samples per case, a 2 MiB
FAT12 volume with 512-byte clusters/device blocks, and disabled hosted caches.
The device is memory-backed: timings measure CPU work, while transfer counts
and byte totals describe device traffic. Baselines were collected from
`dbcb38fe`. Full CSVs are [before](benchmarks/fat12-allocation-before.csv) and
[after](benchmarks/fat12-allocation-after.csv).

| Driver/workload | Reads before → after | Writes before → after | Written bytes before → after | Median CPU before → after |
|---|---:|---:|---:|---:|
| Hosted, append 16 KiB in 512-byte requests | 130 → 68 | 161 → 99 | 82,432 → 50,688 | 6.04 → 5.67 µs |
| Embedded, append 16 KiB in 512-byte requests | 130 → 68 | 161 → 99 | 82,432 → 50,688 | 6.33 → 5.04 µs |
| Hosted, write 1 MiB in 4 KiB requests | 8,257 → 581 | 8,465 → 799 | 5,251,584 → 1,326,592 | 333.92 → 141.00 µs |
| Embedded, write 1 MiB in 4 KiB requests | 8,256 → 1,086 | 8,465 → 1,305 | 5,251,584 → 1,585,664 | 354.50 → 149.50 µs |
| Hosted, write 1 MiB in 64 KiB requests | 8,257 → 101 | 8,225 → 79 | 5,251,584 → 1,080,832 | 310.63 → 87.50 µs |
| Embedded, write 1 MiB in 64 KiB requests | 8,256 → 130 | 8,225 → 109 | 5,251,584 → 1,096,192 | 310.21 → 83.58 µs |

The hosted 64 KiB workload reduces writes by 99.0%, bringing write amplification
from 5.008 to 1.031. Embedded multi-cluster growth still links its old tail
separately, accounting for the difference between hosted and embedded traffic.
The allocator's selection scan adds upfront work for short requests; workload
measurements should accompany any future policy changes.

Tests independently inspect every FAT entry after recovering from each failed
allocation write and each cancelled await, including boundaries in differently
aligned FAT copies. Fragmented/wrapped/full-volume allocations preserve live
entries and neighboring nibbles. Hosted append tests cover first candidates
immediately before, at and after odd/even boundaries, 512/4096-byte sector and
block geometries, and caches disabled/enabled. The counter regression requires
three writes for a four-cluster append sharing a FAT12 block (two FAT writes
and one payload write). All three firmware targets pass their budgets; the
thumbv7em logger uses 41,948 bytes on the final ancestry, including rename
recovery, with no driver-state increase from batching.

The broader embedded cancellation workload exposed an existing rename recovery
gap once faster allocation shifted await timing: cancellation after publishing
the destination but before clearing the source could leave two names owning
one chain. This branch includes the separate [rename-recovery prerequisite
(PR #241)](https://github.com/hxyulin/hadris/pull/241).

Final validation on that ancestry passes all 351 FAT/raw tests and doctests,
including the unchanged randomized embedded cancellation workload. The older
interrupted-operation oracle now requires a clean post-sync image for every
FAT12/FAT32 operation, removing its obsolete torn-entry and rename exemptions.
All 126 benchmark smoke cases retain their measured call/byte/flush counts
following the prerequisite rebase; 40 existing FAT16/32 hosted workloads retain
their original counts. Conformance passes nine tests (four optional peer/native
tests remain ignored), all 15 MSRV feature tiers pass, and formatting, clippy,
warning-denied workspace checking and all three firmware budgets pass without
raising limits. Public API snapshots and sync/async/local parity remain
unchanged.
