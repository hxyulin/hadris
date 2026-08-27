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
                      tree diffing, native mounts, RNG, QEMU
    fat/              FAT model, oracle, scenarios, adapters
      model.rs        FsState / Operation reference model
      adapter.rs      FatAdapter trait and trace drivers
      spec.rs         raw-image oracle
      scenarios.rs    curated, edge-case, and seeded traces
      hadris.rs       Hadris adapter
      fatfs.rs        rust-fatfs adapter
      mtools.rs       GNU mtools + dosfstools adapter
      native.rs       host formatter, checker, and kernel driver
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
    fat/{spec,peers,native}.rs
    iso/{spec,peers,native,volume_descriptors,directory,rock_ridge,boot,hybrid}.rs
```

Tests are addressed as `<format>::<topic>::<name>`:

```bash
cargo test --manifest-path tests/Cargo.toml              # hosted suite
cargo test --manifest-path tests/Cargo.toml fat::        # one format
cargo test --manifest-path tests/Cargo.toml iso::boot::  # one topic
cargo test --manifest-path tests/Cargo.toml -- --ignored # manual peer reports
```

Manual reports and privileged native-mount checks are `#[ignore]`d; the
commands and environment variables are documented in the repository
[`CONTRIBUTING.md`](../CONTRIBUTING.md#conformance-and-interoperability-suite).
Reports are written to `tests/target/reports/<format>/`.

## Adding tests

- Put a new check under `suite/<format>/<topic>.rs`, adding the module to
  `suite/<format>/mod.rs`. Start a new topic file rather than growing an
  unrelated one.
- Put reusable scenario data in `src/<format>/scenarios.rs` (FAT) or
  `src/<format>/model.rs` (ISO) so every adapter can run it.
- Add a new implementation by implementing the format's adapter trait in
  `src/<format>/<peer>.rs`; the existing measurement drivers then score it
  without further changes.
- Tests that need an external tool should call the tool module's `require()`
  (or `harness::require_or_skip`) so they skip locally and fail in CI jobs
  that set `HADRIS_REQUIRE_EXTERNAL_TOOLS=1`.
- When a test is cited as compliance evidence, reference it as
  `<format>::<topic>::<name>` in `@hadris-tests` annotations and
  `docs/spec-coverage.md`, and by file path in `spec/requirements/*.json`.
