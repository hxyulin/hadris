#!/usr/bin/env python3
"""Check Kani proof inventories or execute one with pinned Kani and reports."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import re
import signal
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[1]
INVENTORIES = Path("spec/proofs")
PROOF = re.compile(r"#\[kani::proof\]\s*(?:#\[[^\]]*\]\s*)*fn\s+(\w+)")
MATRIX = re.compile(r"^\s*group:\s*\[([^\]]*)\]\s*$", re.MULTILINE)


def suites(root: Path) -> list[str]:
    return sorted(path.stem for path in (root / INVENTORIES).glob("*.json"))


def discover(root: Path, crate: dict) -> dict[str, str]:
    base = Path(crate["path"]) / "src"
    discovered = {}
    for path in sorted((root / base).rglob("*.rs")):
        names = PROOF.findall(path.read_text())
        if not names:
            continue
        relative = path.relative_to(root / base)
        if relative != Path("verification.rs") and relative.parts[0] != "verification":
            raise ValueError(f"proof harness outside verification modules: {base / relative}")
        parts = list(relative.with_suffix("").parts)
        if parts[-1] == "mod":
            parts.pop()
        prefix = "::".join(parts)
        for name in names:
            harness = f"{prefix}::{name}"
            if harness in discovered:
                raise ValueError(f"duplicate proof harness: {harness}")
            discovered[harness] = str(path.relative_to(root))
    return discovered


def inventory(root: Path, suite: str) -> dict:
    document = json.loads((root / INVENTORIES / f"{suite}.json").read_text())
    if document["schema_version"] != 2:
        raise ValueError(f"{suite}: unsupported proof manifest schema")
    groups = document["groups"]
    crates = document["crates"]
    if not groups or len(groups) != len(set(groups)) or not crates:
        raise ValueError(f"{suite}: groups and crates must be non-empty and unique")
    for name, crate in crates.items():
        if not isinstance(crate["features"], list) or not (root / crate["path"] / "Cargo.toml").is_file():
            raise ValueError(f"{suite}: invalid crate configuration: {name}")
    workflow = (root / document["workflow"]).read_text()
    matrices = [[group.strip() for group in found.split(",")] for found in MATRIX.findall(workflow)]
    if matrices != [groups]:
        raise ValueError(f"{suite}: workflow group matrix must list exactly {groups}")
    discovered = {}
    for name, crate in crates.items():
        discovered.update({(name, harness): path for harness, path in discover(root, crate).items()})
    rows = document["harnesses"]
    keys = [(row["crate"], row["harness"]) for row in rows]
    if not keys or len(keys) != len(set(keys)) or set(keys) != set(discovered):
        raise ValueError(f"{suite}: proof manifest must cover every harness exactly once")
    requirements = json.loads((root / document["catalog"]).read_text())["requirements"]
    requirement_ids = {row["id"] for row in requirements}
    for row in rows:
        key = (row["crate"], row["harness"])
        if row["path"] != discovered[key] or row["group"] not in groups:
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
    unused = set(groups) - {row["group"] for row in rows}
    if unused:
        raise ValueError(f"{suite}: groups without harnesses: {sorted(unused)}")
    return document


def input_digest(root: Path, suite: str, document: dict) -> str:
    paths = set((root / "crates").rglob("*.rs")) | set((root / "crates").rglob("Cargo.toml"))
    paths.update(root / path for path in (
        "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", str(INVENTORIES / f"{suite}.json"),
        "scripts/verify-proofs.py", "scripts/check-compliance-catalog.py", document["catalog"],
    ))
    digest = hashlib.sha256()
    for path in sorted(paths):
        digest.update(str(path.relative_to(root)).encode() + b"\0" + path.read_bytes() + b"\0")
    return digest.hexdigest()


def run_group(command: list[str], cwd: Path, output, timeout: float) -> int | None:
    """Run `command` in its own process group, killing the whole group on timeout."""
    process = subprocess.Popen(command, cwd=cwd, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        return process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
        return None


def command_for(row: dict, features: list[str], timeout: int) -> list[str]:
    command = ["cargo", "kani", "-p", row["crate"], "--lib", "--no-default-features"]
    if features:
        command += ["--features", ",".join(features)]
    return command + [
        "--exact", "--harness", row["harness"], "--output-format", "terse",
        "-Z", "unstable-options", "--harness-timeout", f"{timeout}s",
    ]


def run_harness(root: Path, row: dict, features: list[str], report_dir: Path, timeout: int) -> dict:
    log = report_dir / f"{row['crate']}-{row['harness'].replace('::', '-')}.log"
    command = command_for(row, features, timeout)
    started = time.monotonic()
    status = "failed"
    with log.open("w") as output:
        returncode = run_group(command, root, output, timeout + 120)
        if returncode is None:
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
    return {**row, "features": features, "status": status, "returncode": returncode,
            "seconds": round(time.monotonic() - started, 3), "log": log.name, "command": command,
            "satisfied_covers": sum(passed for passed, _ in covers)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", nargs="?", choices=suites(ROOT),
                        help="inventory under spec/proofs/; --check without it checks every inventory")
    parser.add_argument("--check", action="store_true", help="validate inventory without running Kani")
    parser.add_argument("--group")
    parser.add_argument("--report-dir", type=Path)
    parser.add_argument("--timeout", type=int, default=300, help="solver seconds per harness")
    args = parser.parse_args(argv)
    if args.timeout < 1:
        parser.error("--timeout must be positive")
    if args.suite is None and not args.check:
        parser.error("a suite is required unless --check is given")
    selected = [args.suite] if args.suite else suites(ROOT)
    documents = {}
    try:
        for suite in selected:
            documents[suite] = inventory(ROOT, suite)
    except (ValueError, KeyError, TypeError, OSError) as error:
        print(error, file=sys.stderr)
        return 1
    if args.group is not None and (args.suite is None or args.group not in documents[args.suite]["groups"]):
        parser.error(f"--group must be one of {documents[args.suite]['groups'] if args.suite else 'a suite group'}")
    if subprocess.run([sys.executable, "scripts/check-compliance-catalog.py"], cwd=ROOT).returncode:
        return 1
    if args.check:
        for suite, document in documents.items():
            print(f"{suite} proof inventory: {len(document['harnesses'])} harnesses, all mapped")
        return 0
    suite, document = args.suite, documents[args.suite]
    version = subprocess.run(["cargo", "kani", "--version"], cwd=ROOT, capture_output=True, text=True, check=True).stdout
    if f"Kani Rust Verifier {document['verifier_version']} " not in version:
        print(f"install kani-verifier {document['verifier_version']} before running proofs", file=sys.stderr)
        return 1
    report_dir = (args.report_dir or Path(f"output/{suite}-proofs")).resolve()
    report_dir.mkdir(parents=True, exist_ok=True)
    rows = [row for row in document["harnesses"] if args.group is None or row["group"] == args.group]
    report = {
        "schema_version": 2, "suite": suite,
        "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
        "input_sha256": input_digest(ROOT, suite, document), "host": platform.machine(),
        "verifier": version.strip(), "crates": document["crates"], "solver_timeout_seconds": args.timeout,
        "group": args.group, "expected_harnesses": len(rows), "status": "running", "results": [],
    }
    def save() -> None:
        (report_dir / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    save()
    for row in rows:
        print(f"Verifying {row['crate']} {row['harness']}...", flush=True)
        features = document["crates"][row["crate"]]["features"]
        result = run_harness(ROOT, row, features, report_dir, args.timeout)
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
