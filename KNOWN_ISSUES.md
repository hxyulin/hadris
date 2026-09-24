# Known issues

Open problems on the `next` (3.0) branch that are understood but not fixed yet. Each entry says what goes wrong, who is affected, and the plan. When an entry is fixed, remove it in the same PR and mention the fix in `CHANGELOG.md`.

Requirement IDs (`FILE-CLOSE-01`, `NF-CRASH-01`) refer to [`docs/v3/actions.md`](docs/v3/actions.md).

## FAT and exFAT

- **Recovery after power loss is partial.** Operations interrupted by cancellation or a failed write are finished or rolled back by the next write or `sync`. After a real power cut, lost clusters can remain until a check and repair tool runs. An exFAT entry set that spans two device blocks can be left with orphan entries or a bad checksum, because the recovery state is kept in memory and not rebuilt at mount. TexFAT volumes can end with the mirror bitmap out of step. (NF-CRASH-01)
- **Async handles dropped without `close`.** In async mode `Drop` cannot await, so the size of a written file is published by the next call on the volume. If the driver is then dropped without `sync`, the size is lost. Always `close` files and `sync` the volume. (FILE-CLOSE-01)
- **`Volume` without `alloc`** panics when a 17th distinct node is queued while its lock is held. Plan: `Volume` requires `alloc` in 3.0 (design 4.15). (HOST-CONC-01)
- **chmod and chown are silent no-ops.** Plan: fail with `Unsupported` unless the value is what the format would report (META-TIME-02). (META-PERM-02, META-OWNER-02)
- **Creating many files in one directory is still quadratic,** only faster than before. (NF-PERF-01)

## ISO 9660 and UDF

- **Relocation container names other than `rr_moved` and `.rr_moved`.** libarchive and bsdtar only recognize those two names, so an image built with any other `Relocation::Directory` name cannot be extracted by them once directories are relocated. Plan: offer only the two recognized names. (BUILD-ISO-RR-02)
- **The UDF and cpio CLIs accept `--revision 2.50` and `2.60`,** which the library refuses. Plan: removed with the single `hadris` CLI (design 4.17 S1). (BUILD-UDF-REV-01)
