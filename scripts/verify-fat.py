#!/usr/bin/env python3
"""Check the FAT proof inventory or execute it with pinned Kani and reports."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
from pathlib import Path
import re
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = Path("spec/proofs/fat.json")
GROUPS = ("entries", "geometry", "allocation", "names", "checksums")
PROOF = re.compile(r"#\[kani::proof\]\s*(?:#\[[^\]]*\]\s*)*fn\s+(\w+)")


def inventory(root: Path) -> dict:
    document = json.loads((root / MANIFEST).read_text())
    if document["schema_version"] != 1 or document["crate"] != "hadris-fat-raw" or document["features"]:
        raise ValueError("unsupported proof manifest configuration")
    base = Path("crates/block/hadris-fat-raw/src")
    discovered = {}
    for path in sorted((root / base).rglob("*.rs")):
        names = PROOF.findall(path.read_text())
        if not names:
            continue
        relative = path.relative_to(root / base)
        if relative != Path("verification.rs") and relative.parts[0] != "verification":
            raise ValueError(f"proof harness outside verification modules: {relative}")
        parts = list(relative.with_suffix("").parts)
        if parts[-1] == "mod":
            parts.pop()
        prefix = "::".join(parts)
        for name in names:
            harness = f"{prefix}::{name}"
            if harness in discovered:
                raise ValueError(f"duplicate proof harness: {harness}")
            discovered[harness] = str(path.relative_to(root))
    rows = document["harnesses"]
    names = [row["harness"] for row in rows]
    if not names or len(names) != len(set(names)) or set(names) != set(discovered):
        raise ValueError("proof manifest must cover every harness exactly once")
    requirements = json.loads((root / "spec/requirements/hadris-fat.json").read_text())["requirements"]
    requirement_ids = {row["id"] for row in requirements}
    for row in rows:
        if row["path"] != discovered[row["harness"]] or row["group"] not in GROUPS:
            raise ValueError(f"invalid proof mapping: {row['harness']}")
        if not row["scope"].strip() or not set(row["requirements"]) <= requirement_ids:
            raise ValueError(f"invalid proof scope or requirement: {row['harness']}")
        citations = {
            requirement["id"] for requirement in requirements
            if any(evidence["kind"] == "proof" and evidence["path"] == row["path"]
                   and evidence["name"] == row["harness"].split("::")[-1]
                   for evidence in requirement["tests"])
        }
        if citations != set(row["requirements"]):
            raise ValueError(f"proof citations disagree with requirement catalog: {row['harness']}")
        if not isinstance(row["covers"], int) or row["covers"] < 0:
            raise ValueError(f"invalid cover count: {row['harness']}")
    return document


def input_digest(root: Path) -> str:
    paths = set((root / "crates").rglob("*.rs")) | set((root / "crates").rglob("Cargo.toml"))
    paths.update(root / path for path in (
        "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", str(MANIFEST),
        "scripts/verify-fat.py", "scripts/check-compliance-catalog.py", "spec/requirements/hadris-fat.json",
    ))
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(root)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def run_harness(root: Path, row: dict, report_dir: Path, timeout: int) -> dict:
    log = report_dir / (row["harness"].replace("::", "-") + ".log")
    command = [
        "cargo", "kani", "-p", "hadris-fat-raw", "--lib", "--no-default-features",
        "--exact", "--harness", row["harness"], "--output-format", "terse",
        "-Z", "unstable-options", "--harness-timeout", f"{timeout}s",
    ]
    started = time.monotonic()
    status = "failed"
    with log.open("w") as output:
        try:
            result = subprocess.run(command, cwd=root, stdout=output, stderr=subprocess.STDOUT, timeout=timeout + 120)
            returncode = result.returncode
        except subprocess.TimeoutExpired:
            returncode = None
            status = "timeout"
    text = log.read_text()
    covers = [(int(passed), int(total)) for passed, total in re.findall(
        r"(\d+) of (\d+) cover properties satisfied", text
    )]
    covers_passed = sum(total for _, total in covers) == row["covers"] and all(
        passed == total for passed, total in covers
    )
    if returncode == 0 and "VERIFICATION:- SUCCESSFUL" in text and re.search(
        r"1 successfully verified harnesses, 0 failures, 1 total", text
    ) and covers_passed:
        status = "passed"
    return {**row, "status": status, "returncode": returncode,
            "seconds": round(time.monotonic() - started, 3), "log": log.name, "command": command,
            "satisfied_covers": sum(passed for passed, _ in covers)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="validate inventory without running Kani")
    parser.add_argument("--group", choices=GROUPS)
    parser.add_argument("--report-dir", type=Path, default=Path("output/fat-proofs"))
    parser.add_argument("--timeout", type=int, default=300, help="solver seconds per harness")
    args = parser.parse_args(argv)
    if args.timeout < 1:
        parser.error("--timeout must be positive")
    try:
        document = inventory(ROOT)
    except (ValueError, KeyError, OSError) as error:
        print(error, file=sys.stderr)
        return 1
    if subprocess.run([sys.executable, "scripts/check-compliance-catalog.py"], cwd=ROOT).returncode:
        return 1
    if args.check:
        print(f"proof inventory: {len(document['harnesses'])} harnesses, all mapped")
        return 0
    version = subprocess.run(["cargo", "kani", "--version"], cwd=ROOT, capture_output=True, text=True, check=True).stdout
    if f"Kani Rust Verifier {document['verifier_version']} " not in version:
        print(f"install kani-verifier {document['verifier_version']} before running proofs", file=sys.stderr)
        return 1
    report_dir = args.report_dir.resolve()
    report_dir.mkdir(parents=True, exist_ok=True)
    rows = [row for row in document["harnesses"] if args.group is None or row["group"] == args.group]
    if not rows:
        print("no harnesses selected", file=sys.stderr)
        return 1
    report = {
        "schema_version": 1, "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
        "input_sha256": input_digest(ROOT), "host": platform.machine(), "verifier": version.strip(),
        "features": document["features"], "solver_timeout_seconds": args.timeout,
        "group": args.group, "expected_harnesses": len(rows), "status": "running", "results": [],
    }
    def save() -> None:
        (report_dir / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    save()
    for row in rows:
        print(f"Verifying {row['harness']}...", flush=True)
        result = run_harness(ROOT, row, report_dir, args.timeout)
        report["results"].append(result)
        save()
        print(f"  {result['status']} ({result['seconds']}s)", flush=True)
    passed = sum(row["status"] == "passed" for row in report["results"])
    report["status"] = "passed" if passed == len(rows) else "failed"
    save()
    print(f"{passed}/{len(rows)} proofs passed; reports: {report_dir}")
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
