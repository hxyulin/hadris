# Hadris Whole-Workspace Professionalization Review

**Date:** 2026-07-09
**Scope:** Entire workspace (libraries, CLIs, meta-crate, CI, fuzz, specs, docs)
**Method:** Parallel domain deep-dives (FS libs, shared/part core, surface crates, infra/docs), then merge
**Out of scope this session:** Code/doc fixes, SPEC tooling implementation, GitHub issues

Related prior plan (shared crates only): [`plans/2026-07-09-shared-crates-professionalization.md`](../../../plans/2026-07-09-shared-crates-professionalization.md)

---

## 1. Executive summary (top 10)

| Rank | Sev | Finding | Why it matters |
|------|-----|---------|----------------|
| 1 | P0 | Multiple READMEs document **non-existent APIs** (`Fat::open`, `Mbr::read`, `SectorCursor` in `hadris-io`) | First-touch trust failure for crates.io / GitHub visitors |
| 2 | P0 | Root README version pins `0.2`/`0.3` while workspace is **`1.2.1`**; CHANGELOG `[Unreleased]` empty | Release surface looks abandoned or wrong |
| 3 | P0 | `hadris-cli` is installable but a **panic-prone stub** (unwrap + debug dump) | Competes with real CLIs; WIP honesty incomplete |
| 4 | ~~P0~~ | ~~`fuzz/README.md` claims a fuzz CI job~~ — **resolved:** fuzz is documented as local-only (not CI) | — |
| 5 | P1 | ISO README overclaims **Rock Ridge write** (symlinks / POSIX); `RripOptions` unwired | Spec/marketing mismatch |
| 6 | P1 | UDF has **no public file-content read**; `UdfDirEntry.size` is placeholder `0`; CLI `cat`/`extract` blocked | Incomplete product surface for a “supported” format |
| 7 | P1 | CLI naming/UX inconsistent (`fatutil`/`cpioutil` vs `hadris-*-cli`; `ls` vs `list`); **zero CLI tests** | Surface polish lag behind libraries |
| 8 | P1 | `PartitionError::Io` **discards** underlying I/O error; GPT backup header **silently synthesized** on read failure | Debuggability / correctness honesty |
| 9 | P1 | Meta-crate / root README claim **“all formats”** but omit UDF (default), `hadris-cd`, `hadris-part` | Umbrella crate oversells |
| 10 | P1 | In-repo ISO specs WIP; no formal **spec↔test** traceability; async features largely **compile-only** in CI/tests | Hard to claim standards compliance professionally |

**Overall verdict:** Implementation and CI for libraries are strong (feature matrix, Miri, external-tool interop, fuzz harnesses). The main professionalization gap is **surface-layer consistency** — READMEs, version pins, CLI honesty, and process docs that claim jobs/APIs that do not exist.

---

## 2. Cross-cutting themes

### 2.1 Stale and incorrect documentation

Wrong or outdated Quick Starts appear in:

- Root [`README.md`](../../../README.md) — version pins, incomplete crate inventory
- [`crates/hadris-fat/README.md`](../../../crates/hadris-fat/README.md) — `Fat` vs `FatFs`
- [`crates/hadris-part/README.md`](../../../crates/hadris-part/README.md) — phantom `Mbr`/`Gpt`
- [`crates/hadris-io/README.md`](../../../crates/hadris-io/README.md) — phantom `SectorCursor`
- [`crates/hadris-common/README.md`](../../../crates/hadris-common/README.md) — wrong import paths
- [`crates/hadris-fat-cli/README.md`](../../../crates/hadris-fat-cli/README.md) — phantom `extract`/`stats`
- [`crates/hadris-iso-cli/README.md`](../../../crates/hadris-iso-cli/README.md) — incomplete command set / wrong flags
- [`crates/hadris-udf-cli/README.md`](../../../crates/hadris-udf-cli/README.md) — binary name `hadris-udf` vs `hadris-udf-cli`

Crate-level `//!` rustdoc is often better than READMEs (especially fat/iso/cpio). Prefer regenerating README Quick Starts from working doctests.

### 2.2 API consistency across crates

| Pattern | Mature example | Lagging |
|---------|----------------|---------|
| Default features include `read`/`write` | `hadris-fat` | `hadris-part` (`default = ["std"]` only) |
| Preserve I/O error context | `FatError::Io(...)` | `PartitionError::Io` unit variant |
| Dual sync/async via macros | FS crates | exFAT outside sync/async; some ISO introspection sync-only |
| docs.rs feature metadata | `hadris-fat` | part, iso, udf, cpio largely missing |
| Extension-trait I/O | part (`*ReadExt`) | Discoverability weaker than `open()`-style FS APIs |

### 2.3 Spec traceability

- UDF already tags many structs with `ECMA-167 x/y.z` in module/item docs — best existing pattern.
- ISO cites `ECMA-119` in places; in-repo [`crates/hadris-iso/spec/`](../../../crates/hadris-iso/spec/) is incomplete and **excluded from crates.io** (`exclude = ["/spec"]`).
- FAT/CPIO/part rely on informal comments + external references.
- Tests almost never cite section numbers; compliance is proven via roundtrip / external tools, not a coverage matrix.

### 2.4 Testing maturity skew

| Layer | Strength | Gap |
|-------|----------|-----|
| Library unit/integration | Strong (fat/iso/cpio especially) | UDF write→read roundtrip incomplete |
| External tools | xorriso, fsck.fat, mkudffs (when present) | Not all wired as dedicated CI jobs |
| Fuzz | Four targets + corpus | Local-only by design (not PR CI) |
| Miri | Targeted unsafe paths | Scoped by design |
| Async | Feature flags exist | ~zero async integration tests; CI async tiers thin |
| CLIs | — | No tests at all |

---

## 3. Finding catalog

Severity: **P0** = blocking trust / wrong public docs; **P1** = API or compliance gap; **P2** = polish.
Categories: `docs` | `api-ergonomics` | `missing-api` | `spec` | `code-quality` | `tests` | `ci`

### 3.1 Documentation (P0–P2)

| ID | Sev | Cat | Location | Evidence | Suggested follow-up |
|----|-----|-----|----------|----------|---------------------|
| D1 | P0 | docs | `crates/hadris-fat/README.md` | Documents `Fat::open`, `root_dir().iter()`, `FatAnalyzer`; real API is `FatFs::open` / `builder`, `entries()` / `next_entry()`, `FatAnalysisExt` | Rewrite Quick Start; add doctests that compile |
| D2 | P0 | docs | `crates/hadris-part/README.md` | Documents `Mbr::read` / `Gpt::read` and accessors that do not exist | Rewrite against `*ReadExt` / `DiskPartitionScheme` (see shared-crates plan Task 1) |
| D3 | P0 | docs | `crates/hadris-io/README.md` | Documents `hadris_io::SectorCursor`; type lives in `hadris-fat` | Remove or relocate; sync feature table with `Cargo.toml` |
| D4 | P0 | docs | Root `README.md` | `hadris-iso = "0.2"`, `hadris-fat = "0.3"`; omits udf/cpio/cd and several CLIs | Bump pins to workspace version; expand inventory |
| D5 | ~~P0~~ | docs | `fuzz/README.md` | ~~Claims CI fuzz job~~ — **resolved:** local-only workflow documented | — |
| D6 | P1 | docs | `CHANGELOG.md` | Workspace `1.2.1` but `[Unreleased]` empty; last dated release `1.2.0` | Document 1.2.1 delta or retag |
| D7 | P1 | docs | `crates/hadris-iso/README.md` + `src/lib.rs` | RRIP write claimed as full POSIX/symlinks; limitations only in `//` comments | Public Limitations section; downgrade claims |
| D8 | P1 | docs | `crates/hadris-iso/README.md` | `features = ["read"]` as no-alloc bootloader path; high-level `IsoImage` needs `alloc` | Fix feature docs |
| D9 | P1 | docs | `crates/hadris-common/README.md` | Wrong paths for `U16`/`U32` / endian imports | Match `tests/types_integration.rs` |
| D10 | P1 | docs | `crates/hadris-common/src/lib.rs` | Claims sync/async “forwarded to hadris-io” but no public re-export | Re-export or fix wording |
| D11 | P1 | docs | `crates/hadris-part/src/lib.rs` | Calls `read` a “marker feature”; it gates all `*ReadExt` traits | Correct crate docs |
| D12 | P1 | docs | `crates/hadris-macros/README.md` | One-line README; no `io_transform!` cookbook | Expand from CLAUDE.md + fat/part patterns |
| D13 | P1 | docs | `crates/hadris` + root README | “All formats” / “all filesystem implementations” overclaim | Feature table: defaults vs optional; mention cd/part |
| D14 | P1 | docs | `crates/hadris-fat-cli/README.md` | Documents `extract`/`stats`; code has `stat`, no extract | Regenerate from `--help` |
| D15 | P1 | docs | `crates/hadris-iso-cli/README.md` | Lists 4 commands; code has 8; wrong flag names | Regenerate from `--help` |
| D16 | P1 | docs | `crates/hadris-udf-cli/README.md` | Examples use `hadris-udf`; binary is `hadris-udf-cli` | Rename bin or fix docs |
| D17 | P1 | docs | `crates/hadris-udf/README.md` | UDF 2.x “Planned” while writer emits NSR03 for ≥2.00 | Align support matrix with tested write paths |
| D18 | P2 | docs | Multiple crate READMEs | Stale version pins (`1.0`, `0.2`, …) | Workspace-wide pin sync |
| D19 | P2 | docs | No `CONTRIBUTING.md` / `CODE_OF_CONDUCT.md` | Contributor workflow lives only in `CLAUDE.md` | Extract public CONTRIBUTING |
| D20 | P2 | docs | `tests/README.md` | Stub top-level tests dir | Delete or explain purpose |
| D21 | P2 | docs | No `#![deny(missing_docs)]` | Item docs uneven | Gradual enable per crate |

### 3.2 API ergonomics and missing APIs

| ID | Sev | Cat | Location | Evidence | Suggested follow-up |
|----|-----|-----|----------|----------|---------------------|
| A1 | P0 | api-ergonomics | `crates/hadris-cli` | Installable stub with `unwrap()`; no real subcommands | `publish = false` and/or explicit experimental gate; remove install advice until ready |
| A2 | P1 | missing-api | `hadris-udf` `fs.rs` / `file.rs` | `size = 0; // Placeholder`; `UdfFile` has only `size()`, no read | Populate size from ICB; add `read_file` / `open_file` |
| A3 | P1 | missing-api | `hadris-iso` write `File` enum | Only file/dir; no symlink/device despite RRIP write claims | Extend input model + wire RRIP |
| A4 | P1 | api-ergonomics | `hadris-iso` read | Joliet+RRIP written together; reader `best_choice()` picks one root | Document; expose per-namespace roots |
| A5 | P1 | api-ergonomics | `hadris-iso` | Some introspection APIs sync-only (`read_pvd`, boot sections) | Async parity or document |
| A6 | P1 | api-ergonomics | `hadris-part` | Default features omit `read`; desktop examples imply read works | Add `read` to default or always show features |
| A7 | P1 | api-ergonomics | `hadris-part` `error.rs` | `PartitionError::Io` unit; `.map_err(|_| …)` | `Io(hadris_io::Error)` + `source()` |
| A8 | P1 | api-ergonomics | `hadris-io` | No-std `Error` not full `std::io::Error` parity; under-documented | Document parity matrix in lib.rs/README |
| A9 | P1 | api-ergonomics | `hadris-fat` exFAT | Public at crate root, not under sync/async codegen | Document intentional split or migrate |
| A10 | P1 | missing-api | `hadris-fat-cli` | Library has `read_file`, format, exFAT; CLI lacks cat/extract/format | Phase cat/extract then mkfs |
| A11 | P1 | api-ergonomics | CLI binaries | Mixed names: `fatutil`, `cpioutil`, `hadris-*-cli`; `ls` vs `list` | Workspace CLI convention doc |
| A12 | P1 | code-quality | `hadris-cd` `writer.rs` | `fs::read(p).unwrap_or_default()` → empty file on failure | Propagate error |
| A13 | P1 | api-ergonomics | `hadris-cd` | Write-only; no reader/verifier/CLI; weak roundtrip tests | Write→iso+udf reopen tests; optional CLI |
| A14 | P2 | missing-api | Workspace | No `hadris-cd-cli` | Thin `create` wrapper when CD matures |
| A15 | P2 | code-quality | iso-cli / udf-cli | Unused `tracing` deps | Remove or wire `--verbose` |

### 3.3 Spec compliance

| ID | Sev | Cat | Location | Evidence | Suggested follow-up |
|----|-----|-----|----------|----------|---------------------|
| S1 | P0 | spec | `crates/hadris-iso/spec/Specification.md` | Stops at PVD with `(WIP)`; Booting.md unfinished | Finish or mark planned sections; link from rustdoc |
| S2 | P1 | spec | ISO RRIP write | Hardcoded modes/uid/gid; `RripOptions` ignored | Wire options or document non-compliance |
| S3 | P1 | spec | ISO El-Torito write | `// TODO: Create Virtual FAT` for section entries | Implement or document multi-platform limits |
| S4 | P1 | spec | FAT LFN write | Cross-cluster LFN runs rejected (`DirEntryRunTooLong`) | Implement or document max name vs cluster size |
| S5 | P1 | spec | exFAT | Fragmented bitmap/upcase → `UnsupportedFatType` | Document hard limits; add fixtures |
| S6 | P1 | spec | `hadris-part` `scheme_io.rs` | Backup GPT header synthesized on failure without error/flag | Dedicated error or documented fallback |
| S7 | P2 | spec | UDF | `UnsupportedRevision` never constructed; NSR mapping coarse | Use variant or remove |

### 3.4 Tests and CI

| ID | Sev | Cat | Location | Evidence | Suggested follow-up |
|----|-----|-----|----------|----------|---------------------|
| T1 | ~~P0~~ | ci | fuzz vs CI | ~~Missing fuzz CI~~ — **wontfix:** fuzz stays local; PR gate uses unit/integration tests | — |
| T2 | P1 | tests | `hadris-udf` | No hadris write→`UdfFs::open` roundtrip; write tests admit incomplete read | Add V1_02 / V2_01 roundtrips |
| T3 | P1 | tests | All `*-cli` crates | Zero `#[test]` | `--help` + one happy path per CLI; `assert_cmd`/`trycmd` |
| T4 | P1 | tests | `hadris-cd` | `test_basic_writer` only checks `Ok`; no mount/verify | Roundtrip with iso+udf readers |
| T5 | P1 | ci | Feature matrix | Thin/missing `async` tiers for io/part; clippy may not use `-D warnings` uniformly | Align with CLAUDE.md quality checks |
| T6 | P2 | tests | FS crates | No async smoke tests despite `async` features | Minimal open/iterate per crate |
| T7 | P2 | tests | `hadris-part` | No `*ReadExt` I/O tests via `Cursor` | Sync (+ async) Cursor roundtrips |
| T8 | P2 | ci | Toolchain | No root `rust-toolchain.toml`; no `rust-version` in workspace; CLAUDE claims 1.85+ | Pin MSRV in manifest + toolchain file |
| T9 | P2 | ci | Governance | No Dependabot; no docs.rs CI job | Weekly cargo/actions; `cargo doc --workspace` |
| T10 | P2 | docs | docs.rs | Only fat has rich `[package.metadata.docs.rs]` | Mirror for iso/udf/cpio/part |

---

## 4. Per-area summaries

### 4.1 Filesystem libraries

**hadris-iso** — Richest tests and rustdoc; RRIP/Joliet/El-Torito depth. Gaps: overclaimed RRIP write, Joliet+RRIP read selection, sync-only introspection, WIP in-repo spec excluded from crates.io.

**hadris-fat** — Strongest library professionalism (builder, cache/tool, fsck roundtrips, docs.rs). Gaps: **broken README**, LFN cross-cluster limit, exFAT fragmentation limits, exFAT outside dual-async pattern.

**hadris-udf** — Clean descriptor layer and ECMA-167 tags. Gaps: **no file read API**, size placeholder, write→read roundtrip hole, support-matrix messaging lag, dead `UnsupportedRevision`.

**hadris-cpio** — Most honest, focused crate; solid roundtrips and alloc-bomb tests. Gaps: stale version pin, no async tests, no in-repo format notes beyond man-page refs.

### 4.2 Shared / partition core

**hadris-io** — Good lib.rs doctests; README wrong (`SectorCursor`, features). No-std error parity under-documented; async untested in CI.

**hadris-common** — Solid types + Miri coverage; README import paths wrong; “I/O forwarded” overstated.

**hadris-macros** — Critical infrastructure, near-empty README; no edge-case tests for `strip_async!`.

**hadris-part** — Mature on-disk types; P0 README; error context loss; silent backup GPT synthesis; default features omit `read`. Optional `crc`/`rand` deps work as implicit features but are undocumented in `[features]` / README.

### 4.3 Surface crates

| Crate | Verdict |
|-------|---------|
| `hadris-iso-cli` | Strongest CLI; docs lag implementation |
| `hadris-cpio-cli` (`cpioutil`) | Best docs↔code match |
| `hadris-fat-cli` (`fatutil`) | Good analysis commands; README wrong; missing cat/extract/format |
| `hadris-udf-cli` | Honest limitations; blocked on library read; binary name mismatch |
| `hadris-cli` | Not ready; gate or gut |
| `hadris-cd` | Promising writer; silent empty-file bug; no CLI; weak verification |
| `hadris` | Convenient umbrella; overclaims “all formats” |

**CLI command parity (abbreviated):**

| Capability | ISO | FAT | CPIO | UDF | CD |
|------------|-----|-----|------|-----|-----|
| info | yes | yes | yes | yes | — |
| ls/list | ls | ls | list | ls | — |
| tree | yes | yes | — | yes | — |
| cat | yes | — | yes | lib gap | — |
| extract | yes | lib ok | yes | lib gap | — |
| create | yes | lib ok | yes | yes | lib only |
| verify | yes | yes | — | yes | — |

### 4.4 Infra

**Strengths:** Multi-OS tests, feature-tier `-D warnings` checks, Miri job, pre-commit (fmt/clippy), Keep a Changelog, workspace versioning.

**Gaps (remaining):** ISO in-repo spec incomplete; top-level `tests/` stub; optional CoC.
**Addressed in Phase D:** MSRV pin, CONTRIBUTING, Dependabot, docs.rs metadata, CLI smoke + doc CI jobs; fuzz explicitly local-only.

---

## 5. Spec-compliance notes

### What “compliant” means today

Compliance is **behavioral**: roundtrip tests and external tools (xorriso, fsck.fat, mkudffs). There is no section-level coverage matrix tying ECMA/Microsoft/cpio(5) clauses to tests.

### Known mismatches / incompleteness

1. **ISO RRIP write** — not a faithful RRIP writer for permissions/symlinks/devices.
2. **ISO Joliet + RRIP coexistence on read** — single-root selection loses one namespace’s strengths.
3. **ISO El-Torito** — virtual FAT for non-default section entries unfinished.
4. **FAT LFN** — cross-cluster directory entry runs unsupported on write.
5. **exFAT** — fragmented critical metadata unsupported.
6. **UDF file content** — directory walk without content read; sizes stubbed.
7. **GPT backup** — failure path invents a header instead of failing closed.
8. **In-repo ISO spec** — not a usable compliance oracle yet.

### Existing good practice to build on

UDF module docs already look like:

```text
//! Partition Descriptor (ECMA-167 3/10.5)
```

ISO has scattered `ECMA-119 9.1`-style comments. Standardizing and linking these to tests is the natural next step (Section 6).

---

## 6. Future: automated compliance suite (design notes only)

**Goal (later session):** Make “we implement §X” auditable — not a full formal verification suite on day one.

### Proposed annotation convention

Build on existing ECMA tags; prefer a machine-grepable prefix:

```rust
/// @hadris-spec ECMA-167:3/10.5
/// @hadris-compliance full   // or partial | none | n/a
/// @hadris-tests comprehensive_udf::partition_descriptor
/// @hadris-fuzz udf_read
```

For ISO:

```rust
// @hadris-spec ECMA-119:9.1
// @hadris-compliance full
// @hadris-fuzz iso_read
```

**Rules of thumb:**

- One primary `@hadris-spec` per on-disk struct or parser entry point.
- `full` requires at least one `@hadris-tests` **or** `@hadris-fuzz` entry.
- `partial` requires a one-line note of what is missing (e.g. “symlinks not written”).
- Do not annotate every helper; annotate **spec-facing** types and public parse/format paths.

### Phased tooling

| Phase | Deliverable | Effort |
|-------|-------------|--------|
| v0 | Convention + `rg '@hadris-spec'` → hand-maintained `spec-coverage.md` | Low |
| v1 | CI check: `full` without tests/fuzz fails | Medium |
| v2 | Optional proc-macro or build script generating coverage table from annotations | Higher |
| Later | Golden vectors per section; expand in-repo specs as human-readable index | Ongoing |

### What not to do early

- Do not invent a custom DSL inside comments that needs a parser before any annotations exist.
- Do not require 100% section coverage before shipping; start with UDF (already tagged) + ISO directory/PVD + FAT BPB/LFN.
- Do not replace external-tool interop tests — annotations **link** to them.

### Relationship to full test suite

A “full compliance suite” later should combine:

1. Annotation coverage (traceability)
2. Hand-built / corpus fixtures per section
3. Existing roundtrip + external-tool jobs
4. Fuzz corpus promotion for every fixed crash

---

## 7. Recommended workstream order (subsequent sessions)

Ordered for **professional appearance first**, then correctness honesty, then depth.

### Phase A — Release truth (docs only, high ROI)

1. Fix P0 README lies: fat, part, io, root version pins + crate inventory.
2. Populate CHANGELOG for `1.2.1` or clarify versioning.
3. Gate/gut `hadris-cli` messaging; document fuzz as local-only (no CI job).
4. Regenerate CLI READMEs from `--help` (iso, fat, udf binary name).

*Exit:* A new user following any README Quick Start gets compiling examples.

### Phase B — Shared-crate ergonomics

Execute / extend [`plans/2026-07-09-shared-crates-professionalization.md`](../../../plans/2026-07-09-shared-crates-professionalization.md):

- `PartitionError::Io` chaining
- part default features / docs.rs
- macros cookbook
- common/io README accuracy
- Cursor I/O tests for part

### Phase C — Library honesty + missing APIs

1. ISO Limitations in rustdoc; RRIP options or downgraded claims; Joliet/RRIP read docs.
2. UDF file read API + size from ICB → unlock CLI cat/extract.
3. UDF write→read roundtrips; FAT/exFAT limitation docs.
4. `hadris-cd` error propagation + roundtrip verification.

### Phase D — CI / process polish

1. ~~Fuzz CI job~~ — **dropped:** fuzz remains a local developer tool; docs updated accordingly.
2. Async feature tiers + CLI smoke tests.
3. `rust-version` + `rust-toolchain.toml`.
4. CONTRIBUTING, Dependabot, docs.rs metadata, `cargo doc` CI job.

### Phase E — Spec compliance program

1. Finish or restructure ISO `spec/` with coverage headers.
2. Pilot `@hadris-spec` on UDF + key ISO/FAT types (v0 markdown table).
3. Later: CI gate + expanded golden suite.

---

## 8. Strengths to preserve

1. Consistent **no-std + dual sync/async** architecture via `hadris-macros`.
2. **Security-conscious** parsers (ISO size caps, CPIO alloc_bomb, FAT loop guards, fuzz targets).
3. **External interoperability** testing where it matters (xorriso, fsck, mkudffs).
4. **Feature-gated** public surfaces mostly disciplined.
5. **Miri** scoped to historically unsafe paths with CI enforcement.
6. Strong crate-level rustdoc on several libraries (better than READMEs — keep that bar).
7. Existing internal plan for shared crates shows intentional professionalization already underway.

---

## Appendix A — Workspace inventory reviewed

| Member | Role |
|--------|------|
| hadris-io, hadris-common, hadris-macros | Shared foundation |
| hadris-part | MBR/GPT/hybrid |
| hadris-iso, hadris-fat, hadris-udf, hadris-cpio | Filesystems / archives |
| hadris-cd | Hybrid ISO+UDF writer |
| hadris | Meta re-exports |
| hadris-iso-cli, hadris-fat-cli, hadris-cpio-cli, hadris-udf-cli, hadris-cli | CLIs |
| fuzz/, .github/workflows/, .pre-commit-config.yaml | Quality infra |
| crates/hadris-iso/spec/, CHANGELOG, READMEs, CLAUDE.md | Docs / specs |

## Appendix B — Rubric

| Field | Meaning |
|-------|---------|
| Severity | P0 blocking trust / wrong docs; P1 API or compliance gap; P2 polish |
| Category | docs, api-ergonomics, missing-api, spec, code-quality, tests, ci |
| Suggested follow-up | Later session workstream — not implemented in this review |
