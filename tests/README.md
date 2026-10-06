# hadris-tests

Centralized conformance and interoperability suite for the Hadris filesystem
crates. The package is detached from the workspace (it declares its own
`[workspace]`, like `fuzz/`), depends on the library crates by path, and is
designed to grow to hundreds of tests and to be extractable into its own
repository.

## Principles

- **The oracle is the ground truth.** Each format has a test-only raw-image
  reader written directly from the on-disk rules (`src/<format>/spec.rs`). It
  shares no code with the implementations under test.
- **Hadris is one adapter among peers.** Every implementation, Hadris
  included, is driven through the same trait (`src/<format>/adapter.rs`) and
  scored the same way. A Hadris-to-Hadris round trip is never evidence on its
  own.
- **Peers are measured, not trusted.** External tools produce scorecards with
  pass/attempt counters and failure details; a peer deviating from the
  specification is reported, not treated as a Hadris failure.

## Layout

```
tests/
  Cargo.toml          detached package `hadris-tests`
  src/                harness library (`hadris_tests`)
    harness/          format-agnostic: commands, workspaces, scorecards,
                      tree diffing, path helpers, native mounts, RNG, QEMU
    fat/              FAT model, oracle, scenarios, adapters
      model.rs        FsState / Operation reference model
      adapter.rs      FatAdapter trait and trace drivers
      spec.rs         raw-image oracle
      scenarios.rs    curated, edge-case, rejection, and seeded traces
      limits.rs       root-directory, data-region, and long-extent exercises
      generic.rs      adapter over any hadris-fs FileSystem; Hadris FatFs
      fatfs.rs        rust-fatfs adapter
      mtools.rs       GNU mtools + dosfstools adapter
      native.rs       host formatter, checker, and kernel driver
    exfat/            exFAT oracle, scenarios, limits and adapters; shares
                      the FAT model and adapter trait
      spec.rs         raw-image oracle
      scenarios.rs    exFAT-only traces and rejections
      limits.rs       data-region exhaustion, directory growth to a full
                      volume, and long extents
      generic.rs      Hadris ExFatFs through the generic FAT adapter
      native.rs       exfatprogs (local or Docker), newfs_exfat,
                      fsck_exfat and the macOS kernel driver
    iso/              ISO 9660 model, oracle, scenarios, adapters
      model.rs        IsoState and conformance scenarios
      adapter.rs      IsoProducer / IsoConsumer traits
      measure.rs      scoring drivers for producers and consumers
      spec.rs         raw ECMA-119 oracle
      hadris.rs       Hadris adapter
      xorriso.rs      xorriso/libisofs adapter and helpers
      mkisofs.rs      cdrtools mkisofs / genisoimage adapter
      native.rs       hdiutil producer and kernel ISO reader
  suite/              the single test binary
    main.rs
    fat/{spec,limits,peers,native}.rs
    exfat/{spec,limits,native}.rs
    iso/{spec,peers,native,volume_descriptors,directory,multi_extent,
         rock_ridge,relocation,boot,hybrid,session}.rs
```

Tests are addressed as `<format>::<topic>::<name>`:

```bash
cargo test --manifest-path tests/Cargo.toml              # hosted suite
cargo test --manifest-path tests/Cargo.toml exfat::      # one format
cargo test --manifest-path tests/Cargo.toml fat:: -- --skip exfat::
cargo test --manifest-path tests/Cargo.toml iso::boot::  # one topic
cargo test --manifest-path tests/Cargo.toml -- --ignored # manual peer reports
cargo test --manifest-path tests/Cargo.toml iso::boot::test_qemu_boot -- --ignored # QEMU boot checks

# Require every command-line peer tool through the repository flake
nix develop -c env HADRIS_REQUIRE_EXTERNAL_TOOLS=1 \
  cargo test --manifest-path tests/Cargo.toml
```

A filter is a substring match on the test path, so `fat::` also selects the
`exfat::` tests. CI runs `fat:: -- --skip exfat::` and `exfat::` as separate
jobs.

FAT, exFAT and ISO use the same three test tiers. Hosted oracle tests always run.
External-tool interoperability tests skip when their tools are absent, while
CI sets `HADRIS_REQUIRE_EXTERNAL_TOOLS=1` to make them mandatory. Accuracy
reports, QEMU checks, and privileged native-mount checks are `#[ignore]`d and
run only with `-- --ignored`; the QEMU boot checks fail when the image does not
boot, skip when QEMU is absent, and treat a missing QEMU as a failure under
`HADRIS_REQUIRE_EXTERNAL_TOOLS=1`.
The commands and environment variables are documented in the repository
[`CONTRIBUTING.md`](../CONTRIBUTING.md#conformance-and-interoperability-suite).
Reports are written to `tests/target/reports/<format>/`.

## Adding tests

- Put a new check under `suite/<format>/<topic>.rs`, adding the module to
  `suite/<format>/mod.rs`. Start a new topic file rather than growing an
  unrelated one.
- Put reusable scenario data in `src/<format>/scenarios.rs` (FAT and exFAT)
  or `src/<format>/model.rs` (ISO) so every adapter can run it. FAT and exFAT
  scenarios come in three shapes: operation traces every implementation must
  complete, rejection scenarios whose final operation every implementation
  must refuse while leaving the image untouched, and geometry-sized limit
  exercises in `src/<format>/limits.rs` that fill a directory or the data
  region.
- Add a new implementation by implementing the format's adapter trait in
  `src/<format>/<peer>.rs`; the existing measurement drivers then score it
  without further changes.
- Tests that need an external tool should call the tool module's `require()`
  (or `harness::require_or_skip`) so they skip locally and fail when
  `HADRIS_REQUIRE_EXTERNAL_TOOLS=1` is set.

- When a test is cited as compliance evidence, reference it as
  `<format>::<topic>::<name>` in `@hadris-tests` annotations and
  `docs/spec-coverage.md`, and by file path in `spec/requirements/*.json`.

CI runs the FAT, exFAT and ISO slices with their command-line peer tools
installed, and also formats and lints the package. Manual accuracy reports,
QEMU checks, and privileged native-mount checks remain ignored.

Rock Ridge relocation extraction is checked with `bsdtar` and `xorriso` under
`iso::relocation::`. Both compare every extracted path, entry kind, and file byte
against the input model, including user `rr_moved` and `.rr_moved` root
directories and name collisions inside a reused relocation directory. xorriso
may leave an empty relocation directory behind after restoring the logical
tree; bsdtar hides it. Install libarchive (`libarchive-tools` on Debian/Ubuntu)
and xorriso, or use the repository flake. Optical CI requires these tools; local
runs skip a test if its tool is unavailable unless
`HADRIS_REQUIRE_EXTERNAL_TOOLS=1` is set.

## V3 performance harness

```bash
cargo bench --manifest-path tests/Cargo.toml --bench performance
HADRIS_TESTS_PERF_SAMPLES=1 cargo bench --manifest-path tests/Cargo.toml --bench performance
HADRIS_TESTS_PERF_FILTER=udf cargo bench --manifest-path tests/Cargo.toml --bench performance
```

The runner writes `performance/v3.csv` under `HADRIS_TESTS_REPORT_DIR` (default
`tests/target/reports`). Each row records one sample's elapsed nanoseconds,
requested read/write calls and bytes, flushes, I/O failures and stream seeks. Keep the raw
samples to compare medians and spread rather than relying on a single run.
The default is 21 samples plus one discarded warm-up for each workload.
The format filter is a substring; a filter matching nothing fails.

FAT12/16/32, exFAT, ISO and UDF run the same V3 `FileSystem` operations:
mount, root listing, last-file lookup, missing-file lookup, stat, 4 KiB and
64 KiB reads, and five scattered 4 KiB reads. Each image has 32 small files
and a 128 KiB patterned payload. FAT, exFAT and ISO fixtures pass the
independent raw-image oracles before measurement. Every driver must return
the expected names and payload bytes; UDF has no raw oracle in this package.

Every sample mounts a fresh read-only memory device. Mount is timed separately;
other windows exclude mounting, file lookup/open for stat/read, buffer setup,
correctness verification, close, destruction and reporting. Listing includes
collecting inline entries into a preallocated vector. Lookup includes forgetting
the returned pin. Scattered reads use preallocated buffers. These measure CPU
and requested I/O on memory, including the counter overhead; they do not measure
physical disk latency, OS-cache behavior, peak memory, or async execution.
Record the Rust version, machine and revision alongside saved comparisons.
CI runs one sample as a correctness smoke check, with no timing threshold.

`harness::performance::Counted` forwards V3 device geometry, capacity, disk
offset and writability. Put it below `hadris_storage::Cache` to measure backend
I/O, or above a cache to measure driver requests. Counters include failed calls
and requested bytes, and `Measurement::run` resets them for each window. This
shared synchronous infrastructure is available to new format and peer adapters;
external CLI timings need separate labels because process startup and host I/O
have different measurement boundaries. NTFS, APFS and streaming cpio workloads
remain follow-ups requiring suitable fixtures and workload boundaries.

The `peers` benchmark compares Hadris with buffered/unbuffered rust-fatfs,
dosfstools/mtools, xorriso, mkisofs/genisoimage and bsdtar on shared host
workflows. Run `cargo bench --manifest-path tests/Cargo.toml --bench peers`.
See [peer performance and reference coverage](../docs/peer-performance.md)
for timings, counter boundaries, larger fixtures and more complete references.
Both runners accept `HADRIS_TESTS_PERF_FILES` (32 by default). For large
directories, use a FAT32, exFAT, ISO or UDF filter with `performance`, and a
FAT32 or ISO filter with `peers`.


For isolated extraction RSS, the `peers` executable also supports a one-operation
worker on a prebuilt FAT32 image. Set `HADRIS_TESTS_PEER_WORKER`,
`HADRIS_TESTS_PEER_IMAGE`, and `HADRIS_TESTS_PEER_DESTINATION`, then measure the
child with a platform resource tool. Fixture generation and validation must
run in the parent or separate processes. See
[`docs/peer-performance.md`](../docs/peer-performance.md#isolated-extraction-rss-and-peer-comparison)
for worker/cache modes, measurement boundaries, external image-read counting
and the per-process RSS/speed results.
