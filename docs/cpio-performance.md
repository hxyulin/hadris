# CPIO streaming audit

The allocation-free reader is already a suitable architecture for pipes and
initramfs. The audit keeps stream ownership, caller-supplied name storage,
checksum verification on skipped data, and concatenated-segment semantics.

A correctness regression found that `read_tree` tracked hard-link owners by
encoded path spelling while `Tree` ignores repeated path separators. Replacing
`dir//file` through `dir/file` left a stale owner; a later group member could
overwrite the replacement. Ownership now uses the same separator normalization.
Ordinary paths remain borrowed. Sync and async regressions cover replacements
and repeated names.

## Harness

```console
cargo bench -p hadris-cpio --bench performance -- --samples 21 --csv
cargo bench -p hadris-cpio --bench performance -- --smoke
HADRIS_REQUIRE_EXTERNAL_TOOLS=1 cargo test -p hadris-cpio --all-features --test interop
```

There are 144 cases: `newc`, checksum `newc` and `odc`; reading payloads,
skipping payloads, loading a `Tree` and writing an archive; direct streams and
8 KiB caller buffering; and six fixtures:

- 256 independent files of 17 bytes.
- A 4 MiB memory-backed file.
- 256 hard links sharing 4,097 bytes, with the payload on the last name.
- 128 directories and 128 symlinks.
- A 17-byte host file.
- A 4 MiB host file.

Counters wrap the stream below caller buffering. They count successful backend
calls, transferred bytes, flushes and maximum request sizes; an EOF read counts
as a call. Fixture construction and caller buffer allocation are outside timing.
Writing includes planning, writer scratch allocation, content opening and
copying, output writes, and flush. Output storage is preallocated. Tree loading
includes payload allocation and hard-link reconstruction; tree verification and
destruction are outside timing. Streamed host inputs use temporary files and a
warm filesystem cache; timings do not represent physical disk latency.

Writer cases compare complete archive bytes, tree-loading cases compare restored
nodes and hard links, and read/skip cases check entry counts before measuring.
Every case requires identical counters across samples. The smoke run repeats
these checks in each measured sample. Native `cpio` independently extracts Hadris `newc`, checksum
`newc` and `odc` archives, checking payloads, empty files, hard links and symlinks
on Unix. Hadris also reads native `newc` and `odc` archives. Missing tools skip
locally unless `HADRIS_REQUIRE_EXTERNAL_TOOLS` is set.

The baseline CSV is `benchmarks/cpio-streaming-before.csv`, measured with
Rust 1.97.1 on an Apple M3 Pro, with 21 samples per case.

## Findings and boundaries

Caller buffering already reduces the 256-small-file `newc` read workload from
1,541 backend calls to five. Increasing reader stack buffers or introducing
internal read-ahead is not justified by that workload. The reader currently
drains skipped data in 512-byte chunks; a buffered 4 MiB skip takes 513 backend
reads instead of 8,202 direct reads. Pipe compatibility and exact stream recovery
remain useful constraints.

The writer allocates a 64 KiB scratch buffer even for directories and symlinks,
and copies memory-backed file data through it. This is a concrete opportunity
to borrow resident data and allocate scratch only for streamed content. Header
names also emit their NUL and alignment padding separately; these adjacent zero
bytes can be emitted together without changing archive bytes.

`read_tree` loads file payloads into memory and tracks names to reconstruct hard
links in archive order. This is intentional for its owned `Tree` return value;
applications that need bounded payload memory should use `CpioReader`. Replacing
it with a lazy representation would change stream ownership and deserves a
separate API proposal. Large hard-link groups and payload allocation remain
future profiling candidates.

Checksum archives place the payload checksum in the header. Streamed content
therefore needs a preliminary checksum pass; comparisons must account for that
format requirement. This audit does not introduce cancellation recovery or
persistent state for an interrupted writer.

## Borrowed resident payloads

Memory-backed files now use the same borrowed-data path as symlinks. CRC sums
are computed directly over resident bytes. The writer opens nonresident content
as before, allocating reusable scratch only when it has payload to stream. A
fresh writer's scratch length is `min(content length, 64 KiB)` for streamed data
and zero for resident data, directories and symlinks. Scratch is retained for
reuse rather than shrunk between entries. This is a bound from the implementation,
not a measurement of total allocator overhead or retained report memory.

The 21-sample `newc` 4 MiB resident-file workload falls from 72 to nine direct
backend writes and from 132.08 to 59.25 microseconds. Host-streamed 4 MiB writes
retain 72 calls and their 64 KiB maximum request. Resident output may now request
the whole payload; `write_all` still handles short backend writes. Sync and async
regressions use seven-byte writes and compare complete archive bytes in every
writable format.

## Combined results

`benchmarks/cpio-streaming-after.csv` records the final 21-sample run on the same
machine and compiler. Terminators and name padding now use one write for every
alignment. Directory-only regressions require six direct writes for an entry
plus trailer, check all eight name lengths in all writable formats, and repeat
with one-byte short writes.

| `newc` writing workload | Caller buffer | Writes before → after | Median time before → after |
|---|---:|---:|---:|
| 4 MiB resident file | none | 72 → 7 | 132.08 → 65.67 µs |
| 4 MiB resident file | 8 KiB | 66 → 3 | 137.04 → 66.67 µs |
| 17-byte host file | none | 10 → 8 | 10.71 → 9.83 µs |
| 4 MiB host file | none | 72 → 70 | 182.79 → 179.88 µs |
| 256 small resident files | none | 1,284 → 1,283 | 64.88 → 63.21 µs |
| 256 hard links | none | 774 → 773 | 57.21 → 59.58 µs |

The uniform small-file names already need no name padding, so that fixture saves
only the trailer's extra call. The hard-link timing is slightly worse in this
run despite fewer calls; this is not evidence of a broad throughput improvement.
The substantial gain is the resident bulk-file path. Host-streamed input remains
chunked, and caller buffering already coalesces most metadata writes.

All cases preserve transferred archive bytes and flush counts. Read, skip and
tree-loading cases retain their read calls, transferred bytes and request sizes.
The reader has no new buffer, allocator dependency or eager index. No public API
or feature changes are needed for these optimizations.
