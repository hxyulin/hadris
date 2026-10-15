---
title: Stability and compatibility
---

# Stability and compatibility

These pages document Hadris 3.0.0-rc.2, the next release candidate of the
V3 API; 3.0.0-rc.1 is the latest published release candidate.
Hadris 2.5.0 is the current stable release of V2, whose source is retained at the
[`v2.5.0` tag](https://github.com/hxyulin/hadris/tree/v2.5.0).
See [installation](./getting-started.md) for the release candidate.
Archived V2 pages are available in the version selector. Released stable APIs follow Semantic Versioning: within a major series,
breaking changes require a new major version, minor releases add
backward-compatible functionality, and patch releases carry correctness
fixes, interoperability qualification, and documentation.

Read the [changelog](https://github.com/hxyulin/hadris/blob/main/CHANGELOG.md),
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

- Both crates carry 3.x versions and ship with the 3.0 release, but are
  exempt from semver: a 3.y minor release may break their API, and patch
  releases stay compatible. Depend on them with `~`, for example
  `hadris-apfs = "~3.0"`, to take only compatible updates.
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
  `unstable-*` feature does not bump it. `hadris-common`, whose endian
  integers the `hadris-fat-raw` layouts use, is an internal crate that,
  like the NTFS and APFS previews, keeps a 3.x version but breaks only in
  minor releases; no stable 3.x crate exposes its types.

## Raw crates

The `hadris-<format>-raw` crates (`hadris-fat-raw` 0.2, `hadris-iso-raw`,
`hadris-udf-raw` and `hadris-cpio-raw` 0.1) hold on-disk layouts and I/O-free
codecs. They stay at 0.x and version on their own, so any 0.x minor release
may break them; depend on them directly with a `0.x` requirement.

A format crate never re-exports its raw crate wholesale. It re-exports only
the raw items its own API takes or returns, and the types their fields and
methods use. Those items are part of the format crate's 3.x API and are
frozen: a raw release may add methods or new variants of `#[non_exhaustive]`
enums to them, but a breaking change to one would need a new major of the
format crate. A format crate also stays on one 0.x line of its raw crate for
its whole major, so its re-exported items stay the same types as those of a
direct dependency on that line. The rest of the raw crate is not covered by
the format crate's promise.

| Format crate | Raw items it re-exports |
|--------------|-------------------------|
| `hadris-fat` | `FatKind`, `Geometry`, `RootLocation`, `ChainError`, `Detail`, `exfat::Geometry`, `exfat::Detail` and the `check` functions |
| `hadris-iso` | `hadris_iso::raw`: `VolumeDescriptor`, `BootSectionEntry` and the descriptor, directory-record header, date, identifier and integer field types they contain |
| `hadris-udf` | None |
| `hadris-cpio` | None |

A CI lint rejects `pub use hadris_<format>_raw::*` in the format crates.

`cargo semver-checks` checks compatibility against each crate's latest 3.x
release, with feature builds, sync/async contract tests and the non-exhaustive
lint covering separate API rules. Format behavior is
also tracked in the repository's specification-compliance catalog.
