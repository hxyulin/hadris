# Spec Compliance Program v0 Pilot — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the Phase E v0 pilot — grepable `@hadris-spec` tags on UDF/ISO/FAT hot paths, a hand-maintained `docs/spec-coverage.md`, and a CONTRIBUTING blurb — with no CI fuzz job and no proc-macros.

**Architecture:** Comment-only annotations on spec-facing types (design: [`docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`](../specs/2026-07-09-spec-compliance-program-design.md)). One workspace markdown index; tags link existing tests/fuzz targets. Maintainer audit first; public matrix later.

**Tech Stack:** Rust rustdoc/`//` comments, `rg`, Markdown, existing `cargo test` / local `cargo fuzz` targets (`udf_read`, `iso_read`, `fat_read`).

## Global Constraints

- Follow annotation grammar in the design doc §4 exactly (`@hadris-spec`, `@hadris-compliance`, `@hadris-tests` / `@hadris-fuzz`, `@hadris-note` for `partial`).
- `full` requires at least one of `@hadris-tests` or `@hadris-fuzz`; prefer naming real existing tests.
- Prefer `partial` + honest `@hadris-note` over claiming `full` when write paths or options are incomplete.
- Annotate **spec-facing** structs / parse entry points only — not every helper.
- Do **not** add a fuzz CI job, proc-macro, or table generator in v0.
- Do **not** rewrite `crates/hadris-iso/spec/` beyond a one-line pointer.
- Out of pilot: exFAT depth, full RRIP write matrix, GPT/MBR, CPIO, golden vectors, v1 CI checker.
- Keep `RUSTFLAGS="-D warnings"` green; comment-only changes should not alter behavior.
- Design docs under `docs/superpowers/specs/` may still be untracked — include them in the first commit of this plan if not already on `main`.

---

## File structure (v0)

| Path | Role |
|------|------|
| `docs/spec-coverage.md` | **Create** — workspace audit table (UDF / ISO / FAT sections) |
| `CONTRIBUTING.md` | **Modify** — short “Spec annotations” subsection |
| `docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md` | Ensure committed (source of truth) |
| `docs/superpowers/specs/2026-07-09-professionalization-review.md` | Ensure Phase E pointer committed |
| `crates/hadris-udf/src/descriptor/{tag,anchor,primary,partition,logical,fileset,mod}.rs` | Add tags on ECMA-167 types |
| `crates/hadris-iso/src/volume.rs` | Tag `PrimaryVolumeDescriptor` |
| `crates/hadris-iso/src/directory.rs` | Tag `DirectoryRecord` / header |
| `crates/hadris-iso/src/boot.rs` | Tag El-Torito validation entry **only if cheap** (see Task 4) |
| `crates/hadris-iso/spec/Specification.md` | One-line pointer to coverage doc |
| `crates/hadris-fat/src/raw.rs` | Tag `RawBpb` (`FAT:BPB`) and `RawLfnEntry` (`FAT:LFN`) |

No new Rust modules, no CI workflow changes in v0.

---

### Task 1: Scaffold coverage doc + CONTRIBUTING + commit design specs

**Files:**
- Create: `docs/spec-coverage.md`
- Modify: `CONTRIBUTING.md`
- Add (if untracked): `docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`, `docs/superpowers/specs/2026-07-09-professionalization-review.md`, this plan file

**Interfaces:**
- Produces: Empty-but-structured coverage table and contributor instructions that later tasks fill in.

- [ ] **Step 1: Create `docs/spec-coverage.md` stub**

```markdown
# Spec coverage

Maintainer audit index for standards-facing types in Hadris.
Not a public marketing matrix (v0).

**Design:** [`docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`](superpowers/specs/2026-07-09-spec-compliance-program-design.md)

**How to update**

1. `rg '@hadris-spec' crates/`
2. Sync rows below (one primary row per annotated item).
3. Prefer `partial` + Notes over claiming `full`.

Fuzz columns name targets under `fuzz/` (local only — not PR CI).

## hadris-udf

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| *(pilot rows added in Task 2)* | | | | | |

## hadris-iso

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| *(pilot rows added in Task 3–4)* | | | | | |

## hadris-fat

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| *(pilot rows added in Task 5)* | | | | | |
```

- [ ] **Step 2: Add CONTRIBUTING subsection** (after “Safety and fuzzing”)

```markdown
## Spec annotations

When changing on-disk layouts or public parse/format entry points for a
standard section, add or update `@hadris-spec` tags (see
[`docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`](docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md))
and sync [`docs/spec-coverage.md`](docs/spec-coverage.md).

- `full` needs `@hadris-tests` and/or `@hadris-fuzz`.
- `partial` needs `@hadris-note` describing the gap.
- Fuzz targets are local discovery tools, not CI gates.
```

- [ ] **Step 3: Verify design docs exist and review pointer is present**

Run: `test -f docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md && rg -n 'spec-compliance-program-design' docs/superpowers/specs/2026-07-09-professionalization-review.md`
Expected: file exists; review mentions the design.

- [ ] **Step 4: Commit**

```bash
git add docs/spec-coverage.md CONTRIBUTING.md \
  docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md \
  docs/superpowers/specs/2026-07-09-professionalization-review.md \
  docs/superpowers/plans/2026-07-09-spec-compliance-v0-pilot.md
git commit -m "$(cat <<'EOF'
docs: scaffold Phase E v0 spec coverage and contributor guide

Add hand-maintained coverage table stub, CONTRIBUTING annotation
blurb, and commit the approved design/plan for the compliance pilot.
EOF
)"
```

---

### Task 2: Tag hadris-udf descriptors (pilot core)

**Files:**
- Modify: `crates/hadris-udf/src/descriptor/tag.rs`
- Modify: `crates/hadris-udf/src/descriptor/anchor.rs`
- Modify: `crates/hadris-udf/src/descriptor/primary.rs`
- Modify: `crates/hadris-udf/src/descriptor/partition.rs`
- Modify: `crates/hadris-udf/src/descriptor/logical.rs` (`LogicalVolumeDescriptor`, `Type1PartitionMap`)
- Modify: `crates/hadris-udf/src/descriptor/fileset.rs`
- Modify: `crates/hadris-udf/src/descriptor/mod.rs` (shared building blocks: `ExtentDescriptor`, `LongAllocationDescriptor`, `ShortAllocationDescriptor`, `EntityIdentifier`, `CharSpec` — tag only if they are clearly spec-facing and already titled; skip `LbAddr` / tiny helpers if noisy)
- Modify: `docs/spec-coverage.md` (UDF section rows)

**Interfaces:**
- Consumes: Existing ECMA-167 titles already on these modules.
- Produces: Grepable tags + UDF table rows. Suggested test ids (integration crate `comprehensive_udf`):
  - Tag / checksum: `comprehensive_udf::constants_tests` / `test_tag_structure`, `test_tag_checksum`, `test_descriptor_tag_ids`
  - AVDP: `comprehensive_udf::…::test_avdp_structure` (use the actual module path from the file — today nested under `mod` blocks; prefer the **runnable** filter string, e.g. document as `comprehensive_udf` + test name, or `cargo test -p hadris-udf --test comprehensive_udf test_avdp_structure`)
  - Prefer stable form in tags: `comprehensive_udf::test_avdp_structure` (test name unique in that file)
  - Fuzz: `udf_read` for all UDF parse-facing rows

**Compliance guidance (default honesty):**
- Layout + `validate` / checksum paths that are exercised → `full` with tests + `udf_read`.
- If write path invents or stubs fields → `partial` + note (check write module briefly; do not expand write work in this task).

- [ ] **Step 1: Add tags to `DescriptorTag` and `TagIdentifier` in `tag.rs`**

Example shape (adapt to existing rustdoc):

```rust
/// Descriptor tag (ECMA-167 3/7.2)
///
/// @hadris-spec ECMA-167:3/7.2
/// @hadris-compliance full
/// @hadris-tests comprehensive_udf::test_tag_structure
/// @hadris-fuzz udf_read
pub struct DescriptorTag { ... }
```

- [ ] **Step 2: Tag `AnchorVolumeDescriptorPointer`, `PrimaryVolumeDescriptor`, `PartitionDescriptor`, `LogicalVolumeDescriptor`, `Type1PartitionMap`, `FileSetDescriptor`**

Use section ids already in module titles:
- AVDP → `ECMA-167:3/10.2`
- PVD → `ECMA-167:3/10.1`
- Partition → `ECMA-167:3/10.5`
- LVD → `ECMA-167:3/10.6`
- Type 1 map → `ECMA-167:3/10.7.2`
- File Set → `ECMA-167:4/14.1`

- [ ] **Step 3: Optionally tag shared types in `mod.rs`** (`ExtentDescriptor` `ECMA-167:3/7.1`, long/short AD, `EntityIdentifier`, `CharSpec`) — only if a clear test or fuzz link exists; otherwise leave for a later pass to avoid empty `full` claims.

- [ ] **Step 4: Fill UDF rows in `docs/spec-coverage.md`**

- [ ] **Step 5: Verify tags are grepable**

Run: `rg -n '@hadris-spec ECMA-167' crates/hadris-udf/src/descriptor`
Expected: one hit per tagged item (≥6).

Run: `RUSTFLAGS='-D warnings' cargo check -p hadris-udf --no-default-features --features 'read,sync'`
Expected: success.

- [ ] **Step 6: Commit**

```bash
git add crates/hadris-udf/src/descriptor docs/spec-coverage.md
git commit -m "$(cat <<'EOF'
docs(udf): add @hadris-spec tags for ECMA-167 descriptor pilot

Annotate volume/file-set descriptor types and sync the UDF section of
docs/spec-coverage.md for maintainer traceability.
EOF
)"
```

---

### Task 3: Tag hadris-iso PVD + directory record

**Files:**
- Modify: `crates/hadris-iso/src/volume.rs` (`PrimaryVolumeDescriptor` ~line 329)
- Modify: `crates/hadris-iso/src/directory.rs` (`DirectoryRecordHeader` / `DirectoryRecord`)
- Modify: `docs/spec-coverage.md` (ISO section)

**Interfaces:**
- Tests (integration `comprehensive_iso`):
  - PVD: `comprehensive_iso::test_pvd_standard_identifier`, `test_volume_id_extraction`
  - Directory: `comprehensive_iso::test_directory_record_structure`, `test_root_directory_access`
- Fuzz: `iso_read`

**Compliance guidance:**
- PVD read layout → likely `full` if tests cover identifier/volume id.
- Directory record → `full` for basic ECMA-119 9.1 layout **or** `partial` if RRIP/Joliet coexistence caveats belong on this type (prefer a short note rather than overclaiming).

- [ ] **Step 1: Tag `PrimaryVolumeDescriptor`**

```rust
/// @hadris-spec ECMA-119:8.4
/// @hadris-compliance full
/// @hadris-tests comprehensive_iso::test_pvd_standard_identifier
/// @hadris-fuzz iso_read
```

(Confirm section number against existing comments / ECMA-119 PVD clause; if the codebase already cites a different section, use that id consistently.)

- [ ] **Step 2: Tag `DirectoryRecord` (and header if it is the layout carrier)**

```rust
/// @hadris-spec ECMA-119:9.1
/// @hadris-compliance full   # or partial + note
/// @hadris-tests comprehensive_iso::test_directory_record_structure
/// @hadris-fuzz iso_read
```

- [ ] **Step 3: Update ISO rows in `docs/spec-coverage.md`**

- [ ] **Step 4: Verify**

Run: `rg -n '@hadris-spec ECMA-119' crates/hadris-iso/src`
Expected: ≥2 hits.

Run: `RUSTFLAGS='-D warnings' cargo check -p hadris-iso --no-default-features --features 'read,sync'`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add crates/hadris-iso/src/volume.rs crates/hadris-iso/src/directory.rs docs/spec-coverage.md
git commit -m "$(cat <<'EOF'
docs(iso): tag PVD and directory record for spec coverage pilot

Link ECMA-119 sections to comprehensive_iso tests and iso_read fuzz.
EOF
)"
```

---

### Task 4: El-Torito — include only if cheap

**Files:**
- Modify (conditional): `crates/hadris-iso/src/boot.rs` (`BootValidationEntry` and/or `BaseBootCatalog`)
- Modify (conditional): `docs/spec-coverage.md`
- Modify: `crates/hadris-iso/spec/Specification.md` (pointer — always do this step even if El-Torito is skipped)

**Decision gate:** If `BootValidationEntry` already has a clear struct + parse path and a nearby test (`xorriso_boot` or unit tests), tag with `@hadris-spec El-Torito:validation` (or the id used in `Booting.md`) as `partial`/`full` with note. If tagging requires inventing tests or large docs, **skip El-Torito** and record “skipped — not cheap” in the PR description.

- [ ] **Step 1: Decide include/skip** after 2-minute skim of `boot.rs` + `tests/xorriso_boot.rs`

- [ ] **Step 2a (include):** Add tags + coverage row; fuzz `iso_read` if applicable

- [ ] **Step 2b (skip):** No code change in `boot.rs`

- [ ] **Step 3: Add pointer at top of `crates/hadris-iso/spec/Specification.md`**

```markdown
> Maintainer machine-index for implemented sections:
> [`docs/spec-coverage.md`](../../../docs/spec-coverage.md).
```

- [ ] **Step 4: Commit**

```bash
git add crates/hadris-iso/spec/Specification.md docs/spec-coverage.md
# plus boot.rs if tagged
git commit -m "$(cat <<'EOF'
docs(iso): link in-repo spec notes to workspace coverage table

Optional El-Torito tags only when the validation entry path is cheap.
EOF
)"
```

---

### Task 5: Tag hadris-fat BPB + LFN

**Files:**
- Modify: `crates/hadris-fat/src/raw.rs` (`RawBpb` ~line 13, `RawLfnEntry` ~line 281)
- Modify: `docs/spec-coverage.md` (FAT section)

**Interfaces:**
- Spec ids: `FAT:BPB`, `FAT:LFN` (stable; Microsoft FAT names already appear as `BPB_*` / `LFN_*` field comments)
- Tests: `comprehensive_fat::test_valid_sector_sizes`, `test_detect_fat32` (BPB); `comprehensive_fat::test_lfn_builder_sequence`, `test_unicode_filename` (LFN)
- Fuzz: `fat_read`
- LFN is feature-gated (`lfn`); note that in compliance/`@hadris-note` if claiming depends on the feature.

**Compliance guidance:**
- `RawBpb` read validation heavily tested → `full` + `fat_read`.
- `RawLfnEntry` + builder: if cross-cluster LFN write still unsupported (Known Limitations), use `partial` with note e.g. `cross-cluster LFN runs unsupported on write`.

- [ ] **Step 1: Tag `RawBpb`**

```rust
/// @hadris-spec FAT:BPB
/// @hadris-compliance full
/// @hadris-tests comprehensive_fat::test_valid_sector_sizes
/// @hadris-fuzz fat_read
```

- [ ] **Step 2: Tag `RawLfnEntry`**

```rust
/// @hadris-spec FAT:LFN
/// @hadris-compliance partial
/// @hadris-tests comprehensive_fat::test_lfn_builder_sequence
/// @hadris-fuzz fat_read
/// @hadris-note cross-cluster LFN directory runs unsupported on write
```

(Adjust note to match current Known Limitations wording.)

- [ ] **Step 3: Fill FAT rows in `docs/spec-coverage.md`**

- [ ] **Step 4: Verify**

Run: `rg -n '@hadris-spec FAT:' crates/hadris-fat/src/raw.rs`
Expected: 2 hits.

Run: `RUSTFLAGS='-D warnings' cargo check -p hadris-fat --no-default-features --features 'read,sync,lfn'`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add crates/hadris-fat/src/raw.rs docs/spec-coverage.md
git commit -m "$(cat <<'EOF'
docs(fat): tag BPB and LFN entries for spec coverage pilot

Use stable FAT:BPB / FAT:LFN ids and link comprehensive_fat + fat_read.
EOF
)"
```

---

### Task 6: Final sync + v0 acceptance check

**Files:**
- Modify: `docs/spec-coverage.md` (ensure no stub placeholders remain)
- Optionally: one-line mention in root `README.md` under contributing — **skip unless** you already touch README; design says internal-first.

- [ ] **Step 1: Inventory tags vs table**

Run:

```bash
rg -n '@hadris-spec' crates/hadris-udf crates/hadris-iso crates/hadris-fat
rg -n '^\| ECMA-|^\| FAT:|^\| El-' docs/spec-coverage.md
```

Expected: every `@hadris-spec` value appears as a table row; every `full` row has Tests or Fuzz; every `partial` row has Notes.

- [ ] **Step 2: Spot-check `full` rule manually**

Run: `rg -n '@hadris-compliance full' -A2 crates/hadris-udf crates/hadris-iso crates/hadris-fat`
Expected: each block shows `@hadris-tests` and/or `@hadris-fuzz` within the next few lines.

- [ ] **Step 3: Workspace warning check (touched crates)**

Run:

```bash
RUSTFLAGS='-D warnings' cargo check -p hadris-udf -p hadris-iso -p hadris-fat
```

Expected: success.

- [ ] **Step 4: Confirm no fuzz CI / no proc-macro**

Run: `rg -n 'cargo fuzz|hadris_spec|spec-coverage' .github/workflows || true`
Expected: no fuzz job; no new macro.

- [ ] **Step 5: Final commit if table polish needed**

```bash
git add docs/spec-coverage.md
git commit -m "$(cat <<'EOF'
docs: finish Phase E v0 spec-coverage table sync

EOF
)"
```

(Skip empty commit if already clean.)

- [ ] **Step 6: Open PR (feature branch → main)** summarizing pilot scope and explicit outs (exFAT, part, cpio, v1 CI).

---

## v0 done checklist (from design §9)

- [ ] Design doc is on the branch / `main`
- [ ] UDF descriptors tagged
- [ ] ISO PVD + directory record tagged
- [ ] FAT BPB + LFN tagged
- [ ] `docs/spec-coverage.md` filled for those rows
- [ ] `CONTRIBUTING.md` mentions the convention + fuzz not CI
- [ ] No fuzz CI job; no proc-macro

**Stop.** Do not start v1 CI checker in this plan.

---

## Self-review (plan author)

1. **Spec coverage:** Maps design §6 pilot (UDF → ISO → FAT → docs) and §9 success criteria; excludes §6.4 outs and v1 tooling.
2. **Placeholder scan:** Test path strings use real `comprehensive_*` names; implementer must confirm nested `mod` paths when writing tags (filter by test function name if module path is awkward).
3. **Type consistency:** Spec ids use `ECMA-167:…`, `ECMA-119:…`, `FAT:BPB` / `FAT:LFN` as decided in design §10.
4. **Ambiguity:** El-Torito is explicitly a cheapness gate (Task 4), not required for v0 done.
5. **Scope:** Comment + markdown only; no behavior changes expected.
