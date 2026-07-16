# Hadris V2 Professionalization Review

**Originally reviewed:** 2026-07-09
**Reassessed:** 2026-07-16
**Scope:** V2 library/API readiness and release-candidate polish

## Current verdict

The core V2 library surface is close to API freeze. The earlier review's
highest-risk correctness and public-API gaps have been resolved: source-carrying
partition errors, strict backup GPT validation, UDF file reads, ISO namespace
selection, metadata-complete RRIP writing, El-Torito multi-section coverage,
and FAT cross-cluster LFN writes.

No remaining P0 correctness or API blocker was found in the supported
FAT/ISO/UDF/partition paths. The remaining work is intentionally split:

- **Current freeze tranche:** finish public API cleanup and keep this review current.
- **Later session:** perform CLI/CD release polish.

## Resolved since the original review

| Area | Resolution |
|------|------------|
| Workspace versions and inventory | Packages and examples use independent `2.0.0` versions; root README lists category, leaf, and CLI crates accurately |
| Public documentation trust | FAT, partition, I/O, common, ISO, and root documentation use the current APIs and feature model |
| `hadris-cli` release risk | Unpublished FAT debug stub removed; V2 exposes only the supported specialized CLI family |
| Partition defaults and errors | `read` is default; `PartitionError::Io` preserves its source |
| GPT integrity | Primary and backup headers, CRCs, geometry, and reciprocal locations are validated strictly |
| UDF content access | Public file reads and real directory-entry sizes are implemented and round-tripped |
| ISO namespace selection | All roots can be enumerated and selected explicitly |
| RRIP writing | Metadata, timestamps, symlinks, devices, and deep-directory relocation are written |
| El-Torito | Caller-prepared emulation images and multi-section catalogs are covered |
| FAT LFN writing | Maximum-length entry runs may cross directory cluster-chain boundaries |
| Tooling/process | Strict clippy, API snapshots, MSRV, docs metadata, Dependabot, contributor docs, and CLI smoke coverage are present |
| Fuzz expectations | Fuzzing is correctly documented as a local developer workflow |

## Current findings

Severity: **P0** release blocker, **P1** important before a polished V2 release,
**P2** follow-up polish.

| ID | Sev | Area | Current state | Disposition |
|----|-----|------|---------------|-------------|
| A1 | Resolved | ISO API | PVD access is fallible and available in sync/async builds; descriptor and boot-section cursors have async methods; `BootEntryInfo::media_type` spelling is corrected | Freeze after snapshot review |
| A2 | Resolved | UDF API | Dead `UdfError::UnsupportedRevision` variant removed; VRS continues to report the supported NSR revision family | Freeze after snapshot review |
| A3 | Resolved | exFAT | Retained as the leaf-only `unstable-exfat` preview, excluded from the stable V2 API snapshot and unified opener | Promote only after metadata, directory-growth, and I/O-mode qualification |
| A4 | Resolved | CLI surface | Canonical `hadris-*` binaries share common verbs, legacy executable aliases remain available, and FAT now covers create/read/extract workflows | Keep canonical names primary in V2 documentation |
| A5 | Resolved | `hadris-cd` | Non-empty nested bridge images reopen byte-for-byte through ISO and UDF, and `hadris-cd-cli` provides create/info/verify workflows | Keep bridge qualification in the all-feature suite |
| A6 | P2 | ISO specification | In-repo specification notes remain incomplete and are excluded from the package | Clearly treat as developer notes or finish as a later documentation project |
| A7 | P2 | UDF revision reporting | NSR02/NSR03 identify revision families, so `UdfInfo::udf_revision` is a representative family revision rather than guaranteed exact media revision | Document semantics if exact domain-suffix parsing is not added |
| A8 | Resolved | Async tests | FAT, partition, ISO, and UDF leaf crates directly exercise their public async namespaces in addition to facade coverage | Keep focused leaf and facade tests in the all-feature suite |
| A9 | Resolved | Public API docs | All published library crates deny missing docs; legacy FAT, partition, ISO, and CPIO gaps and generated sync/async duplicates are documented | Keep the workspace library-only missing-doc check clean |

## Release-candidate checklist

### Required before API freeze

- Review and commit the ISO/UDF public API snapshot changes.
- Keep the worktree clean and run workspace tests, strict clippy, formatting,
  feature checks, and public API validation.
- Keep the `unstable-exfat` preview outside the stable public API snapshot and
  unified opener.

### Required before V2 release candidate

- Re-run README examples and CLI `--help` smoke tests.
- Confirm crate publication flags and package contents.
- Produce release notes from the V2 commit history.
- Run external interoperability tests where tools are available:
  `fsck.fat`, `xorriso`, `udfinfo`/`mkudffs`, and archive extraction tools.

### Explicitly deferred

- Promotion of the exFAT preview, including fragmented metadata, safe directory
  growth/entry placement, and its final sync/async namespace.
- Completion of the bundled ISO specification notes.

## Compliance status

| Format | Current evidence | Remaining limitation |
|--------|------------------|----------------------|
| FAT12/16/32 | Unit/integration roundtrips, direct and facade async lifecycles, corruption guards, `fsck.fat`, cache paths, cross-cluster LFN lifecycle | None identified for the advertised V2 surface |
| ISO 9660/Joliet/RRIP | Roundtrips, xorriso comparison, RRIP metadata/relocation, boot catalogs, direct and facade async traversal | Developer specification notes incomplete |
| UDF | Descriptor tests, writer-to-reader roundtrips, external-tool tests, direct and facade async content reads | Exact revision within an NSR family is not always derivable from VRS alone |
| GPT/MBR | Direct and facade async lifecycles, strict primary/backup validation, typed errors | None identified for the advertised V2 surface |
| exFAT | Basic format/read/write roundtrips and external-tool checks | `unstable-exfat` preview; not part of the stable V2 support promise |
| CPIO | Roundtrips, async writer, corruption and allocation-bomb tests | CLI convention polish only |

## Strengths to preserve

1. Shared sync/async implementations through `hadris-macros`.
2. No-std and feature-tier compilation as first-class constraints.
3. Typed trust-boundary errors and bounded parsing/allocation.
4. External interoperability tests alongside internal roundtrips.
5. Public API snapshots reviewed before freeze.
6. Explicitly experimental surfaces are documented instead of overclaimed.
