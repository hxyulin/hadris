# Known issues

Open problems on the `main` (3.0) development branch that are understood but not fixed yet. Each entry says what goes wrong, who is affected, and the plan. When an entry is fixed, remove it in the same PR and mention the fix in `CHANGELOG.md`.

Requirement IDs (`FILE-CLOSE-01`, `NF-CRASH-01`) refer to [`docs/v3/actions.md`](docs/v3/actions.md).

## FAT and exFAT

- **Recovery after power loss is partial.** Operations interrupted by cancellation or a failed write are finished or rolled back by the next write or `sync`. After a real power cut, lost clusters can remain until a check and repair tool runs. An exFAT entry set that spans two device blocks can be left with orphan entries or a bad checksum, because the recovery state is kept in memory and not rebuilt at mount. TexFAT volumes can end with the mirror bitmap out of step. (NF-CRASH-01)
- **Async handles dropped without `close`.** In async mode `Drop` cannot await, so the size of a written file is published by the next call on the volume. If the driver is then dropped without `sync`, the size is lost. Always `close` files and `sync` the volume. (FILE-CLOSE-01)
- **Directory operations can still become quadratic** when long names or directories exceeding the bounded insertion/prefix caches require repeated scans. Dense short-name creation and sequential extraction have optimized paths. (NF-PERF-01)
- **FAT16/32 never sets its dirty flag.** `was_dirty` reports the clean bit of FAT entry 1 at mount, but writes do not clear it and `sync` does not set it again, in `FatFs` and the embedded API. exFAT handles `VolumeDirty`. Plan: set the flag on the first write and clear it on a clean sync or unmount. (VOL-MOUNT-04)
- **`setattr` on the root refuses any change,** even one that keeps the value the root reports, such as chmod 0o755. Plan: accept values equal to what the root reports. (META-PERM-02)
- **The exFAT allocator reads only the bitmap,** so a cluster marked bad in the FAT with its bitmap bit clear can be allocated. FAT allocates only free entries. (CHECK-BAD-01)
- **`fat::write` and `exfat::write` format before copying,** so a tree that does not fit fails with `NoSpace` after the device was formatted. Options are checked before `format` writes. Plan: a `plan` for FAT and exFAT in 3.x, used by `write` first. (BUILD-PLAN-01)

## ISO 9660 and UDF

- **ISO file versions.** `lookup` compares names with the version stripped, so an explicit `A.TXT;2` is never found, and `readdir` lists every version under the same name. The highest version wins only when the image stores versions in ECMA-119 order. (DIR-LOOKUP-03)
- **The Rock Ridge relocation directory is listed.** `IsoFs` hides the relocated children but still lists `rr_moved` itself, so extraction through `read_tree` creates an empty `rr_moved/`. `Session` hides it. (BUILD-ISO-RR-02)
- **A Rock Ridge `TF` entry without a modification time drops the directory record's time,** because `TF` replaces all four times. (META-TIME-01)
- **`hadris udf create` and `hadris udf bridge` accept `--revision 2.50` and `2.60`,** and list them in `--help`, but the library refuses them when it plans the image, so the command fails with `Unsupported`. Plan: drop the two values from the CLI's `RevisionArg` until the writer supports a metadata partition. (BUILD-UDF-REV-01)
