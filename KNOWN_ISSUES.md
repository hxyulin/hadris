# Known issues

Open problems on the `main` (3.0) development branch that are understood but not fixed yet. Each entry says what goes wrong, who is affected, and the plan. When an entry is fixed, remove it in the same PR and mention the fix in `CHANGELOG.md`.

Requirement IDs (`FILE-CLOSE-01`, `NF-CRASH-01`) refer to [`docs/v3/actions.md`](docs/v3/actions.md).

## FAT and exFAT

- **Recovery after power loss is partial.** Operations interrupted by cancellation or a failed write are finished or rolled back by the next write or `sync`. After a real power cut, lost clusters can remain until a check and repair tool runs. An exFAT entry set that spans two device blocks can be left with orphan entries or a bad checksum, because the recovery state is kept in memory and not rebuilt at mount. TexFAT volumes can end with the mirror bitmap out of step. (NF-CRASH-01)
- **Async handles dropped without `close`.** In async mode `Drop` cannot await, so the size of a written file is published by the next call on the volume. If the driver is then dropped without `sync`, the size is lost. Always `close` files and `sync` the volume. (FILE-CLOSE-01)
- **Directory operations can still become quadratic** when long names or directories exceeding the bounded insertion/prefix caches require repeated scans. Dense short-name creation and sequential extraction have optimized paths. (NF-PERF-01)
- **`setattr` on the root refuses any change,** even one that keeps the value the root reports, such as chmod 0o755. Plan: accept values equal to what the root reports. (META-PERM-02)
