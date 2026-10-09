# FAT and exFAT verification

The scheduled proof stage checks 25 properties of production `hadris-fat-raw`
code with Kani 0.68.0. It supplements the independent image oracles and operation models in
the detached `tests/` suite. It does not establish whole-driver conformance.

## Reproduce

```bash
cargo +stable install --locked kani-verifier --version 0.68.0
cargo kani setup
python3 scripts/verify-fat.py

# Validate the inventory without a verifier, or run one group.
python3 scripts/verify-fat.py --check
python3 scripts/verify-fat.py --group geometry
```

The runner checks the requirement catalog and the complete
[`proof inventory`](../spec/proofs/fat.json), then selects each harness by
exact name. Missing or duplicate mappings, mismatched requirement citations,
failed properties, insufficient unwinding bounds, unsatisfied acceptance
covers, zero selected proofs and timeouts fail the run. Kani itself can return
success for an unreachable cover, so the runner checks cover counts explicitly.
Normal Rust builds exclude the harnesses with
`cfg(kani)`; no verifier dependency or allocator is added to library builds.

Reports default to `output/fat-proofs/`. Each harness has a complete compiler
and solver log. `results.json` records the commit, dirty-worktree flag, input
digest, host architecture, verifier version, feature configuration, scopes,
commands, cover counts, statuses and timings. It is updated after each
harness; an interrupted report remains `running` rather than claiming success.
The digest covers crate Rust sources/manifests, the workspace manifest and
lockfile, toolchain, proof inventory, runner, catalog and catalog checker.
The default solver timeout is 300 seconds per harness; `--timeout` changes it.

## Scheduled execution

[`fat-proofs.yml`](../.github/workflows/fat-proofs.yml) runs daily at 03:17 UTC
(11:17 Hong Kong time) and supports manual `workflow_dispatch`. It has no PR
or push trigger. Five independent jobs cover entries, geometry, allocation,
names and checksums, without cancelling other groups on a failure. Every job
installs the pinned verifier, tests the runner and retains reports for 30 days,
including on failure. Workflow permissions are read-only.

Normal PR CI validates the inventory and runner without executing proofs.
The scheduled result is evidence for the tested commit, not a pre-merge gate.
Run the relevant group locally or dispatch the workflow on a branch before
merging a critical change.

GitHub schedules start once the workflow exists on the default branch and run
its latest commit. They may be delayed or dropped under load; public repositories
can have schedules disabled after 60 days without activity. See
[GitHub's schedule documentation](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule).

## Properties and boundaries

The original six codec harnesses are in
[`verification.rs`](../crates/block/hadris-fat-raw/src/verification.rs); broader
properties are split into modules under `src/verification/`.
They invoke production functions and independently express expected byte
layouts, marker constants and checksum arithmetic.

- `fat12_updates_preserve_neighbor`: for every three-byte initial state,
  `u32` value and even/odd entry choice, the stored 12-bit value is correct,
  decoding agrees, and the adjacent entry is unchanged. This checks packing
  at a valid two-byte window, not offset calculation or device I/O.
- `fat16_entries_are_little_endian`: for every initial two-byte state,
  `u32` value and `u64` cluster argument, encoding stores the low 16 bits in
  little-endian order and decoding returns those bits.
- `fat32_updates_preserve_reserved_bits`: for every initial four-byte state,
  `u32` value and `u64` cluster argument, encoding stores the low 28 bits,
  preserves the high four bits and decoding returns the stored word.
  This supplies codec evidence for
  `MICROSOFT-FAT-1.03:fat-data#fat32-reserved-high-bits`.
- `fat_chain_markers_match_specification`: for each FAT width and every
  stored `u32` and maximum-cluster `u32`, classification matches the specified
  end-of-chain and bad-cluster constants and the supplied cluster bounds.
  The only assumption selects one of the three FAT variants. This supplies
  classification evidence for `MICROSOFT-FAT-1.03:fat-data#chain-markers`,
  not evidence of cycle detection or traversal termination.
- `exfat_entry_checksum_matches_specification`: for every 32-byte entry,
  incoming `u16` checksum and `usize` entry index, the checksum update follows
  the rotate-and-add rule and excludes bytes 2 and 3 only in the primary
  entry. The fixed 32-byte loop has an unwind bound of 33. This supplies
  calculation evidence for `MICROSOFT-EXFAT-1.00:file-entry-set#checksum`,
  not evidence that a mounted driver rejects every invalid set.
- `exfat_name_hash_matches_specification`: for every incoming `u16` hash and
  UTF-16 code unit, the update processes the low byte before the high byte
  using the specified arithmetic. The fixed two-byte loop has an unwind
  bound of 3. The caller must already have up-cased the unit; this does not
  verify the up-case table or Unicode conversion.

The additional 19 properties cover:

- **Geometry (five):** FAT12/16 and FAT32 accepted boot layouts keep data,
  FAT entry windows and FAT copies within their volume regions; FAT32 root
  and active-copy selection are valid. Invalid FAT sector/cluster sizes are
  rejected for arbitrary boot bytes. Accepted exFAT geometry bounds the heap,
  FAT, root and cluster offsets and inverts offsets to cluster numbers. The
  explicit formatter layout fits the volume and FAT for all `u32` sector
  counts and all three variants with 512-byte sectors/clusters and default
  metadata. These prove layout safety, not validation of every reserved
  field. Acceptance covers ensure the conditional proofs are not vacuous.
- **Allocation (three):** FAT entry offsets follow the byte formulas across
  the entire `u32` cluster domain. Brent's guard detects cycles in every
  successor graph of four nodes from every start within 16 steps, and never
  reports a cycle before repetition. FSInfo requires all three signatures
  and its encoder preserves every `u32` free-count/next-free hint. The graph
  proof is explicitly bounded and is not a termination theorem for arbitrary
  volume sizes.
- **Names and directory entries (eight):** the full short-name checksum,
  thirteen-unit LFN byte layout and terminator position, full directory-slot
  classification, short-entry cluster extraction, leading-byte escaping,
  ASCII folding, exFAT forbidden-unit classification and FAT timestamp bit
  packing match independent rules. Timestamp packing deliberately masks
  fields; it does not validate calendars. Unicode tables and the complete
  LFN assembly state machine remain outside these properties.
- **Checksums (three):** boot-checksum calculation matches the independent
  recurrence over every 113-byte prefix and every incoming `u32` sum and
  sector index, including the three excluded mutable fields. Up-case-table
  checksums cover every prefix of length zero through eight; name hashes
  cover every up-cased name of zero through eight code units. These bounds
  are not claims for whole boot regions, tables or maximum-length names.

The specification anchors are the FAT 1.03 FAT Data Structure section and
exFAT 1.00 sections 6.3.3 and 7.6.4. Source editions and provenance remain in
[`sources.json`](../spec/sources.json). The mathematical expectations require
review against those documents; a solver verifies the encoded property, not
its interpretation of the standard.

The codec properties use finite input domains with complete fixed-length
loop unwinding. The graph and variable-prefix proofs additionally impose the
explicit bounds above. Proofs assume correctly sized buffers, as required
by the codec APIs. Kani, its compiler frontend and solver are trusted; the
proofs do not verify the Rust compiler, device behavior, async cancellation,
disk durability or all undefined behavior. Miri and existing failure and
cancellation tests remain necessary.

## Next verification tiers

1. Audit the supported FAT/exFAT profiles clause by clause. Separate
   specification requirements from compatibility policies, including the
   reader's acceptance of 64 KiB FAT clusters. Catalog coverage is not a
   clause-completeness claim.
2. Extend geometry proofs to additional formatter configurations and add
   independent malformed-image cases for every required rejection rule.
3. Run requirement evidence under explicit Cargo targets and features,
   checking runtime discovery and results rather than source names alone.
4. Extend the existing operation models and interruption tests across
   hosted/embedded and sync/async drivers, checking invariants with the
   independent raw-image oracles. Model write failure, cancellation and
   power loss separately, with explicit atomicity and flush assumptions.
5. Use deliberate implementation mutations to check that the relevant
   proof or conformance test fails. Consider inductive verification for
   allocation and recovery algorithms when finite-state exploration no
   longer scales.

See the [Kani usage guide](https://model-checking.github.io/kani/usage.html),
[unwinding guide](https://model-checking.github.io/kani/tutorial-loop-unwinding.html)
and [feature support](https://model-checking.github.io/kani/rust-feature-support.html)
for the verifier's guarantees and limitations.
