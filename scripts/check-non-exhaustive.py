#!/usr/bin/env python3
"""Report public enums and public-field structs that lack #[non_exhaustive].

Enforces V3 stability rule R1 (docs/v3-api-design.md). Scans the library
crates under crates/{core,block,optical,archive}/*/src and reports:

- every `pub enum` without `#[non_exhaustive]`
- every `pub struct` with at least one `pub` field without `#[non_exhaustive]`

Skipped: modules named `raw` (file `raw.rs`, directory `raw/`, or inline
`mod raw { .. }`), `#[cfg(test)]` inline modules, files named `tests.rs`,
`macro_rules!` bodies, and items marked `#[repr(C..)]` or
`#[repr(transparent)]` (on-disk layouts, R4).

This is a text heuristic, not a Rust parser. Known limits:
- Visibility is taken literally: a `pub` item in a private module is still
  reported, and items created by macros (other than the source they expand
  from) are not seen.
- `cfg_attr(.., non_exhaustive)` counts as present.
- Attributes are the text between the previous `;`, `{` or `}` and the item,
  so an attribute containing braces can confuse it.
- Test-only modules declared as `#[cfg(test)] mod name;` in another file are
  scanned unless the file is named `tests.rs`.

Exits 1 when anything is reported.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GROUPS = ("core", "block", "optical", "archive")

ITEM = re.compile(r"(?m)^[ \t]*pub[ \t]+(enum|struct)[ \t]+([A-Za-z_][A-Za-z0-9_]*)")
PUB_FIELD = re.compile(r"\bpub\b(?!\s*\()")
SKIP_BLOCK = re.compile(
    r"\bmod\s+raw\s*\{"
    r"|#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*(?:#\s*\[[^\]]*\]\s*)*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+\w+\s*\{"
    r"|\bmacro_rules!\s*\w+\s*\{"
)
REPR_LAYOUT = re.compile(r"#\s*\[\s*repr\s*\(\s*(?:C\b|transparent\b)")


def strip_comments_and_strings(src: str) -> str:
    out = list(src)
    i, n = 0, len(src)

    def blank(a: int, b: int) -> None:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif (m := re.compile(r'b?r(#*)"').match(src, i)) and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            end = src.find('"' + m.group(1), m.end())
            j = n if end < 0 else end + 1 + len(m.group(1))
            blank(m.end(), j - 1 - len(m.group(1)))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            blank(i + 1, min(j, n))
            i = j + 1
        elif c == "'":
            m = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'").match(src, i)
            if m:
                blank(i + 1, m.end() - 1)
                i = m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


def matching(src: str, open_idx: int, open_ch: str, close_ch: str) -> int:
    depth = 0
    for k in range(open_idx, len(src)):
        if src[k] == open_ch:
            depth += 1
        elif src[k] == close_ch:
            depth -= 1
            if depth == 0:
                return k
    return len(src) - 1


def skip_ranges(src: str) -> list[tuple[int, int]]:
    ranges = []
    for m in SKIP_BLOCK.finditer(src):
        start = m.end() - 1
        ranges.append((m.start(), matching(src, start, "{", "}")))
    return ranges


def attributes_before(src: str, idx: int) -> str:
    k = idx - 1
    while k >= 0 and src[k] not in ";{}":
        k -= 1
    return src[k + 1 : idx]


def has_pub_field(src: str, after_name: int) -> bool:
    k, angle = after_name, 0
    while k < len(src):
        ch = src[k]
        if ch == "<":
            angle += 1
        elif ch == ">" and angle:
            angle -= 1
        elif angle == 0 and ch in "{(;":
            break
        k += 1
    if k >= len(src) or src[k] == ";":
        return False
    close = matching(src, k, src[k], "}" if src[k] == "{" else ")")
    return bool(PUB_FIELD.search(src, k + 1, close))


def scan_source(src: str) -> list[tuple[int, str, str]]:
    clean = strip_comments_and_strings(src)
    skips = skip_ranges(clean)
    findings = []
    for m in ITEM.finditer(clean):
        if any(a <= m.start() <= b for a, b in skips):
            continue
        attrs = attributes_before(clean, m.start())
        if "non_exhaustive" in attrs or REPR_LAYOUT.search(attrs):
            continue
        kind, name = m.group(1), m.group(2)
        if kind == "struct" and not has_pub_field(clean, m.end()):
            continue
        line = clean.count("\n", 0, m.start()) + 1
        findings.append((line, kind, name))
    return findings


def source_files(root: Path) -> list[tuple[str, Path]]:
    files = []
    for group in GROUPS:
        for crate in sorted((root / "crates" / group).glob("*/")):
            src = crate / "src"
            if not src.is_dir():
                continue
            for path in sorted(src.rglob("*.rs")):
                rel = path.relative_to(src)
                if "raw" in rel.parts[:-1] or path.stem in ("raw", "tests"):
                    continue
                files.append((crate.name, path))
    return files


def self_test() -> int:
    sample = """
/// doc with pub enum Fake
pub enum Bad { A }
#[non_exhaustive]
pub enum Good { A }
#[derive(Debug)]
#[non_exhaustive]
pub struct GoodS { pub a: u8 }
pub struct Private { a: u8, pub(crate) b: u8 }
pub struct BadS<T: Into<u8>> where T: Copy { pub a: T }
pub struct BadT(pub u8);
#[repr(C)]
pub struct Disk { pub a: u8 }
#[repr(transparent)]
pub struct Wrap(pub u32);
pub(crate) enum Hidden { A }
const S: &str = "pub enum InString { A }";
mod raw { pub enum InRaw { A } }
#[cfg(test)]
mod tests { pub enum InTest { A } }
macro_rules! m { () => { pub enum InMacro { A } } }
pub enum AfterAll { A }
"""
    got = [(k, n) for _, k, n in scan_source(sample)]
    want = [("enum", "Bad"), ("struct", "BadS"), ("struct", "BadT"), ("enum", "AfterAll")]
    if got != want:
        print(f"self-test failed: got {got}, want {want}", file=sys.stderr)
        return 1
    print("self-test passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--summary", action="store_true", help="print per-crate counts only")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    counts: Counter[str] = Counter()
    for crate, path in source_files(args.root):
        text = path.read_text(encoding="utf-8")
        for line, kind, name in scan_source(text):
            counts[crate] += 1
            if not args.summary:
                rel = path.relative_to(args.root)
                print(f"{rel}:{line}: pub {kind} {name} lacks #[non_exhaustive]")

    if counts:
        print("\nR1 findings per crate:")
        for crate, count in sorted(counts.items()):
            print(f"  {crate}: {count}")
        print(f"  total: {sum(counts.values())}")
        return 1
    print("no R1 findings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
