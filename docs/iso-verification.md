# ISO 9660 verification

The scheduled proof stage checks 13 properties of production `hadris-iso-raw`
and `hadris-iso` code with Kani 0.68.0. It supplements the raw ECMA-119 oracle,
peer interoperability tests and fuzzing described in the
[compliance profile](compliance/hadris-iso.md). It does not establish
whole-driver conformance.

## Reproduce

```bash
cargo +stable install --locked kani-verifier --version 0.68.0
cargo kani setup
python3 scripts/verify-proofs.py iso

# Validate every inventory without a verifier, or run one group.
python3 scripts/verify-proofs.py --check
python3 scripts/verify-proofs.py iso --group records
```

The runner is shared with the [FAT/exFAT proofs](fat-verification.md) and
behaves identically: it checks the requirement catalog and the complete
[`proof inventory`](../spec/proofs/iso.json), then selects each harness by
exact name. Missing or duplicate mappings, a workflow matrix that omits a
group, mismatched requirement citations, failed properties, insufficient
unwinding bounds, unsatisfied acceptance covers, zero selected proofs and
timeouts fail the run.

The inventory lists both crates with their Cargo features. `hadris-iso-raw`
harnesses build with no features; `hadris-iso` harnesses build with
`--no-default-features --features alloc,sync`, an existing CI feature tier,
because the name conversion they check needs the writer's planner. Normal
Rust builds exclude the harnesses with `cfg(kani)`; neither crate gains a
verifier dependency.

Reports default to `output/iso-proofs/` with the same `results.json` format
as the FAT stage, including each harness's crate and features. The default
solver timeout is 300 seconds per harness.

## Scheduled execution

[`iso-proofs.yml`](../.github/workflows/iso-proofs.yml) runs daily at 03:47 UTC
and supports manual `workflow_dispatch`, with no PR or push trigger. Six
independent jobs cover fields, records, dates, susp, boot and names. Every job
installs the pinned verifier, tests the runner and retains reports for 30 days,
including on failure. Workflow permissions are read-only. PR CI validates both
inventories and the runner without executing proofs. The scheduling caveats
in the [FAT guide](fat-verification.md#scheduled-execution) apply equally.

## Properties and boundaries

Harnesses live in `src/verification.rs` and `src/verification/` of each
crate. They call production functions and state the expected byte layouts,
acceptance rules and arithmetic independently.

- **Fields (three):** every byte image of the both-byte-order types
  `U16Both`/`U32Both` decodes its little-endian half with `get`, its
  big-endian half with `get_be`, and is consistent exactly when the halves
  agree; writing any value stores both encodings. The single-byte-order types
  and both path-table header forms match their byte layouts, and the record
  length is the header, identifier and one padding byte for odd identifiers.
- **Records (two):** `DirectoryRecord::parse` is checked against an
  independent acceptance predicate for every 256-byte buffer and every input
  length from 0 to 256: the record length is at least 34, even and available;
  the identifier is non-empty and fits; reserved flags are clear; the three
  both-byte-order fields agree; and an even-length identifier is followed by a
  zero padding byte. Accepted records copy exactly their bytes, and the name
  and system use views cover exactly the specified regions, checked at a
  symbolic index rather than by element-wise loops. `DirectoryRecord::new`
  succeeds exactly when the record fits 255 bytes, and its output parses back
  with the same name and the system use area padded to an even length.
- **Dates (three):** the seven-byte directory date and seventeen-byte
  descriptor date decoders accept exactly the calendar-valid field values
  (leap years included) for every input, record the offset, apply it to the
  instant, and keep hundredths. The descriptor digit codec writes every value
  the encoder produces as fixed-width ASCII digits and parses exactly the
  all-digit strings back.
- **SUSP (one):** the raw `SuspEntries` iterator is compared with an
  independent model of the entry walk for every area of up to 32 bytes and
  every `usize` skip. Each yielded entry is the in-bounds slice its length
  byte describes, the walk stops at an `ST` entry or a length that is too
  small or too large, it stays stopped, and it terminates within one step per
  four bytes.
- **Boot (two):** for every 32-byte validation entry, the checksum the writer
  computes excludes only the checksum word, and an entry is valid exactly when
  its header ID is 1, its key is `55 AA` and its sixteen words sum to zero.
  `BootInfoTable::word_sum`, which the writer uses for the boot information
  table, equals the sum of every byte shifted by its position in its 32-bit
  word; a trailing partial word is therefore counted zero-padded, the fix
  made in #299. Splitting the input at any multiple of four bytes gives the
  same sum, which justifies the writer's chunked loop. These cover images of
  up to 16 bytes.
- **Names (two):** `name::version` accepts exactly a trailing `;` followed by
  one or more digits with a value from 1 to 32767, in both single-byte and
  UCS-2 identifiers of up to 8 bytes, and `name::strip_version` removes exactly
  that suffix and a `.` ending an empty extension. Every Unicode scalar value
  becomes a d-character under the default `NameCase::Upper`, and an
  alphanumeric or underscore under `NameCase::Preserve`.

The specification anchors are ECMA-119 (2nd edition) clauses 7.2-7.6, 8.4.26.1,
9.1 and 9.4, El Torito 1.0 section 2.1 and SUSP 1.12 sections 4.1 and 5.4.
The El Torito and SUSP documents have not been retrieved into `.spec-cache/`;
their clause numbers need review against the source.
Source editions and provenance are in [`sources.json`](../spec/sources.json).
The boot information table is a mkisofs convention without a standard; its
proof carries no requirement ID.

### Findings

- The date decoders accept any offset whose magnitude stays within a day,
  -95 to +95 quarter hours, rather than only the specified -48 to +52. The
  proofs encode this tolerance and the catalog records it as a gap on
  `8.4.26.1#decimal-date-ranges` and `9.1.5#recording-date`.
- `NameCase::Preserve` deliberately records lowercase letters; only the
  default produces strict d-characters (`7.5.1#d-characters`).

### Not covered

- **Driver Rock Ridge scan.** `hadris-iso` walks system use areas with its own
  `rock_ridge::Scan::feed`, not `SuspEntries`. A harness comparing it with the
  same walk model did not finish within 10 minutes even for 8-byte areas with
  the date decoders stubbed: symbolic execution of its entry dispatch and
  `SL`/`TF` body loops grows too large. The catalog marks
  `IEEE-P1281-1.12:4.1#entry-bounds` partial. Building the scan on a proven raw
  walker is the natural next step.
- **Date round trips.** Re-encoding a decoded date requires proving that the
  `hadris-fs` calendar conversions invert each other over every supported
  year. Neither MiniSat nor Kissat finished within 20 minutes, so encoder
  calendar fields remain covered by unit tests. The proofs above cover the
  decoders and the digit codec.
- **Identifier assembly.** The ISO 9660:1999 207-byte limit and Level 1/2
  identifier assembly operate on allocated strings longer than bounded model
  checking handles efficiently; unit tests cover them.
- Descriptor-set traversal, path-table ordering, sector boundaries,
  multi-extent files and continuation-area chasing involve device I/O and
  remain covered by the oracle and integration tests.

Kani, its compiler frontend and solver are trusted, and the proofs assume the
buffers the APIs document. Miri and existing tests remain necessary.
