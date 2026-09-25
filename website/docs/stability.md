---
title: Stability and compatibility
---

# Stability and compatibility

Hadris 3.0 is developed on the `next` branch and is not released yet. Hadris
2.4.0 is the current stable release of the V2 API, and the 2.x series
continues on `main`. Both follow Semantic Versioning: within a major series,
breaking changes require a new major version, minor releases add
backward-compatible functionality, and patch releases carry correctness
fixes, interoperability qualification, and documentation.

Read the [unreleased changes](https://github.com/hxyulin/hadris/blob/next/CHANGELOG.md),
the [3.0 API design](https://github.com/hxyulin/hadris/blob/next/docs/v3-api-design.md)
or the [2.4.0 changelog](https://github.com/hxyulin/hadris/blob/main/CHANGELOG.md#240---2026-09-08),
and report real-world compatibility findings through
[GitHub Issues](https://github.com/hxyulin/hadris/issues).

In 3.0, exFAT is stable as `hadris_fat::exfat::ExFatFs`, with no feature
flag; in 2.x it was the `unstable-exfat` preview. The `hadris-ntfs` reader,
and the `unstable-ntfs` feature of `hadris` that exposes
its native API, stay outside the stability promise: its `FileSystem`
implementation follows the frozen trait, but its native methods may change in
3.x minor releases.

## Compatibility policy

- Stable crates follow Semantic Versioning within a major series.
- New format support and additive APIs may arrive in minor releases. Traits
  that users implement grow only through methods with default bodies.
- Correctness and interoperability fixes may arrive in patch releases.
- APIs behind an `unstable-*` feature can change before they are declared
  stable. No other feature changes what an item does.
- On-disk compatibility fixes take priority over preserving incorrect output.

Public API snapshots cover every stable crate and run in CI. Format behavior is
also tracked in the repository's specification-compliance catalog.
