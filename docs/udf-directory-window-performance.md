# UDF directory-sector buffering

Measured on 2026-10-06 on the same Mac used for the harness audit, using
Rust 1.88.0 in release mode, file-backed images, 1,000 files and seven samples
per workload. The harness discards a warm-up sample; each measured sample runs
in an isolated process. OS page caching is uncontrolled.

The baseline is correctness commit `355395b1`, combined in a temporary,
unpublished worktree with harness PR #271 (`492ff651`). The resulting temporary
merge is `4a14683624b80736541e74f67e880e6a4324db3b`. The after run uses that
same worktree and harness with only the directory-buffer implementation added
as an uncommitted change; its metadata records this accurately. Neither the
harness commits nor this temporary merge are part of the performance PR.

| Operation | Before median | After median | Reads before → after | Requested bytes before → after |
| --- | ---: | ---: | ---: | ---: |
| list | 4.572 ms | 3.359 ms | 4,100 → 2,029 | 5,175,808 → 4,155,392 |
| lookup-last | 1.589 ms | 0.270 ms | 2,098 → 27 | 1,075,712 → 55,296 |
| lookup-miss | 1.561 ms | 0.266 ms | 2,098 → 27 | 1,075,712 → 55,296 |
| lookup-batch | 782.708 ms | 131.127 ms | 1,050,615 → 14,193 | 539,450,880 → 29,067,264 |
| read-full | 0.005 ms | 0.005 ms | 2 → 2 | 133,120 → 133,120 |

Batch lookup is about 6× faster with 98.6% fewer read calls and 94.6% fewer
requested bytes. Names still require a linear scan for each lookup; this change
does not introduce a name index. Listing still reads the directory file entry
and each returned node's metadata, which limits its improvement. File payload
reads bypass the buffer and retain the same read counts.

The fixed buffer stores 4,096 bytes plus its validity/address key. On this
64-bit target, `UdfFs<MemDevice<&[u8]>>` grows from 1,072 to 5,184 bytes:
exactly 4,112 extra bytes per mount. It makes no heap allocations. One fill reads
the larger of the logical block and device block, bounded by the volume length.
Only directory identifier data uses this buffer; allocation-descriptor walks,
file entries and file data retain their existing I/O paths.

Process RSS includes startup, mount, warm-up and verification, and is not an
incremental allocation measurement. Median batch RSS was 2,576 KiB before and
2,560 KiB after; median listing RSS was 3,568 and 3,584 KiB. These 16 KiB changes
are page-level measurement variation, not evidence of reduced memory usage.
The measured struct size is the reliable cost of this implementation.

The buffer is invalidated before a fill starts and becomes valid only after a
successful read. Regression coverage exercises cancellation after the device
has modified the fill buffer, directory switching, missing names and all device
block sizes from 512 through 4,096 bytes. Existing CRC, padding, tag-location,
embedded-directory and cross-extent tests run with the buffer enabled.

Reproduction with PR #271's harness and each reader revision:

```sh
HADRIS_TESTS_PERF_FILTER=udf \
HADRIS_TESTS_PERF_FILES=1000 \
HADRIS_TESTS_PERF_SAMPLES=7 \
HADRIS_TESTS_PERF_BACKEND=file \
HADRIS_TESTS_REPORT_DIR=/tmp/udf-window-measurement \
cargo +1.88.0 bench --manifest-path tests/Cargo.toml --bench performance
```

The CSVs in `docs/benchmarks/udf-directory-window-*` include all eleven
workloads, individual samples, medians and provenance. Read counts measure
requests at the block-device boundary, not physical disk accesses. This fixture
is a flat directory; it does not establish performance on fragmented or deeply
nested images, nor compare UDF peer implementations.
