#!/usr/bin/env python3
"""Validate @hadris-* spec annotation blocks (Phase E v1).

Rules (see docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md §7):
  - @hadris-compliance full  ⇒ @hadris-tests and/or @hadris-fuzz
  - @hadris-compliance partial ⇒ @hadris-note
  - every @hadris-spec value appears in docs/spec-coverage.md (unless --no-table-sync)

Line-oriented only — no Rust AST. Never invokes cargo fuzz.
"""

from __future__ import annotations

import argparse
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
        if "hadris-tests" not in tags and "hadris-fuzz" not in tags:
            cline = block.tag_lines.get("hadris-compliance", block.start_line)
            errors.append(
                f"{block.path}:{cline}: @hadris-compliance full requires "
                f"@hadris-tests and/or @hadris-fuzz"
            )
    elif compliance == "partial":
        if "hadris-note" not in tags or not tags["hadris-note"]:
            cline = block.tag_lines.get("hadris-compliance", block.start_line)
            errors.append(
                f"{block.path}:{cline}: @hadris-compliance partial requires "
                f"@hadris-note"
            )

    return errors


def specs_in_coverage(coverage_path: Path) -> set[str]:
    text = coverage_path.read_text(encoding="utf-8")
    # Table cells: | ECMA-167:3/10.5 | ...
    found: set[str] = set()
    for line in text.splitlines():
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if not cells or cells[0] in {"Spec", "------", ""}:
            continue
        if cells[0].startswith("*") or cells[0].startswith("("):
            continue
        # Skip markdown separator rows
        if set(cells[0]) <= {"-", ":"}:
            continue
        found.add(cells[0])
    return found


def check_table_sync(blocks: list[Block], coverage_path: Path) -> list[str]:
    if not coverage_path.is_file():
        return [f"missing coverage table: {coverage_path}"]

    table_specs = specs_in_coverage(coverage_path)
    errors: list[str] = []
    seen: set[str] = set()

    for block in blocks:
        spec = block.tags.get("hadris-spec")
        if not spec:
            continue
        if spec in seen:
            continue
        seen.add(spec)
        if spec not in table_specs:
            line = block.tag_lines.get("hadris-spec", block.start_line)
            errors.append(
                f"{block.path}:{line}: @hadris-spec {spec} missing from "
                f"{coverage_path} (add a table row or fix the id)"
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

    if table_sync:
        errors.extend(check_table_sync(all_blocks, root / coverage_rel))

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

    # fuzz alone satisfies full
    fuzz_only = parse_blocks(
        path,
        "/// @hadris-spec X\n/// @hadris-compliance full\n/// @hadris-fuzz udf_read\n",
    )[0]
    assert check_block(fuzz_only) == []

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
            "See docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md",
            file=sys.stderr,
        )
        return 1

    n_files = len(iter_rust_files(root))
    print(f"Spec annotation check passed ({n_files} Rust files under crates/).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
