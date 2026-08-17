#!/usr/bin/env python3

from __future__ import annotations

import re
import sys
import tempfile
import tomllib
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?\[[^]]*\]\(([^)\s]+)(?:\s+['\"][^)]*['\"])?\)")
EXTERNAL = ("http://", "https://", "mailto:")


def documentation_files() -> list[Path]:
    files = [ROOT / "README.md"]
    files.extend((ROOT / "crates").glob("**/README.md"))
    files.extend((ROOT / "website" / "docs").glob("**/*.md"))
    return sorted(files)


def resolve_link(source: Path, raw_target: str, root: Path = ROOT) -> Path | None:
    target = unquote(raw_target.strip("<>"))
    if target.startswith(EXTERNAL) or target.startswith("#"):
        return None
    target = target.split("#", 1)[0]
    if not target:
        return None
    if target.startswith("/img/"):
        return root / "website" / "static" / target.removeprefix("/")
    if target.startswith("/"):
        return None
    return (source.parent / target).resolve()


def check_links(files: list[Path], root: Path = ROOT) -> list[str]:
    root = root.resolve()
    errors = []
    for source in files:
        source = source.resolve()
        for line_number, line in enumerate(source.read_text().splitlines(), 1):
            for match in LINK.finditer(line):
                raw_target = match.group(1)
                target = resolve_link(source, raw_target, root)
                if target is None:
                    continue
                relative_source = source.relative_to(root)
                if not target.is_relative_to(root):
                    errors.append(
                        f"{relative_source}:{line_number}: link target escapes repository {raw_target}"
                    )
                elif not target.exists():
                    errors.append(
                        f"{relative_source}:{line_number}: missing link target {raw_target}"
                    )
    return errors


def self_test() -> int:
    with tempfile.TemporaryDirectory() as directory:
        temporary_root = Path(directory).resolve()
        root = temporary_root / "repository"
        docs = root / "docs"
        docs.mkdir(parents=True)
        (temporary_root / "outside.md").write_text("outside\n")
        (docs / "target.md").write_text("target\n")
        source = docs / "source.md"
        source.write_text("[valid](target.md)\n[escape](../../outside.md)\n")
        errors = check_links([source], root)
    expected = [
        "docs/source.md:2: link target escapes repository ../../outside.md"
    ]
    if errors != expected:
        print(f"self-test failed: {errors!r}", file=sys.stderr)
        return 1
    print("documentation checker self-test passed")
    return 0


def check_package_readmes() -> list[str]:
    errors = []
    for manifest in sorted((ROOT / "crates").glob("**/Cargo.toml")):
        package = tomllib.loads(manifest.read_text()).get("package", {})
        readme_value = package.get("readme")
        if not isinstance(readme_value, str):
            continue
        readme = (manifest.parent / readme_value).resolve()
        if not readme.exists():
            errors.append(f"{manifest.relative_to(ROOT)}: package README does not exist")
            continue
        if readme == ROOT / "README.md":
            continue
        contents = readme.read_text()
        relative = readme.relative_to(ROOT)
        if "## Documentation" not in contents:
            errors.append(f"{relative}: missing Documentation section")
        if "LICENSE-MIT" not in contents:
            errors.append(f"{relative}: missing MIT license link")
    return errors


def check_active_names(files: list[Path]) -> list[str]:
    errors = []
    stale_paths = ("release-candidate.md", "read-and-create-iso.md")
    for source in files:
        contents = source.read_text()
        for stale in stale_paths:
            if stale in contents:
                errors.append(f"{source.relative_to(ROOT)}: stale documentation path {stale}")
    if (ROOT / "website" / "docs" / "release-candidate.md").exists():
        errors.append("website/docs/release-candidate.md: use stability.md")
    return errors


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if sys.argv[1:]:
        print("usage: check-docs.py [--self-test]", file=sys.stderr)
        return 2
    files = documentation_files()
    errors = check_links(files)
    errors.extend(check_package_readmes())
    errors.extend(check_active_names(files))
    if errors:
        print("documentation checks failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(f"checked {len(files)} documentation files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
