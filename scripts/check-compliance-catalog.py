#!/usr/bin/env python3
"""Validate source provenance and atomic compliance requirement catalogs.

The authoritative documents are intentionally absent from the repository.
This checker validates tracked metadata and evidence without reading
`.spec-cache/` or accessing the network.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tempfile
from pathlib import Path
from typing import Any


SOURCE_FIELDS = {
    "id",
    "title",
    "issuer",
    "edition",
    "published",
    "official_url",
    "local_path",
    "redistribution",
    "retrieved",
    "sha256",
    "profiles",
}
REQUIREMENT_FIELDS = {
    "id",
    "source",
    "clause",
    "normativity",
    "summary",
    "direction",
    "status",
    "symbols",
    "tests",
    "gap",
}
NORMATIVITY = {"shall", "should", "may", "informative"}
DIRECTIONS = {"read", "write", "both"}
STATUSES = {
    "verified",
    "partial",
    "not_implemented",
    "not_applicable",
    "unknown",
}
TEST_KINDS = {"unit", "integration", "interop", "property", "fuzz"}
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
ID_RE = re.compile(r"^[A-Z0-9][A-Z0-9._-]*:[^#\s]+#[a-z0-9][a-z0-9._-]*$")


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: cannot load JSON: {error}") from error


def relative_repo_path(root: Path, value: str, *, field: str) -> tuple[Path | None, str | None]:
    path = Path(value)
    if path.is_absolute() or ".." in path.parts:
        return None, f"{field} must be a repository-relative path: {value!r}"
    return root / path, None


def validate_sources(root: Path) -> tuple[dict[str, dict[str, Any]], list[str]]:
    path = root / "spec" / "sources.json"
    errors: list[str] = []
    try:
        document = load_json(path)
    except ValueError as error:
        return {}, [str(error)]

    if document.get("schema_version") != 1:
        errors.append(f"{path}: schema_version must be 1")
    rows = document.get("sources")
    if not isinstance(rows, list):
        return {}, [*errors, f"{path}: sources must be an array"]

    sources: dict[str, dict[str, Any]] = {}
    for index, row in enumerate(rows):
        loc = f"{path}:sources[{index}]"
        if not isinstance(row, dict):
            errors.append(f"{loc}: source must be an object")
            continue
        missing = SOURCE_FIELDS - row.keys()
        if missing:
            errors.append(f"{loc}: missing fields: {', '.join(sorted(missing))}")
            continue
        source_id = row["id"]
        if not isinstance(source_id, str) or not source_id:
            errors.append(f"{loc}: id must be a non-empty string")
            continue
        if source_id in sources:
            errors.append(f"{loc}: duplicate source id {source_id!r}")
        sources[source_id] = row
        if not str(row["official_url"]).startswith("https://"):
            errors.append(f"{loc}: official_url must use HTTPS")
        local_path = str(row["local_path"])
        if not local_path.startswith(".spec-cache/sources/"):
            errors.append(f"{loc}: local_path must be under .spec-cache/sources/")
        digest = row["sha256"]
        retrieved = row["retrieved"]
        if (digest is None) != (retrieved is None):
            errors.append(f"{loc}: retrieved and sha256 must both be set or both be null")
        if digest is not None and not SHA256_RE.fullmatch(str(digest)):
            errors.append(f"{loc}: sha256 must be 64 lowercase hexadecimal characters")
        if not isinstance(row["profiles"], list) or not row["profiles"]:
            errors.append(f"{loc}: profiles must be a non-empty array")
    return sources, errors


def has_test_attribute(text: str, name: str) -> bool:
    lines = text.splitlines()
    function = re.compile(rf"\bfn\s+{re.escape(name)}\b")
    attribute = re.compile(r"#\[[^\]]*test[^\]]*\]")
    for index, line in enumerate(lines):
        if function.search(line):
            return any(attribute.search(candidate) for candidate in lines[max(0, index - 6):index])
    return False


def validate_requirement(
    root: Path,
    catalog_path: Path,
    crate: str,
    requirement: Any,
    index: int,
    sources: dict[str, dict[str, Any]],
) -> list[str]:
    loc = f"{catalog_path}:requirements[{index}]"
    if not isinstance(requirement, dict):
        return [f"{loc}: requirement must be an object"]
    errors: list[str] = []
    missing = REQUIREMENT_FIELDS - requirement.keys()
    if missing:
        return [f"{loc}: missing fields: {', '.join(sorted(missing))}"]

    requirement_id = requirement["id"]
    if not isinstance(requirement_id, str) or not ID_RE.fullmatch(requirement_id):
        errors.append(
            f"{loc}: id must be edition-qualified DOCUMENT:clause#claim using stable characters"
        )
    source = requirement["source"]
    if source not in sources:
        errors.append(f"{loc}: unknown source {source!r}")
    elif isinstance(requirement_id, str) and not requirement_id.startswith(f"{source}:"):
        errors.append(f"{loc}: id must start with source id {source!r}")
    if not isinstance(requirement["clause"], str) or not requirement["clause"].strip():
        errors.append(f"{loc}: clause must be a non-empty string")
    if requirement["normativity"] not in NORMATIVITY:
        errors.append(f"{loc}: invalid normativity {requirement['normativity']!r}")
    if requirement["direction"] not in DIRECTIONS:
        errors.append(f"{loc}: invalid direction {requirement['direction']!r}")
    status = requirement["status"]
    if status not in STATUSES:
        errors.append(f"{loc}: invalid status {status!r}")
    summary = requirement["summary"]
    if not isinstance(summary, str) or len(summary.strip()) < 12:
        errors.append(f"{loc}: summary must be an original, meaningful paraphrase")
    if isinstance(summary, str) and len(summary.split()) > 80:
        errors.append(f"{loc}: summary exceeds 80 words; do not reproduce source prose")

    gap = requirement["gap"]
    if status in {"partial", "not_implemented", "not_applicable"}:
        if not isinstance(gap, str) or len(gap.strip()) < 8:
            errors.append(f"{loc}: status {status!r} requires an explicit gap/rationale")
    elif gap is not None:
        errors.append(f"{loc}: gap must be null for status {status!r}")

    symbols = requirement["symbols"]
    if not isinstance(symbols, list) or not symbols:
        errors.append(f"{loc}: symbols must contain at least one implementation mapping")
    else:
        for symbol_index, symbol in enumerate(symbols):
            symbol_loc = f"{loc}:symbols[{symbol_index}]"
            if not isinstance(symbol, dict) or set(symbol) != {"path", "name"}:
                errors.append(f"{symbol_loc}: expected exactly path and name")
                continue
            resolved, path_error = relative_repo_path(
                root, str(symbol["path"]), field=f"{symbol_loc}.path"
            )
            if path_error:
                errors.append(path_error)
            elif resolved is not None and not resolved.is_file():
                errors.append(f"{symbol_loc}: missing implementation file {symbol['path']!r}")
            if not str(symbol["path"]).startswith(f"crates/{crate}/"):
                errors.append(f"{symbol_loc}: path is outside catalog crate {crate!r}")

    tests = requirement["tests"]
    if not isinstance(tests, list):
        errors.append(f"{loc}: tests must be an array")
        tests = []
    direct_evidence = False
    for test_index, test in enumerate(tests):
        test_loc = f"{loc}:tests[{test_index}]"
        if not isinstance(test, dict) or set(test) != {"path", "name", "kind"}:
            errors.append(f"{test_loc}: expected exactly path, name, and kind")
            continue
        if test["kind"] not in TEST_KINDS:
            errors.append(f"{test_loc}: invalid test kind {test['kind']!r}")
        resolved, path_error = relative_repo_path(
            root, str(test["path"]), field=f"{test_loc}.path"
        )
        if path_error:
            errors.append(path_error)
        elif resolved is None or not resolved.is_file():
            errors.append(f"{test_loc}: missing test file {test['path']!r}")
        else:
            text = resolved.read_text(encoding="utf-8")
            if test["kind"] != "fuzz" and not has_test_attribute(text, str(test["name"])):
                errors.append(
                    f"{test_loc}: {test['name']!r} is not a test function in {test['path']!r}"
                )
        if test["kind"] != "fuzz":
            direct_evidence = True
    if status == "verified" and requirement["normativity"] == "shall" and not direct_evidence:
        errors.append(f"{loc}: verified shall requirement needs non-fuzz executable evidence")
    return errors


def validate_catalogs(
    root: Path, sources: dict[str, dict[str, Any]]
) -> list[str]:
    errors: list[str] = []
    seen_ids: dict[str, Path] = {}
    requirements_dir = root / "spec" / "requirements"
    for path in sorted(requirements_dir.glob("*.json")):
        try:
            document = load_json(path)
        except ValueError as error:
            errors.append(str(error))
            continue
        if document.get("schema_version") != 1:
            errors.append(f"{path}: schema_version must be 1")
        crate = document.get("crate")
        if not isinstance(crate, str) or not (root / "crates" / crate).is_dir():
            errors.append(f"{path}: crate must name a directory directly under crates/")
            continue
        requirements = document.get("requirements")
        if not isinstance(requirements, list):
            errors.append(f"{path}: requirements must be an array")
            continue
        for index, requirement in enumerate(requirements):
            errors.extend(
                validate_requirement(root, path, crate, requirement, index, sources)
            )
            if isinstance(requirement, dict) and isinstance(requirement.get("id"), str):
                requirement_id = requirement["id"]
                if requirement_id in seen_ids:
                    errors.append(
                        f"{path}: duplicate requirement id {requirement_id!r}; "
                        f"first declared in {seen_ids[requirement_id]}"
                    )
                else:
                    seen_ids[requirement_id] = path
    return errors


def check_cache_digests(root: Path, sources: dict[str, dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    for source_id, source in sources.items():
        digest = source["sha256"]
        if digest is None:
            continue
        path = root / source["local_path"]
        if not path.is_file():
            errors.append(f"{source_id}: cached source is missing: {path}")
            continue
        computed = hashlib.sha256(path.read_bytes()).hexdigest()
        if computed != digest:
            errors.append(
                f"{source_id}: cached source digest is {computed}, expected {digest}"
            )
    return errors


def run(root: Path, *, check_cache: bool) -> list[str]:
    sources, errors = validate_sources(root)
    errors.extend(validate_catalogs(root, sources))
    if check_cache:
        errors.extend(check_cache_digests(root, sources))
    return errors


def self_test() -> list[str]:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "spec" / "requirements").mkdir(parents=True)
        (root / "crates" / "demo").mkdir(parents=True)
        (root / "crates" / "demo" / "lib.rs").write_text(
            "#[test]\nfn rejects_zero() {}\n", encoding="utf-8"
        )
        (root / "spec" / "sources.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "sources": [
                        {
                            "id": "DEMO-1",
                            "title": "Demo",
                            "issuer": "Demo",
                            "edition": "1",
                            "published": None,
                            "official_url": "https://example.com/spec",
                            "local_path": ".spec-cache/sources/demo",
                            "redistribution": "restricted",
                            "retrieved": None,
                            "sha256": None,
                            "profiles": ["demo"],
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )
        valid = {
            "id": "DEMO-1:2.1#zero-rejected",
            "source": "DEMO-1",
            "clause": "2.1",
            "normativity": "shall",
            "summary": "A zero value is rejected by the parser.",
            "direction": "read",
            "status": "verified",
            "symbols": [{"path": "crates/demo/lib.rs", "name": "parse"}],
            "tests": [
                {
                    "path": "crates/demo/lib.rs",
                    "name": "rejects_zero",
                    "kind": "unit",
                }
            ],
            "gap": None,
        }
        catalog_path = root / "spec" / "requirements" / "demo.json"
        catalog_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "crate": "demo",
                    "profile": "demo",
                    "requirements": [valid],
                }
            ),
            encoding="utf-8",
        )
        errors = run(root, check_cache=False)
        if errors:
            return [f"valid fixture failed: {error}" for error in errors]
        invalid = dict(valid)
        invalid["tests"] = []
        invalid["gap"] = "not allowed for verified"
        document = load_json(catalog_path)
        document["requirements"] = [invalid]
        catalog_path.write_text(json.dumps(document), encoding="utf-8")
        errors = run(root, check_cache=False)
        if len(errors) < 2:
            return ["invalid fixture did not trigger evidence and gap checks"]
    return []


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--check-cache", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    errors = self_test() if args.self_test else run(args.root, check_cache=args.check_cache)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("compliance catalog check: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
