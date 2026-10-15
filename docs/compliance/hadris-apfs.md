# `hadris-apfs` compliance scope

`hadris-apfs` is a read-only preview outside the 3.x stability promise. No
source is pinned in [`spec/sources.json`](../../spec/sources.json) and there
is no requirement catalog under `spec/requirements/`, so no APFS behavior is
labeled source-verified, and this is a scope note rather than a conformance
profile.

The reader follows Apple's *Apple File System Reference* for the structures
it parses: container and volume superblocks, the checkpoint descriptor area,
object maps, B-trees, inode, directory and extent records, and the
incompatible-feature masks it checks before mounting. The crate's
[supported scope](../../crates/block/hadris-apfs/README.md#supported-scope)
lists what is read and what fails with `Unsupported`, and the
[encryption scope](../apfs-encryption.md) covers software unlocking.

Evidence is interoperability and robustness evidence, not clause-level
conformance: images macOS creates through `hdiutil` and `diskutil`,
including encrypted ones, are read and compared with the native mount,
crafted images exercise checkpoint fallback, per-node damage and feature
refusal, and the `apfs_read` fuzz target runs locally. Writing,
snapshots, Fusion containers, hardware-encrypted volumes and per-file keys
are out of scope.
