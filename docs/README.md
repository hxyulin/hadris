# Repository documentation

Task-oriented guides live on the [documentation site](https://hxyulin.github.io/hadris/),
built from [`website/docs`](../website/docs). This directory holds the longer
references, evidence and records that the site links to.

## For users

- [Migrating from 2.x to 3.0](hadris-3.0.0-migration.md), with symbol tables
- [One async ISO driver](unified-async-iso.md); the async device contract is
  on the site: [implement an asynchronous block device](../website/docs/guides/async-devices.md)
- [Function tracing](tracing.md)
- [APFS use cases](apfs-v3-use-cases.md) and [APFS encryption scope](apfs-encryption.md)
- Compliance profiles: [FAT](compliance/hadris-fat.md), [ISO 9660](compliance/hadris-iso.md),
  [UDF](compliance/hadris-udf.md), [CPIO](compliance/hadris-cpio.md),
  [partition tables](compliance/hadris-part.md), [NTFS](compliance/hadris-ntfs.md)
  and [APFS](compliance/hadris-apfs.md)
- [Performance measurements](performance.md) and [peer performance](peer-performance.md),
  with their data in [`benchmarks/`](benchmarks/README.md)

## For contributors

- [Spec coverage](spec-coverage.md): the `@hadris-spec` annotation convention
  and coverage table that `scripts/check-spec-annotations.py` checks
- [FAT verification](fat-verification.md) and [ISO verification](iso-verification.md):
  the Kani proofs

## Design records

- [V3 API design](v3-api-design.md) and the [V3 action catalog](v3/actions.md).
  These record how 3.0 was designed; the rustdoc and the migration guide
  describe what ships.

## History and internal notes

- [`releases/`](releases): V2 release notes, from 2.0.0-rc.1 to 2.5.0. The
  [changelog](../CHANGELOG.md) covers every release.
- [`internal/`](internal): audits, performance investigations and bug
  write-ups at the revisions they name. They are kept as evidence and are not
  maintained against the current code.
