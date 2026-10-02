#!/usr/bin/env python3

import argparse
import json
import subprocess
from pathlib import Path


def main():
    root = Path(__file__).resolve().parent.parent
    groups = json.loads((root / "scripts/ci-features.json").read_text())
    parser = argparse.ArgumentParser(description="Check the CI feature tiers")
    parser.add_argument("group", choices=["all", *groups])
    args = parser.parse_args()
    selected = groups.values() if args.group == "all" else [groups[args.group]]
    failures = []
    for tiers in selected:
        for tier in tiers:
            label = f"{tier['crate']} ({tier['tier']})"
            print(f"::group::{label}", flush=True)
            result = subprocess.run(
                [
                    "cargo",
                    "check",
                    "--locked",
                    "-p",
                    tier["crate"],
                    "--no-default-features",
                    "--features",
                    tier["features"],
                ],
                cwd=root,
            )
            print("::endgroup::", flush=True)
            if result.returncode:
                failures.append(label)
    if failures:
        parser.exit(1, "Failed feature tiers: " + ", ".join(failures) + "\n")


if __name__ == "__main__":
    main()
