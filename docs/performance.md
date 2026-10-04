# Performance measurements

Start with device I/O counts when optimizing write amplification. Runtime,
driver state, heap use, stack use and flash size are separate measurements;
an improvement in one can cost more in another.

## FAT benchmark

The harness needs no new dependencies and uses Cargo's optimized bench
profile on Rust 1.88 or later:

```bash
cargo bench -p hadris-fat --bench performance
cargo bench -p hadris-fat --bench performance -- --filter fat32/embedded/append
cargo bench -p hadris-fat --bench performance -- --samples 21 --csv > baseline.csv
cargo bench -p hadris-fat --bench performance -- --smoke
```

`--filter` selects a substring of the case name
`<fat12|fat16|fat32>/<hosted|embedded>/<workload>`. A filter matching no cases
fails. `--samples` defaults to seven; `--smoke` uses one measured sample per
case. Each case also gets a checked warm-up. `--help` prints the options.
CSV goes to stdout, so Cargo's build messages on stderr do not enter the file.

The 78 cases combine three variants, two synchronous drivers and these
workloads:

| Workload | Operations |
|----------|------------|
| `mount` | Mount an empty formatted image |
| `append-64-end-sync` | Create a log, append 16 KiB in 64-byte writes, close, sync |
| `append-64-flush-each` | The same log, flushing the file after every write, then close and sync |
| `append-512-end-sync` | The same log in 512-byte writes, close, sync |
| `append-4k-end-sync` | The same log in 4 KiB writes, close, sync |
| `write-4k`, `write-64k` | Create and sequentially write 1 MiB, close, sync |
| `read-4k`, `read-64k` | Open and sequentially read a contiguous 1 MiB file, close |
| `create-remove-short`, `create-remove-long` | Create 64 files, write 64 bytes each, close, remove all, sync |
| `lookup-128` | Look up and retrieve metadata for each of 128 short-name files |
| `list-128` | Enumerate a directory containing 128 short-name files once |

Images are 2 MiB for FAT12, 16 MiB for FAT16 and 64 MiB for FAT32. All use
512-byte device blocks, two FAT copies, a fixed serial number and the
formatter's default cluster size. CSV includes volume, block and cluster
sizes because geometry affects I/O costs. The hosted driver uses its default
mount options; the embedded driver uses four file slots and its default
ASCII name fold. Workload names use ASCII so both compare the same names.
Hosted directory iteration calls `readdir` once per entry; embedded iteration
uses its callback API. These measurements include each API's own costs.

## What the numbers mean

- Read and write counts are successful `BlockDevice` calls, with byte totals
  and maximum request sizes reported separately. One call can transfer many
  blocks. Rewriting the same block counts again.
- CSV also attributes written bytes to each FAT copy, the root directory,
  file data, FSInfo and other reserved blocks. A diagnostic warm-up records
  per-block writes, then classifies them using the final image, so root
  directory growth is included. These workloads use only the root directory.
  Classification and per-block recording are outside measured samples;
  samples retain only the aggregate call counters. Region totals must equal
  the aggregate written bytes.
- Flush counts are successful calls to the device's `flush`. The memory
  device does no physical work on flush, but the count exposes how often a
  real device would be asked to flush.
- Write amplification is device bytes written divided by application payload
  bytes written. It includes directory updates, FAT copies and zero-filling.
  The create/remove workloads divide by all bytes written before deletion.
  Mount, reads, lookup and listing report `n/a`.
- Times cover the workload, including open/create, close and the final sync
  for writing cases. The mount case times only mount. For all other cases,
  mount and its I/O are excluded; the driver's block buffer starts in its
  post-mount state.
- Formatting, fixture population, image cloning, payload and read-buffer
  allocation, checking, readback and driver destruction are outside the
  measured interval. Every sample starts from a fresh copy of the fixture.
- Every warm-up checks the resulting disk structure with `hadris-fat::sync::check`
  and verifies file size/content or directory entry count on a fresh mount.
  Read warm-ups also verify the bytes returned by the measured driver. Later
  samples must reproduce the warm-up's I/O counts exactly.

Timing is instrumented, in-memory CPU time. The counter wrapper adds work
to every device call, and the memory backing remains subject to host CPU
caches. Very short cases, especially mount, are sensitive to timer noise.
CSV reports minimum, median (upper middle sample for even counts) and maximum
nanoseconds to expose variation. Do not interpret these timings as SD-card
latency, flash wear, or throughput on an embedded target. This first harness
does not measure async scheduling, fragmented files, exFAT or heap allocation
counts.

Record the Git revision, `rustc -Vv`, host and command with each baseline.
Use identical toolchains, features, geometry and workloads for before/after
comparisons. Compare I/O counts first and repeated timing runs second. Run
the affected conformance and cancellation tests before accepting an
optimization: metadata ordering and interrupted-write recovery constrain
which writes can be combined or removed.

## Function tracing

See the [tracing guide](tracing.md) for the opt-in hosted instrumentation,
subscriber setup and feature boundaries. The benchmark counters work
independently of tracing.

## Embedded state, stack and flash

The human-readable benchmark output prints host `size_of` values for the
hosted driver and embedded drivers with 1, 4 and 16 slots, subtracting the
device's size. These figures include layout padding and exclude heap
allocations, stack frames and the backing image. They are useful for local
comparisons, but are not embedded-target RAM estimates.

Use the existing firmware harness for target-specific state, stack and flash:

```bash
RUSTUP_TOOLCHAIN=nightly-2026-09-04 scripts/firmware-size.py --check

# Focus on one target during iteration.
RUSTUP_TOOLCHAIN=nightly-2026-09-04 scripts/firmware-size.py --check thumbv7em-none-eabihf
```

The pinned nightly needs `llvm-tools` and the selected Rust targets. See
[`CONTRIBUTING.md`](../CONTRIBUTING.md#build-and-test) for installation.
The script builds the firmware examples with size optimization and fat LTO,
then reports flash, driver state, mount stack, worst-case stack and the largest
Hadris frame. Its indirect-call and compiler-builtins limits are explained in
the script. Keep these measurements alongside the I/O baseline when weighing
an extra cache, buffer or lookup index.

## First embedded FAT optimization

The first pass combines allocation of one cluster and its previous tail link
on FAT16/32 when both entries share a device block in every FAT copy written.
It uses the existing block buffer, writes the active FAT first and records
the new cluster before writing. Recovery mirrors interrupted updates, detaches
the old tail if linked and releases the new cluster. FAT12 and entries in
different blocks keep allocation followed by a separate tail update. Multi-
cluster allocation and hosted driver behavior are unchanged.

For the 16 KiB log workloads above, measured with Rust 1.88.0 on an Apple
Silicon host, the deterministic device counts were:

| Embedded workload | Before writes | After writes | Before bytes | After bytes |
|-------------------|--------------:|-------------:|-------------:|------------:|
| FAT16, 64-byte appends, final sync | 321 | 291 | 164352 | 148992 |
| FAT16, 512-byte appends, final sync | 97 | 67 | 49664 | 34304 |
| FAT32, 64-byte appends, final sync | 386 | 324 | 197632 | 165888 |
| FAT32, 64-byte appends, flush each | 641 | 579 | 328192 | 296448 |
| FAT32, 512-byte appends, final sync | 162 | 100 | 82944 | 51200 |

FAT32's 512-byte case reduces writes by 38.3% and write amplification from
5.062 to 3.125. Each FAT copy drops from 32256 to 16384 written bytes.
Directory, data and FSInfo writes are unchanged. Across all 78 cases, only
the three single-cluster append workloads on embedded FAT16/32 changed I/O
counts; no case increased them.

The small-write costs that remain have different causes. In the FAT32
64-byte final-sync case, 131072 of the 165888 written bytes are file data:
each 64-byte write still writes a 512-byte device block. Flushing after every
append additionally writes the directory entry each time. Buffering could
reduce this traffic, but would require RAM and change when bytes reach the
device; this pass retains write-through data and existing flush semantics.

With the pinned firmware toolchain on `thumbv7em-none-eabihf`, the logger
uses 40648 flash bytes versus 40012 before, an increase of 636 bytes (1.6%).
Its driver state remains 936 bytes, mount stack 1880 bytes and reported worst
stack 4408 bytes. The async FAT example grows by 1504 flash bytes and 72
reported worst-stack bytes. These are target/compiler measurements, not
universal size guarantees. The firmware resource checks pass.

Baselines were compared against the embedded implementation before write
coalescing, using the same counter harness, toolchain, fixtures and seven
samples per case. Runtime is host-dependent; device counts and target resource
costs are the acceptance evidence for this change.

## Embedded arithmetic pass

The next pass reduces arithmetic work in file and directory addressing. Boot
parsing computes the validated cluster-size exponent once;
`Geometry::cluster_shift()` exposes it in every feature tier. File positions
and sizes use their existing 32-bit FAT bounds for shifts, masks and cluster
rounding. Directory indexing uses the same exponent. FAT-copy selection
toggles between the at-most-two copies, and allocation scans wrap with a
comparison instead of modulo. No buffering or I/O ordering changes are involved.

Relative to the first pass, with the same pinned firmware toolchain:

| Target | Logger flash before | After | Saved | Async FAT flash before | After | Saved |
|--------|--------------------:|------:|------:|-----------------------:|------:|------:|
| `thumbv6m-none-eabi` | 41064 | 40960 | 104 | 70296 | 69820 | 476 |
| `thumbv7em-none-eabihf` | 40648 | 40520 | 128 | 65436 | 65312 | 124 |
| `riscv32imc-unknown-none-elf` | 47512 | 47450 | 62 | 75184 | 75096 | 88 |

These are modest flash reductions. Driver state remains 936 bytes on all
three targets, with no allocator requirement. Stack changes are small: Cortex-
M0 mount stack increases by 8 bytes, its async worst stack by 8 bytes, and its
largest frame in the full FAT example by 8 bytes. Cortex-M4 worst stack in
the full FAT example increases by 8 bytes. The RISC-V logger's reported worst
stack decreases by 16 bytes. All firmware resource checks pass.

Disassembly confirms that the embedded file read and fill functions no longer
call 64-bit division helpers directly. Such helpers remain elsewhere, including
timestamp conversion and raw block I/O: device block sizes may be arbitrary
nonzero values, unlike FAT cluster sizes. Reduced helper calls do not establish
a measured target-cycle speedup; the harness still measures host CPU time.

All 78 benchmark cases retain identical I/O counts, bytes, flushes, maximum
request sizes and write-region totals. Regression tests cover every supported
cluster size, all valid sector/cluster combinations, the maximum FAT file
length, fragmented allocation wraparound, full volumes and one or two FAT
copies. Existing sync/async and interruption-recovery tests remain the gates
for accepting the change.

## Bounded FAT timestamp conversion

FAT stores dates from 1980 through 2107. Encoding first applies the requested
or recorded time zone and clamps local timestamps outside that range. Seconds
relative to 1980 fit in `u32` throughout the range, so calendar and time-of-day
conversion use 32-bit arithmetic. Precision and time-zone behavior remain the
same, including falling back to the recorded zone when an explicit zone is
invalid. Decoding is unchanged.

Disassembly confirms that the logger no longer contains signed 64-bit division
helpers on any of the three targets. Unsigned 64-bit division remains in raw
block addressing and other device-offset calculations.

Compared with the arithmetic pass above, using the same pinned firmware toolchain
and settings:

| Target | Logger flash before | After | Saved | Async FAT flash before | After | Saved |
|--------|--------------------:|------:|------:|-----------------------:|------:|------:|
| `thumbv6m-none-eabi` | 40960 | 40648 | 312 | 69820 | 69508 | 312 |
| `thumbv7em-none-eabihf` | 40520 | 40276 | 244 | 65312 | 65068 | 244 |
| `riscv32imc-unknown-none-elf` | 47450 | 46974 | 476 | 75096 | 74620 | 476 |

The full FAT and Unicode FAT examples save the same number of bytes on each
target. Driver state remains 936 bytes, static RAM remains zero, and all
reported mount, worst-stack and largest-frame sizes are unchanged from the
arithmetic pass. The exFAT examples are unchanged. All firmware resource
checks pass. The Cortex-M4 logger remains 264 bytes larger than the 40012-byte
baseline before the write-coalescing pass, so the combined changes reduce
write amplification at a small net flash cost.

All 78 benchmark cases retain identical device counters and write-region
totals against the arithmetic pass, with seven samples per case. Regression
tests compare the bounded encoder with general `DateTime::to_civil()` conversion on every
FAT date, at several time-of-day and sub-second boundaries. They also check
every valid UTC offset at both date-range limits, recorded and overridden
zones, invalid-zone fallback and the extreme supported `DateTime` values.
No embedded-cycle speedup is inferred from the flash reduction.

## Raw block addressing

The shared block primitives compute the initial quotient and remainder once
per nonempty byte-range operation. Reads advance the output slice and block
index; writes advance the input slice, remaining byte count and block index.
After a leading partial block, subsequent transfers start at block boundaries.
Bulk data and zero writes share one device-write await, reducing duplicated
async transfer code. Partial writes still read, patch and write
through the caller's buffer. Empty operations return before address arithmetic.

Device block sizes remain arbitrary nonzero values that fit the buffer,
including non-power-of-two sizes. No block-size exponent, new buffer fields,
heap allocation or new runtime dependency is introduced. Device requests and
cache invalidation retain their previous ordering. Disassembly of the logger
on each target confirms that the unsigned 64-bit division call in `read_bytes`
and `put` moved out of the transfer loop. Initial offset division and division
in other raw primitives still need the unsigned helpers; target cycles have
not been measured.

Compared with the timestamp pass above, using the same pinned firmware toolchain
and settings:

| Target | Logger flash before | After | Saved | Async FAT flash before | After | Saved |
|--------|--------------------:|------:|------:|-----------------------:|------:|------:|
| `thumbv6m-none-eabi` | 40648 | 40588 | 60 | 69508 | 69416 | 92 |
| `thumbv7em-none-eabihf` | 40276 | 40236 | 40 | 65068 | 64904 | 164 |
| `riscv32imc-unknown-none-elf` | 46974 | 46914 | 60 | 74620 | 74328 | 292 |

The full FAT and Unicode FAT examples save the same amount as the logger on
each target. The exFAT example saves 96, 68 and 50 bytes respectively. Static
RAM remains zero, FAT driver state remains 936 bytes and exFAT state remains
1016 bytes. FAT mount and reported sync worst-stack sizes decrease by 8 bytes
on both Cortex-M targets and remain unchanged on RISC-V. Reported async FAT
worst stack decreases by 64, 32 and 48 bytes respectively. The Cortex-M0 async
example's largest Hadris frame grows by 8 bytes to 544 bytes; other largest
frames are unchanged. All firmware resource checks pass.

All 78 benchmark cases, with seven samples per case, retain identical device
calls, bytes, flushes, maximum request sizes, write amplification and write-
region totals against the timestamp pass. The Cortex-M4 logger is now 224
bytes larger than the original 40012-byte baseline before write coalescing,
while retaining the earlier metadata-write reductions.

Direct regression tests cover cold and warm caches, aligned and partial
transfers, buffer capacities that are not block multiples, device block sizes
from 1 to 4096 bytes, offsets above 4 GiB and near the `u64` limit, empty
operations and device refusals. They inject failure at each transfer, including
partial device effects, and drop both Send and local async futures before and
after each device transfer. The same tests also pass against the committed
implementation, checking behavior preservation independently of the cursor
refactor.
