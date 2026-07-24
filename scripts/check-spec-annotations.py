#!/usr/bin/env python3
"""Validate @hadris-* spec annotation blocks.

Rules (see docs/spec-coverage.md):
  - @hadris-compliance full  ⇒ @hadris-tests
  - @hadris-compliance partial ⇒ @hadris-note
  - cited tests and fuzz targets exist
  - annotations and docs/spec-coverage.md agree in both directions

Line-oriented only — no Rust AST. Never invokes cargo fuzz.
"""

from __future__ import annotations

import argparse
from collections import Counter
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

TAG_RE = re.compile(
    r"^\s*(?://|///|\*)?\s*"
    r"@(hadris-(?:spec|compliance|tests|fuzz|note))\s*(.*)$"
)
COMPLIANCE_VALUES = frozenset({"full", "partial", "none", "n/a"})


@dataclass
class Block:
    path: Path
    start_line: int
    tags: dict[str, str] = field(default_factory=dict)
    tag_lines: dict[str, int] = field(default_factory=dict)

    def add(self, name: str, value: str, line: int) -> None:
        # First occurrence wins; duplicates are reported separately.
        if name not in self.tags:
            self.tags[name] = value.strip()
            self.tag_lines[name] = line


@dataclass(frozen=True)
class CoverageRow:
    spec: str
    compliance: str
    tests: str
    fuzz: str
    notes: str


def iter_rust_files(root: Path) -> list[Path]:
    crates = root / "crates"
    if not crates.is_dir():
        return []
    return sorted(p for p in crates.rglob("*.rs") if p.is_file())


def parse_blocks(path: Path, text: str) -> list[Block]:
    blocks: list[Block] = []
    current: Block | None = None

    for lineno, line in enumerate(text.splitlines(), start=1):
        m = TAG_RE.match(line)
        if m:
            name, value = m.group(1), m.group(2)
            if current is None:
                current = Block(path=path, start_line=lineno)
            current.add(name, value, lineno)
            continue
        if current is not None:
            blocks.append(current)
            current = None

    if current is not None:
        blocks.append(current)
    return blocks


def check_block(block: Block) -> list[str]:
    errors: list[str] = []
    tags = block.tags
    loc = f"{block.path}:{block.start_line}"

    if "hadris-spec" not in tags:
        # Orphan tag cluster (e.g. only @hadris-note) — still validate if compliance present.
        if "hadris-compliance" not in tags:
            return errors
        errors.append(f"{loc}: annotation block missing @hadris-spec")

    compliance = tags.get("hadris-compliance")
    if compliance is None:
        if "hadris-spec" in tags:
            errors.append(f"{loc}: @hadris-spec without @hadris-compliance")
        return errors

    if compliance not in COMPLIANCE_VALUES:
        cline = block.tag_lines.get("hadris-compliance", block.start_line)
        errors.append(
            f"{block.path}:{cline}: invalid @hadris-compliance {compliance!r} "
            f"(expected one of {', '.join(sorted(COMPLIANCE_VALUES))})"
        )
        return errors

    if compliance == "full":
        if "hadris-tests" not in tags or not tags["hadris-tests"]:
            cline = block.tag_lines.get("hadris-compliance", block.start_line)
            errors.append(
                f"{block.path}:{cline}: @hadris-compliance full requires "
                "@hadris-tests; fuzzing alone is not conformance evidence"
            )
    elif compliance == "partial":
        if "hadris-note" not in tags or not tags["hadris-note"]:
            cline = block.tag_lines.get("hadris-compliance", block.start_line)
            errors.append(
                f"{block.path}:{cline}: @hadris-compliance partial requires "
                f"@hadris-note"
            )

    return errors


def coverage_rows(coverage_path: Path) -> list[CoverageRow]:
    text = coverage_path.read_text(encoding="utf-8")
    found: list[CoverageRow] = []
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if len(cells) != 6 or cells[0] in {"Spec", "------", ""}:
            continue
        if cells[0].startswith("*") or cells[0].startswith("("):
            continue
        # Skip markdown separator rows
        if set(cells[0]) <= {"-", ":"}:
            continue
        found.append(
            CoverageRow(
                spec=cells[0].strip("`"),
                compliance=cells[2].strip("`"),
                tests=cells[3],
                fuzz=cells[4].strip("`"),
                notes=cells[5],
            )
        )
    return found


def check_table_sync(blocks: list[Block], coverage_path: Path) -> list[str]:
    if not coverage_path.is_file():
        return [f"missing coverage table: {coverage_path}"]

    rows = coverage_rows(coverage_path)
    errors: list[str] = []
    annotated = [
        block
        for block in blocks
        if block.tags.get("hadris-spec") and block.tags.get("hadris-compliance")
    ]
    annotation_counts = Counter(
        (block.tags["hadris-spec"], block.tags["hadris-compliance"])
        for block in annotated
    )
    table_counts = Counter((row.spec, row.compliance) for row in rows)

    for key in sorted(annotation_counts.keys() | table_counts.keys()):
        annotation_count = annotation_counts[key]
        table_count = table_counts[key]
        if annotation_count != table_count:
            spec, compliance = key
            errors.append(
                f"{coverage_path}: {spec} ({compliance}) has {table_count} table "
                f"row(s) but {annotation_count} annotation block(s)"
            )

    for block in annotated:
        matching = [
            row
            for row in rows
            if row.spec == block.tags["hadris-spec"]
            and row.compliance == block.tags["hadris-compliance"]
        ]
        for tag_name, row_field in (
            ("hadris-tests", "tests"),
            ("hadris-fuzz", "fuzz"),
            ("hadris-note", "notes"),
        ):
            value = block.tags.get(tag_name, "")
            if not value:
                continue
            if tag_name in {"hadris-tests", "hadris-fuzz"}:
                agrees = any(
                    set(evidence_names(value))
                    <= set(evidence_names(getattr(row, row_field)))
                    for row in matching
                )
            else:
                agrees = any(value == row.notes for row in matching)
            if not agrees:
                line = block.tag_lines.get(tag_name, block.start_line)
                errors.append(
                    f"{block.path}:{line}: @{tag_name} value {value!r} does not "
                    f"match the {coverage_path} row for {block.tags['hadris-spec']}"
                )
    return errors


def evidence_names(value: str) -> list[str]:
    return [part.strip().strip("`") for part in value.split(",") if part.strip()]


def check_evidence(root: Path, blocks: list[Block]) -> list[str]:
    errors: list[str] = []
    rust_text = "\n".join(
        path.read_text(encoding="utf-8") for path in iter_rust_files(root)
    )
    fuzz_targets = {
        path.stem for path in (root / "fuzz" / "fuzz_targets").glob("*.rs")
    }

    for block in blocks:
        for reference in evidence_names(block.tags.get("hadris-tests", "")):
            function = reference.rsplit("::", 1)[-1]
            if not re.search(rf"\bfn\s+{re.escape(function)}\b", rust_text):
                line = block.tag_lines.get("hadris-tests", block.start_line)
                errors.append(
                    f"{block.path}:{line}: cited test {reference!r} does not name "
                    "a Rust test function"
                )
        for target in evidence_names(block.tags.get("hadris-fuzz", "")):
            if target not in fuzz_targets:
                line = block.tag_lines.get("hadris-fuzz", block.start_line)
                errors.append(
                    f"{block.path}:{line}: cited fuzz target {target!r} does not "
                    "exist under fuzz/fuzz_targets"
                )
    return errors


def check_coverage_evidence(root: Path, coverage_path: Path) -> list[str]:
    rust_text = "\n".join(
        path.read_text(encoding="utf-8") for path in iter_rust_files(root)
    )
    fuzz_targets = {
        path.stem for path in (root / "fuzz" / "fuzz_targets").glob("*.rs")
    }
    errors: list[str] = []
    for row in coverage_rows(coverage_path):
        for reference in evidence_names(row.tests):
            function = reference.rsplit("::", 1)[-1]
            if not re.search(rf"\bfn\s+{re.escape(function)}\b", rust_text):
                errors.append(
                    f"{coverage_path}: {row.spec} cites test {reference!r}, "
                    "which does not name a Rust test function"
                )
        for target in evidence_names(row.fuzz):
            if target not in fuzz_targets:
                errors.append(
                    f"{coverage_path}: {row.spec} cites missing fuzz target {target!r}"
                )
    return errors


def run_checks(
    root: Path,
    *,
    table_sync: bool,
    coverage_rel: str = "docs/spec-coverage.md",
) -> list[str]:
    errors: list[str] = []
    all_blocks: list[Block] = []

    for path in iter_rust_files(root):
        text = path.read_text(encoding="utf-8")
        for block in parse_blocks(path, text):
            all_blocks.append(block)
            errors.extend(check_block(block))

    errors.extend(check_evidence(root, all_blocks))

    if table_sync:
        coverage_path = root / coverage_rel
        errors.extend(check_table_sync(all_blocks, coverage_path))
        errors.extend(check_coverage_evidence(root, coverage_path))

    return errors


def _self_test() -> None:
    """Minimal fixture checks (no repo walk)."""
    sample = """
/// @hadris-spec ECMA-TEST:1
/// @hadris-compliance full
/// @hadris-tests foo::bar

/// @hadris-spec ECMA-TEST:2
/// @hadris-compliance full

/// @hadris-spec ECMA-TEST:3
/// @hadris-compliance partial

/// @hadris-spec ECMA-TEST:4
/// @hadris-compliance partial
/// @hadris-note gap
"""
    path = Path("fixture.rs")
    blocks = parse_blocks(path, sample)
    assert len(blocks) == 4, blocks

    e0 = check_block(blocks[0])
    assert e0 == [], e0
    e1 = check_block(blocks[1])
    assert any("full requires" in e for e in e1), e1
    e2 = check_block(blocks[2])
    assert any("partial requires" in e for e in e2), e2
    e3 = check_block(blocks[3])
    assert e3 == [], e3

    # Fuzzing is robustness evidence, not sufficient conformance evidence.
    fuzz_only = parse_blocks(
        path,
        "/// @hadris-spec X\n/// @hadris-compliance full\n/// @hadris-fuzz udf_read\n",
    )[0]
    assert any("fuzzing alone" in error for error in check_block(fuzz_only))

    assert evidence_names("foo::one, `bar::two`") == ["foo::one", "bar::two"]

    print("self-test: ok")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="Repository root (default: parent of scripts/)",
    )
    parser.add_argument(
        "--no-table-sync",
        action="store_true",
        help="Skip checking @hadris-spec ids against docs/spec-coverage.md",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run embedded fixture checks and exit",
    )
    args = parser.parse_args(argv)

    if args.self_test:
        _self_test()
        return 0

    root = args.root
    if root is None:
        root = Path(__file__).resolve().parent.parent
    root = root.resolve()

    errors = run_checks(root, table_sync=not args.no_table_sync)
    if errors:
        print("Spec annotation check failed:\n", file=sys.stderr)
        for err in errors:
            print(f"  {err}", file=sys.stderr)
        print(
            f"\n{len(errors)} error(s). "
            "See docs/spec-coverage.md#annotation-convention",
            file=sys.stderr,
        )
        return 1

    n_files = len(iter_rust_files(root))
    print(f"Spec annotation check passed ({n_files} Rust files under crates/).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
