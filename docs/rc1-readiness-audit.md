# V3 RC1 readiness audit

Audited `main` at `830ba8a5f7f787a93a0443a2e76626ee76f918c7` on
2026-10-06, after correctness PR #272 and UDF performance PR #273.

## Release recommendation

The supported profiles are suitable for an RC1 after the remaining release
preparation below. This audit found no additional runtime correctness blocker.
It checks release packaging, documentation, specification evidence and known
limitations; it is not a new exhaustive parser audit or a claim of complete
filesystem conformance. APFS and NTFS native APIs remain previews.

Before publishing:

1. Catalog repairs are complete: PR #271 merged at `6fd5254e`, including
   stronger implementation-symbol checking and review fixes. The correctness
   changes from #272 and the UDF buffer from #273 are preserved.
2. Merge this annotation/documentation audit cleanup and wait for its CI.
3. Prepare dated RC1 release notes incorporating the current Unreleased changes.
   `scripts/release-plan.py --notes 3.0.0-rc.1 all` intentionally refuses the
   present `## [3.0.0-rc.1] - Unreleased` heading.
4. Prepare the published installation instructions: README and website currently
   describe a Git-only candidate. Keep them accurate until publication, then
   switch the release instructions to crates.io dependencies.
5. Run the Release workflow on `main` with `mode: dry-run`, `crates: all` and
   `notes: 3.0.0-rc.1`. Publish only after that exact release revision passes.

No release workflow or upload was triggered by this audit.

## Documentation evidence

The all-feature rustdoc build passes with `RUSTDOCFLAGS="-D warnings"`.
Nightly rustdoc's `--show-coverage` reports 2,610 documented items across all
14 library crates, each at 100% documentation presence. These are rustdoc's
counts, including macro-generated APIs; they measure documentation presence,
not prose quality or behavioral completeness. CLI packages and example binaries
are outside that library denominator. Per-crate figures are recorded in
`docs/benchmarks/rc1-rustdoc-coverage.csv`.

At the audited main revision, doctests had 53 passes and 16 ignored tests.
The cleanup converts all 16 ignored instances into compile-checked examples
for Volume, the driver contract kit, CPIO reading/writing, ISO reading/sessions,
UDF and NTFS. They use explicit blocking imports and feature guards. These
`no_run` examples compile; they do not execute I/O on the placeholder device.
Existing executable examples still run. The resulting total is 69 passing
instances with none ignored. Sync examples appear in both generated modes;
this does not add new async usage examples.

The documentation checker originally selected 44 files and omitted most of
`docs/`. It now checks 82 files, including every current `docs/*.md` subtree,
CONTRIBUTING and KNOWN_ISSUES. Link existence and TOML checks pass. External
URLs, anchors, code samples in Markdown and archived website versions are not
validated by that script. Four example binaries also gained crate descriptions.

## Specification evidence

At the audited main revision, source annotations contain 63 blocks:
7 full, 50 partial and 6 unknown. They cover FAT raw layouts, ISO, UDF, NTFS,
partition tables and two APFS items. CPIO has an atomic catalog but no source
annotations. The cleanup adds conservative partial annotations for its three
header formats and the APFS volume superblock, producing 67 blocks:
7 full, 54 partial and 6 unknown.

The UDF descriptor-tag annotation still claimed file-identifier locations were
unchecked after #272 fixed that behavior. Its note and coverage-table row now
agree with the implementation and cite the new malformed-identifier regression.
The remaining partial tag status records that serial-number semantics are not
validated, rather than claiming the entire tag contract is verified.

The annotation checker originally accepted any `fn` with a matching leaf name,
including helpers without `#[test]`. It now requires a test attribute attached
to the function, with regression fixtures for helpers, intervening cfg attributes and long
comment lines. A line-oriented scan avoids backtracking over attribute/comment
sequences. This remains a text checker: it does not resolve complete Rust
module paths or prove that the cited test asserts every claimed rule.

The five atomic catalogs contain 88 selected requirements:
72 verified, 12 partial and 4 not implemented on audited main. These describe
selected claims, not a percentage of any complete standard. APFS and partition
tables have no atomic catalog yet. Applying #271's stricter symbol checker to
main identifies 17 stale implementation mappings that the existing checker
misses; #271 contains the catalog repairs. The normal catalog and annotation
checks and their self-tests pass on main, illustrating why passing metadata
checks alone do not establish conformance.

Nine of eleven catalog sources have retrieval dates and digests. The OSTA UDF
1.02 and UEFI 2.11 entries have neither. Those sources need further provenance
qualification before stronger compliance claims. Local source-cache contents
and source digests were not revalidated in this pass.

## Packaging and API readiness

A local `cargo +stable publish --locked --dry-run` selecting all 16 publishable
packages successfully packaged and verified every crate, including unpublished
workspace dependencies. This tests the registry-shaped source packages rather
than only workspace paths. No registry upload occurs during a dry run.

The manifests already use `3.0.0-rc.1`, except the new `hadris-fat-raw` at
`0.1.0`. Release-plan validation is blocked by the missing release date, not by
Cargo packaging. Credentials and the final GitHub Release workflow still need
the real release dry run; local packaging does not establish those conditions.

Issue #267, a possible local async tier for non-Send devices, is an API extension
and does not block the documented Send async tier. Shared traits already state
partial mutation and close/sync requirements. Once RC1 is published, follow the
repository policy of versioning incompatible changes in the next candidate.

## Limits acceptable for RC1

Keep the current known-issues disclosure and supported-profile boundaries:

- FAT/exFAT recovery after real power loss is partial. Async callers must close
  written handles and sync the volume; dropping a driver is not durability.
- Root setattr behavior and large-directory performance have documented limits.
- NTFS compression/encryption and MFT mirror recovery are unsupported.
- APFS hardware FileVault, per-file encryption and unsupported filesystem
  features remain outside the preview's qualified profile. Software unlocking
  is not a claim that encrypted Apple-silicon system volumes can be read.
- ISO/UDF support is profile-based; unknown and partial annotations must remain
  conservative. Round trips and native-tool acceptance alone are not proof of
  complete standards compliance.

Further performance work, complete standards coverage, and broader APFS/NTFS
qualification can follow RC1. They should not delay a candidate that exposes
its supported profile and these limits clearly.

## Checks

Run for this audit and cleanup: format check, warning-free workspace check,
warning-free all-feature rustdoc, all-feature doctests, annotation/catalog checks
and self-tests, expanded documentation check and self-test, affected annotation
regressions, and sync/async no-default checks on Rust 1.88 for the affected
library crates. Recent #272/#273 CI supplied the full platform, native-tool,
embedded, feature-tier and targeted Miri matrix; this cleanup changes docs and
Python tooling rather than parser algorithms.

Coverage was collected with nightly-2026-09-04; compilation and tests use Rust
1.97.1. Packaging uses the installed stable toolchain. This audit was performed
on macOS/aarch64 and does not replace release CI on Linux.
