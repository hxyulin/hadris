# Spec Compliance Program (Phase E) — Design

**Date:** 2026-07-09
**Status:** v0 + v1 landed (`scripts/check-spec-annotations.py` + CI job `Spec annotations`)
**Parent review:** [`2026-07-09-professionalization-review.md`](./2026-07-09-professionalization-review.md) (§6 / Phase E)
**Approach:** Comment tags only (no proc-macro in v0/v1)

---

## 1. Problem

Hadris already cites standards in places (especially UDF `ECMA-167 …` module docs and scattered ISO `ECMA-119` comments), and proves behavior with roundtrips, external-tool interop, and local fuzz corpora. What is missing is **traceability**: a maintainer cannot quickly answer “do we implement §X, how completely, and which test proves it?”

Phase E adds a lightweight compliance program — not formal verification — so claims stay honest and gaps stay visible.

---

## 2. Goals and non-goals

### Goals (v0 — first implementation pass)

1. **Maintainer traceability first.** Annotate spec-facing types/parsers so `rg '@hadris-spec'` answers “where is §X?”
2. **Single workspace index.** Hand-maintain [`docs/spec-coverage.md`](../../spec-coverage.md) (created during implementation) as the audit table for all crates.
3. **Pilot, not completeness.** Tag UDF descriptors (already ECMA-titled) plus key ISO and FAT surfaces; leave full section coverage for later.
4. **Fuzz stays local.** Annotations may *link* fuzz targets; CI never runs fuzz (see Phase D decision).

### Goals (v1 — designed now, implement after v0)

1. CI fails when `@hadris-compliance full` lacks `@hadris-tests` or `@hadris-fuzz`.
2. Optionally fail when an annotated `@hadris-spec` is missing from `docs/spec-coverage.md` (or the reverse).

### Non-goals

- Formal verification or 100% standards coverage before shipping.
- A custom comment DSL / parser before a useful set of annotations exists.
- Replacing roundtrip / external-tool tests — annotations **point at** them.
- A public marketing “compliance matrix” in v0 (the table may be promoted later).
- Full rewrite of `crates/hadris-iso/spec/` in v0 (pointer + light headers only if useful).
- Proc-macros or build-script table generation in v0/v1 (possible v2).

---

## 3. Design principles

1. **Grepable over clever.** Tags are plain text in rustdoc/`//` comments; tooling is `rg` then a small script.
2. **Spec-facing only.** Annotate on-disk structs and public parse/format entry points, not every helper.
3. **Honesty over green cells.** Prefer `partial` + `@hadris-note` over claiming `full`.
4. **Link existing evidence.** Prefer naming tests and fuzz targets that already exist; add tests only when a `full` claim needs them.
5. **One index.** Workspace table in `docs/`; crate READMEs may later link to it, not fork it.

---

## 4. Annotation grammar

Place tags immediately under the human-readable title (keep existing `ECMA-…` prose).

```rust
/// Partition Descriptor (ECMA-167 3/10.5)
///
/// @hadris-spec ECMA-167:3/10.5
/// @hadris-compliance full
/// @hadris-tests comprehensive_udf::partition_descriptor
/// @hadris-fuzz udf_read
```

ISO / FAT example:

```rust
/// Directory Record (ECMA-119 9.1)
///
/// @hadris-spec ECMA-119:9.1
/// @hadris-compliance partial
/// @hadris-tests directory::parse_roundtrip
/// @hadris-note RRIP write ignores RripOptions
```

### Tag reference

| Tag | Required? | Format / values |
|-----|-----------|-----------------|
| `@hadris-spec` | yes | `DOC:section` — e.g. `ECMA-167:3/10.5`, `ECMA-119:9.1`. When no ECMA section applies, use a short stable id (`FAT:BPB`, `FAT:LFN`, `UEFI:GPT`). |
| `@hadris-compliance` | yes | `full` \| `partial` \| `none` \| `n/a` |
| `@hadris-tests` | if claiming `full` (unless fuzz used) | Rust path: `module::test_name` or integration test identity maintainers can `cargo test` |
| `@hadris-fuzz` | optional | Target name under `fuzz/` (documentation link only) |
| `@hadris-note` | if `partial` | One-line gap description |

### Rules

1. One primary `@hadris-spec` per annotated item.
2. `full` ⇒ at least one of `@hadris-tests` or `@hadris-fuzz`.
3. `partial` ⇒ `@hadris-note` required.
4. `none` = known unimplemented at this site; `n/a` = not applicable for this crate/feature.
5. Do not invent a richer mini-language (no nested JSON in comments, no multi-line structured blocks beyond one tag per line).

### What to annotate

| Annotate | Skip |
|----------|------|
| On-disk `#[repr(C)]` / Pod structs that mirror a standard layout | Private helpers, iterators, CLI glue |
| Public `parse` / `read_*` / `write_*` entry points for those layouts | Purely mechanical endian wrappers unless they *are* the spec type |
| Feature-gated paths that change compliance (document with `partial`/`n/a`) | Every call site of an already-annotated type |

---

## 5. Coverage table (`docs/spec-coverage.md`)

Created during v0 implementation. Hand-maintained; regenerated only by humans after `rg '@hadris-spec'`.

Suggested shape:

```markdown
# Spec coverage

Maintainer audit index for standards-facing types. See
`docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`.

Update: `rg '@hadris-spec' crates/` then sync this table.

## hadris-udf

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-167:3/10.5 | `PartitionDescriptor` | full | `…` | `udf_read` | |

## hadris-iso
…

## hadris-fat
…
```

**v0 drift policy:** Slight lag between code tags and the table is acceptable; fix when touching the area or before cutting a release that cites the table. v1 may enforce sync.

**Public use:** Not linked from crate READMEs in v0 unless a later pass promotes it. Internal / contributor-facing first.

---

## 6. Pilot scope (v0 deliverables)

Ordered for reuse of existing ECMA titles and high-traffic parsers.

### 6.1 hadris-udf (first)

Add tags to descriptor modules under `crates/hadris-udf/src/descriptor/` (anchor, partition, primary, logical, fileset, tag, …) and any other already-titled ECMA-167 items that are parse entry points.

Populate the UDF section of `docs/spec-coverage.md`.

### 6.2 hadris-iso

Minimum:

- Primary Volume Descriptor (`PrimaryVolumeDescriptor` / volume path)
- Directory Record (`DirectoryRecord`)
- El-Torito catalog / boot entry **only if** a clear type already exists and tagging is cheap

Optional light touch: one-line pointer from `crates/hadris-iso/spec/Specification.md` (or README in that folder) to `docs/spec-coverage.md`. No full rewrite of in-repo ISO markdown specs in v0.

### 6.3 hadris-fat

Minimum:

- BPB / boot sector layout (`FAT:BPB` or Microsoft/FAT section id if already cited)
- LFN directory entry path (`FAT:LFN`)

### 6.4 Explicitly out of pilot

- Deep exFAT metadata
- Full RRIP write compliance matrix (may appear as a single `partial` row)
- GPT/MBR (`hadris-part`) and CPIO — add later as separate table sections
- Golden vector suite per section (post-v1 “later”)

### 6.5 Docs / process touchpoints

- Short subsection in `CONTRIBUTING.md`: when to add tags; how to update the table; fuzz remains local.
- Pointer from the professionalization review Phase E to this design (done when this file lands).
- Do **not** add a fuzz CI job.

---

## 7. Tooling roadmap

| Version | Deliverable | When |
|---------|-------------|------|
| **v0** | Convention (this doc) + pilot annotations + hand-maintained `docs/spec-coverage.md` + CONTRIBUTING blurb | First Phase E implementation session |
| **v1** | CI check: `full` without tests/fuzz fails; optional table↔annotation sync | After pilot has enough tags to be useful |
| **v2** | Optional generator (script preferred over proc-macro) that emits or checks the markdown table | Only if hand sync becomes painful |
| **Later** | Golden vectors per section; richer in-repo human specs as an index into the same ids | Ongoing |

### v1 CI (implemented)

Script: [`scripts/check-spec-annotations.py`](../../../scripts/check-spec-annotations.py).
CI job: `Spec annotations` in [`.github/workflows/rust.yml`](../../../.github/workflows/rust.yml).

- Input: walk `crates/**/*.rs` for consecutive `@hadris-*` comment lines.
- Fail if any block has `@hadris-compliance full` and neither `@hadris-tests` nor `@hadris-fuzz`.
- Fail if any block has `@hadris-compliance partial` without `@hadris-note`.
- Fail if any `@hadris-spec` value is missing from `docs/spec-coverage.md` (disable with `--no-table-sync`).
- **Never** invoke `cargo fuzz` in CI.

Keep the checker dumb: line-oriented tags, no AST. If that proves too fragile, escalate to v2 — still prefer a script over a proc-macro.

---

## 8. Relationship to the rest of the test pyramid

A mature compliance story combines:

1. **Annotation coverage** (this program) — traceability
2. **Unit / integration tests** named by `@hadris-tests`
3. **External-tool interop** (xorriso, fsck, mkudffs, …) — unchanged
4. **Fuzz corpus promotion** for fixed crashes — local discovery, PR gate via normal tests
5. **Golden vectors per section** — later, keyed by the same `@hadris-spec` ids

Annotations do not replace (2)–(4); they make them findable from the standard section.

---

## 9. Success criteria

**v0 is done when:**

1. This design is the agreed source of truth (this file).
2. UDF pilot descriptors are tagged; ISO PVD + directory record and FAT BPB + LFN are tagged.
3. `docs/spec-coverage.md` exists with those rows and a short “how to update” header.
4. `CONTRIBUTING.md` mentions the convention and that fuzz is not CI.
5. No CI job runs fuzz; no proc-macro added.

**v1 is done when:**

1. A CI job or script enforces the `full` / `partial` tag rules above. ✅
2. Failures are actionable (print file:line and missing tag). ✅

---

## 10. Open questions (defer until implementation)

Resolved for design; revisit only if implementation hits friction:

| Topic | Decision |
|-------|----------|
| Optimize for | Maintainer audit first; public matrix later |
| Table location | `docs/spec-coverage.md` |
| Tooling depth in first pass | v0 ship; v1 designed |
| Mechanism | Comment tags only |
| Spec id for FAT | Prefer existing citations; else stable `FAT:BPB` / `FAT:LFN` |

Resolved during implementation:

- Exact test path strings: discovered while tagging; see `docs/spec-coverage.md`.
- El-Torito: in (`El-Torito:validation` on `BootValidationEntry`).
- v1 table sync: **required** (default on; `--no-table-sync` for local escape hatch only).

---

## 11. Implementation outline (for a later session)

Do **not** start this until Phase E is scheduled. Suggested order:

1. Add `docs/spec-coverage.md` stub + CONTRIBUTING blurb.
2. Tag UDF descriptors; fill UDF table rows.
3. Tag ISO PVD + directory record; fill rows.
4. Tag FAT BPB + LFN; fill rows.
5. Optional: pointer from `crates/hadris-iso/spec/`.
6. Stop. Schedule v1 CI checker separately.

---

## Appendix A — Mapping from review §6

| Review note | This design |
|-------------|-------------|
| `@hadris-spec` convention | §4 |
| v0 hand table | §5 → `docs/spec-coverage.md` |
| v1 CI for `full` | §7 |
| Start UDF + ISO PVD/dir + FAT BPB/LFN | §6 |
| No early custom DSL / no 100% coverage | §2 non-goals |
| Fuzz not CI | §2, §7 |
