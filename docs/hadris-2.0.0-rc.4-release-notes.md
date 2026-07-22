# Hadris 2.0.0-rc.4 Release Notes

Hadris 2.0.0-rc.4 is the V2 API-freeze candidate. It completes the pre-release
cleanup of naming, lifecycle, feature gating, tests, standards evidence, and
documentation. The release should soak without further public API changes;
only release-blocking correctness fixes are expected before 2.0.0.

## Final V2 API cleanup

- Filesystem handles consistently use `FatVolume` and `UdfVolume`; partition
  tables use `PartitionTable`; archive readers use `CpioArchiveReader`; and the
  ISO/UDF bridge uses `OpticalImageWriter` and `OpticalImageOptions`.
- Public leaf crates consistently expose `Error` and `Result`.
- Temporary pre-release aliases, deprecated `with_*` setters, implicit-size
  helpers, and non-recoverable writer/modifier lifecycle methods were removed.
- CPIO creation now uses the owning `CpioArchiveWriter` lifecycle exclusively.
- The compact ISO `InputFiles` tree remains an intentional convenience model;
  `InputTree` is the metadata-aware and host-filesystem-import model.
- Unreachable UDF modification/file placeholders and duplicated
  `hadris-common` compatibility modules were removed.

These are pre-stable breaking changes from RC3. Applications should follow
[`hadris-1-to-2-migration.md`](hadris-1-to-2-migration.md) before adopting RC4.

## Implementation and feature cleanup

- Dead-code and unused-import suppressions were replaced with precise feature
  gates across sync/async, allocation-free, read-only, and write builds.
- FAT, GPT, ISO, and exFAT internals no longer compile write-only state into
  read-only configurations.
- FAT/exFAT integration helpers now live under `tests/common`, avoiding empty
  integration-test binaries.
- External exFAT interoperability tests skip unavailable or unusable host tools
  in ordinary local runs, while the dedicated CI job requires those tools.

## Verification

RC4 is qualified by:

- all 74 feature-matrix configurations with warnings denied;
- the complete workspace all-feature unit, integration, CLI, example, external
  interoperability, and doctest suite;
- workspace Clippy across all targets and features with warnings denied;
- public API snapshots for every published crate;
- standards annotation grammar and coverage-index checks; and
- FAT, exFAT, ISO 9660/Joliet/Rock Ridge/El Torito, UDF, MBR/GPT, CPIO, and
  ISO/UDF bridge roundtrips, including independent external-tool validation.

The unstable exFAT preview remains outside the stable V2 API snapshot and
stability promise.

## Release policy

Do not promote RC4 directly to 2.0.0. Publish in workspace dependency order,
allow the candidate to soak, and accept only release-blocking corrections or
documentation clarifications before the final release.
