---
title: Stability and compatibility
---

# Stability and compatibility

Hadris 3.0.0-rc.2 is the current release candidate of the V3 API.
Hadris 2.5.0 is the current stable release of V2, whose source is retained at the
[`v2.5.0` tag](https://github.com/hxyulin/hadris/tree/v2.5.0).
See [installation](./getting-started.md) for the release candidate.
The default site documents RC2, and archived V2 pages are available in the
version selector. Released stable APIs follow Semantic Versioning: within a major series,
breaking changes require a new major version, minor releases add
backward-compatible functionality, and patch releases carry correctness
fixes, interoperability qualification, and documentation.

Read the [RC2 release notes](https://github.com/hxyulin/hadris/blob/main/CHANGELOG.md),
the [3.0 API design](https://github.com/hxyulin/hadris/blob/main/docs/v3-api-design.md)
or the [2.5.0 changelog](https://github.com/hxyulin/hadris/blob/v2.5.0/CHANGELOG.md#250---2026-10-03).
The [migration guide](https://github.com/hxyulin/hadris/blob/main/docs/hadris-3.0.0-migration.md)
maps the 2.4 API and the APFS additions in 2.5 to their 3.0 replacements. Report real-world compatibility findings through
[GitHub Issues](https://github.com/hxyulin/hadris/issues).

In 3.0, exFAT is stable as `hadris_fat::exfat::sync::ExFatFs` and
`hadris_fat::exfat::async_::ExFatFs`, with no feature flag; in 2.x it was
the `unstable-exfat` preview. The driver contract kit behind `hadris-fs`'s
`unstable-contract` feature is a test aid for format authors and is also
outside the promise; its rules may change in any release.

## Previews: NTFS and APFS

The `hadris-ntfs` and `hadris-apfs` readers are previews outside the
stability promise. Their `FileSystem` implementations follow the frozen
trait, but their native APIs may still change.

- Both crates are versioned 0.x (0.1.0), not 3.x, and are not part of the
  3.0 release train. Each 0.y release may break their API; patch releases
  stay compatible, so `hadris-apfs = "0.1"` only takes compatible updates.
  crates.io also holds their older 2.x and 3.0.0-rc.1 releases, and a bare
  `cargo add hadris-apfs` picks 2.5.0, so name the version:
  `cargo add hadris-apfs@0.1`.
- In `hadris`, everything the `unstable-ntfs` and `unstable-apfs` features
  add is outside semver and may change in any release. That covers the
  `hadris::ntfs` and `hadris::apfs` re-exports, and with `detect`, the
  `AnyFs::Apfs` variant, APFS mounting in `open`, `open_partition` and
  `host::open`, and `open_apfs`. Pin `hadris` with `~` or `=` when you
  enable one.
- The stable `detect` feature does not depend on either preview crate. It
  lists NTFS volumes and APFS containers as `ImageFormat::Ntfs` and
  `ImageFormat::Apfs` from their signatures, and `open` fails on them with
  `NotRecognized`. `AnyFs` is non-exhaustive, so a variant is added in a
  3.x minor when a preview becomes stable.

## Compatibility policy

- Stable crates follow Semantic Versioning within a major series.
- New format support and additive APIs may arrive in minor releases. Traits
  that users implement grow only through methods with default bodies.
- Correctness and interoperability fixes may arrive in patch releases.
- APIs behind an `unstable-*` feature are outside semver and can change in
  any release before they are declared stable. No other feature changes
  what an item does.
- On-disk compatibility fixes take priority over preserving incorrect output.
- Every stable crate ships 3.0.0 together. After that each crate has its own
  version and release tag (`hadris-fat-v3.1.0`) and bumps its major only
  for its own breaking changes. The umbrella `hadris` bumps its major when a
  stable crate it re-exports does; a break in a preview behind an
  `unstable-*` feature does not bump it. `hadris-fat-raw` versions independently and is
  0.2.0 in RC2. The ISO, UDF and CPIO raw crates start at 0.1.0 and also
  version independently. `hadris-common`, whose endian integers the
  `hadris-fat-raw` layouts use, is a 0.x support crate (0.3.0) versioned
  the same way; no stable 3.x crate exposes its types. The NTFS and APFS
  previews are 0.x, as above.

`cargo semver-checks` checks compatibility against each crate's latest 3.x
release, with feature builds, sync/async contract tests and the non-exhaustive
lint covering separate API rules. Format behavior is
also tracked in the repository's specification-compliance catalog.
