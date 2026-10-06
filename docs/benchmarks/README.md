# Benchmark evidence

These CSVs and metadata record measurements at the revisions named in their
reports. They are historical evidence, not results for the current `main` tip.
Keep before/after files together and retain their measurement boundaries.

| Data | Report |
|---|---|
| `v2-v3-*` | [V3 promotion audit](../v3-promotion-audit.md) |
| `fat-extraction-*`, `fat-read-ahead-*`, `storage-read-ahead-*` | [Peer performance](../peer-performance.md) |
| `fat-hosted-*` | [Hosted FAT audit](../fat-hosted-performance-audit.md) and [performance guide](../performance.md) |
| `fat-directory-index-*` | [Performance guide](../performance.md) |
| `fat12-allocation-*` | [FAT12 allocation batching](../fat12-allocation-batching.md) |
| `exfat-*` | [Performance guide](../performance.md) |
| `udf-*` | [UDF performance](../udf-performance.md) |
| `cpio-*` | [CPIO performance](../cpio-performance.md) |

Store local profiler captures, generated images, PDFs and scratch reports under
the ignored root `output/` directory, or in a worktree's output directory.
Commit small measurement tables and metadata here when a report cites them.
Build directories, generated website versions and local scratch reports are
not release artifacts.
