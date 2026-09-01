---
title: Stability and compatibility
---

# Stability and compatibility

Hadris 2.3.0 is the current stable release of the V2 API, first stabilized in
2.0.0. The public surface frozen during the release-candidate series follows
Semantic Versioning: within the `2.x` series, breaking changes require a new
major version, minor releases add backward-compatible functionality, and patch
releases carry correctness fixes, interoperability qualification, and
documentation.

Read the [2.3.0 changelog](https://github.com/hxyulin/hadris/blob/main/CHANGELOG.md#230---2026-09-01)
or the [V2 upgrade notes](https://github.com/hxyulin/hadris/blob/main/docs/hadris-2.0.0-release-notes.md),
and report real-world compatibility findings through
[GitHub Issues](https://github.com/hxyulin/hadris/issues).

The `unstable-exfat` preview and experimental `hadris-ntfs` reader are
explicitly outside the V2 stability promise.

## Compatibility policy

- Stable crates follow Semantic Versioning within the `2.x` series.
- New format support and additive APIs may arrive in minor releases.
- Correctness and interoperability fixes may arrive in patch releases.
- Feature-gated experimental APIs can change before they are declared stable.
- On-disk compatibility fixes take priority over preserving incorrect output.

Public API snapshots cover every stable crate and run in CI. Format behavior is
also tracked in the repository's specification-compliance catalog.
