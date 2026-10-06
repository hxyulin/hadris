#!/usr/bin/env python3
"""Plans a release of one or more published workspace crates.

usage: scripts/release-plan.py [--notes SECTION] [--notes-dir DIR] CRATE... | all

Prints one line per crate in dependency order: name, version and tag
(`<crate>-v<version>`). Fails when a crate is not published, a version is
not a semantic version, a tag already names another commit, or the
CHANGELOG.md section is missing, undated or empty.

Release notes come from the section `## [<crate> <version>] - YYYY-MM-DD`,
or from `## [SECTION] - YYYY-MM-DD` for every crate with --notes SECTION
(a joint release such as `--notes 3.0.0`). With --notes-dir, the notes of
each crate are written to DIR/<crate>.md.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def run(*args: str) -> str:
    return subprocess.run(args, cwd=ROOT, check=True, capture_output=True, text=True).stdout


def published_packages() -> dict:
    metadata = json.loads(run("cargo", "metadata", "--no-deps", "--format-version", "1"))
    return {p["name"]: p for p in metadata["packages"] if p.get("publish") != []}


def dependency_order(packages: dict, selected: list[str]) -> list[str]:
    """Orders `selected` so every crate follows the workspace crates it
    needs to build (normal and build dependencies)."""
    deps = {
        name: {
            d["name"]
            for d in packages[name]["dependencies"]
            if d.get("path") and d.get("kind") != "dev" and d["name"] in selected
        }
        for name in selected
    }
    order: list[str] = []
    while deps:
        ready = sorted(name for name, needs in deps.items() if needs <= set(order))
        if not ready:
            raise SystemExit(f"dependency cycle among {sorted(deps)}")
        order.extend(ready)
        for name in ready:
            del deps[name]
    return order


def changelog_section(text: str, title: str) -> str:
    heading = re.compile(rf"^## \[{re.escape(title)}\] - (.*)$", re.MULTILINE)
    matches = list(heading.finditer(text))
    if len(matches) != 1:
        raise ValueError(f"CHANGELOG.md needs exactly one `## [{title}] - YYYY-MM-DD` section")
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", matches[0].group(1).strip()):
        raise ValueError(f"CHANGELOG.md section [{title}] has no release date")
    start = matches[0].end()
    following = re.compile(r"^## \[", re.MULTILINE).search(text, start)
    body = text[start : following.start() if following else len(text)].strip()
    if not body:
        raise ValueError(f"CHANGELOG.md section [{title}] is empty")
    return body + "\n"


def tag_commit(tag: str) -> str | None:
    result = subprocess.run(
        ["git", "rev-list", "-n", "1", tag], cwd=ROOT, capture_output=True, text=True
    )
    return result.stdout.strip() if result.returncode == 0 else None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("crates", nargs="+")
    parser.add_argument("--notes", default="")
    parser.add_argument("--notes-dir", type=Path)
    args = parser.parse_args()

    packages = published_packages()
    if args.crates == ["all"]:
        selected = sorted(packages)
    else:
        selected = sorted(set(args.crates))
        unknown = [name for name in selected if name not in packages]
        if unknown:
            print(f"not published workspace crates: {' '.join(unknown)}", file=sys.stderr)
            return 1

    head = run("git", "rev-parse", "HEAD").strip()
    changelog = (ROOT / "CHANGELOG.md").read_text()
    errors = []
    plan = []
    for name in dependency_order(packages, selected):
        version = packages[name]["version"]
        tag = f"{name}-v{version}"
        if not SEMVER.match(version):
            errors.append(f"{name}: {version} is not a semantic version")
        commit = tag_commit(tag)
        if commit and commit != head:
            errors.append(f"{name}: tag {tag} already names {commit}")
        try:
            notes = changelog_section(changelog, args.notes or f"{name} {version}")
        except ValueError as error:
            errors.append(f"{name}: {error}")
            notes = ""
        plan.append((name, version, tag, notes))

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    if args.notes_dir:
        args.notes_dir.mkdir(parents=True, exist_ok=True)
        for name, _, _, notes in plan:
            (args.notes_dir / f"{name}.md").write_text(notes)
    for name, version, tag, _ in plan:
        print(name, version, tag)
    return 0


if __name__ == "__main__":
    sys.exit(main())
