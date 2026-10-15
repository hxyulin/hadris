# Hadris 2.0.0 Release Notes

Hadris 2.0.0 is the first stable release of the V2 API. The public API frozen
during the release-candidate series is now stable under Semantic Versioning:
within the `2.x` series, breaking changes require a new major version, minor
releases add backward-compatible functionality, and patch releases carry
correctness fixes, interoperability qualification, and documentation.

Applications already built against `2.0.0-rc.4` need no source changes; the
work since RC4 is specification-conformance hardening, new compliance
infrastructure, and the experimental NTFS crate.

## Specification conformance

The bulk of the work since RC4 tightened on-disk conformance across the
optical and archive formats. Some of these fixes change emitted bytes for
images that previously deviated from the relevant standard:

- **ISO:** ECMA-119 invariants are enforced, descriptor conformance is
  validated, and the aligned image tail is preserved.
- **UDF:** Anchor and Volume Descriptor Sequence layout were corrected, and
  descriptor validation and decoding are portable across targets.
- **ISO/UDF bridge:** The bridge descriptor layout conforms to ECMA TR/71, and
  space is reserved for the trailing UDF anchor.
- **CPIO:** `newc` archive invariants are enforced, and aligned trailerless
  archives are accepted.
- **FAT:** BPB geometry is validated, filesystem integrity rules are enforced,
  and checksum validation is supported across read tiers.
- **Partitions:** Partition metadata handling was hardened.

Images written by RC4 remain readable. Applications that byte-compare emitted
images against stored fixtures should regenerate those fixtures.

## Compliance catalog

A compliance catalog framework now backs the standards claims, with pinned
source digests and extracted requirement sets for ECMA-119, ECMA-167, UDF
1.02, the ECMA TR/71 bridge format, and source-bounded NTFS MFT behavior.
Full compliance claims require runnable test evidence, and CI verifies both
evidence existence and bidirectional coverage-table parity.

## Experimental NTFS

`hadris-ntfs` is a new read-only NTFS leaf crate with validated boot geometry,
MFT and directory traversal, resident and non-resident file reading, sparse
runs, Unicode filenames, and sync/async `no_std` support. It is **experimental
and outside the stable V2 API promise** — its surface may change in a minor
release. The unstable exFAT preview in `hadris-fat` carries the same caveat.

## Build

The workspace uses Cargo resolver 3 so dependency resolution prefers releases
compatible with the declared Rust 1.88 MSRV.

## Verification

2.0.0 is qualified by:

- the complete workspace unit, integration, CLI, example, external
  interoperability, and doctest suite;
- the per-crate feature tiers, including allocation-free and `no_std`
  configurations, with warnings denied;
- workspace Clippy across all targets and features with warnings denied;
- Miri over the historically-unsafe parsing paths;
- public API snapshots for every published crate;
- standards annotation grammar and coverage-index checks; and
- FAT, exFAT, ISO 9660/Joliet/Rock Ridge/El Torito, UDF, MBR/GPT, CPIO, and
  ISO/UDF bridge roundtrips, including independent external-tool validation.

## Publishing

Publish in workspace dependency order: `hadris-macros`, `hadris-io`,
`hadris-fixed`, `hadris-path`, `hadris-common`, `hadris-storage`, then the
format crates (`hadris-fat`, `hadris-part`, `hadris-ntfs`, `hadris-iso`,
`hadris-udf`), then `hadris-cd`, the facade crates (`hadris-block`,
`hadris-optical`, `hadris-archive`), then `hadris`, and finally the CLI tools.
