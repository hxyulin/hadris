# FUSE prototype

A throwaway FUSE adapter over any sync `hadris_fs` `FileSystem`, mounted on
`FatFs` through a `Volume`. It exists to test the `hadris-fs` traits before
they are frozen (V3 migration step 6). The findings are in
[`docs/v3-trait-review.md`](../../docs/v3-trait-review.md).

The package is detached from the workspace, like `fuzz/` and `tests/`.

- `src/lib.rs`: `Adapter`, one method per FUSE request returning
  `Result<_, Errno>`, and `Fuse`, the `fuser::Filesystem` glue. Kernel
  lookups are driver pins one to one, open files are `open_node` ..
  `close_node`, `close(2)` publishes and `fsync(2)` syncs. Gaps that remain
  in the traits (all compatible 3.x additions) are marked `GAP:`.
- `src/main.rs`: `format <image> <bytes> [fat12|fat16|fat32]` and
  `mount <image> <mountpoint> [--threads N] [--ro]`.
- `tests/ops.rs`: the request sequences the kernel sends, replayed against
  `Adapter` without a mount, plus the `hadris-fs` contract kit on the
  mounted volume. Runs on macOS.
- `scripts/docker.sh`: builds in a privileged Linux container and runs
  `scripts/scenarios.sh`, which mounts a fresh FAT32 image and drives it with
  shell tools, then checks it with `fsck.fat -n`.

```bash
cargo test --manifest-path experiments/fuse-prototype/Cargo.toml
experiments/fuse-prototype/scripts/docker.sh
```

On macOS `fuser` is built with `macos-no-mount`, so no macFUSE is needed to
build or test; mounting only happens in the container. The container needs
`--privileged` and `/dev/fuse` (OrbStack and Docker Desktop provide both).

Known gaps, by design of the prototype or of the contract it tests:

- Removing or replacing an open file fails with `EBUSY` (`ErrorKind::Busy`).
- No symlinks, hard links, devices or xattrs (FAT has none).
- No readdirplus, no writeback cache, no file locks.
- Mode, uid and gid changes succeed and are ignored.
