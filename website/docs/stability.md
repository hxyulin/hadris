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
the `unstable-exfat` preview. The `hadris-ntfs` reader, and the
`unstable-ntfs` feature of `hadris` that exposes its native API, stay outside
the stability promise: its `FileSystem` implementation follows the frozen
trait, but its native methods may change in 3.x minor releases.

## Compatibility policy

- Stable crates follow Semantic Versioning within a major series.
- New format support and additive APIs may arrive in minor releases. Traits
  that users implement grow only through methods with default bodies.
- Correctness and interoperability fixes may arrive in patch releases.
- APIs behind an `unstable-*` feature can change before they are declared
  stable. No other feature changes what an item does.
- On-disk compatibility fixes take priority over preserving incorrect output.
- Every crate ships 3.0.0 together. After that each crate has its own
  version and release tag (`hadris-fat-v3.1.0`) and bumps its major only
  for its own breaking changes. The umbrella `hadris` bumps its major when a
  crate it re-exports does. `hadris-fat-raw` versions independently and is
  0.2.0 in RC2. The ISO, UDF and CPIO raw crates start at 0.1.0 and also
  version independently.

`cargo semver-checks` checks compatibility against each crate's latest 3.x
release, with feature builds, sync/async contract tests and the non-exhaustive
lint covering separate API rules. Format behavior is
also tracked in the repository's specification-compliance catalog.
