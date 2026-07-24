# Compliance source policy

This directory contains Hadris's original, clause-indexed compliance catalogs.
It does not contain copies or mechanical rewrites of upstream specifications.

Authoritative documents and locally extracted text belong under
`.spec-cache/`, which is intentionally ignored. `sources.json` records the
exact edition, official location, retrieval digest, and redistribution status
for every audit input.

## Requirement records

Each `requirements/*.json` file describes one crate's supported profile as
atomic requirements. A requirement records:

- an edition-qualified source and clause;
- a short, independently written engineering summary;
- whether it applies to reading, writing, or both;
- the implementation symbols involved;
- executable evidence;
- a conservative status and an explicit gap where applicable.

`verified` means the stated atomic requirement has direct evidence. It does not
claim that an entire document, profile, or filesystem is conformant.
Round-trip tests alone are not sufficient evidence for validation rules,
reserved fields, checksums, or rejection behavior.

Run:

```bash
python3 scripts/check-compliance-catalog.py --self-test
python3 scripts/check-compliance-catalog.py
```

The checker never downloads specifications and does not require `.spec-cache/`
to exist in CI.
