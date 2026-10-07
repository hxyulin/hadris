#!/usr/bin/env python3
"""Check the competing named-GAT contracts on the selected Rust toolchain."""

import argparse
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", default="1.88.0")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent / "probes"
    cases = [
        ("gat_hrtb", False, "does not live long enough"),
        ("gat_lifetime", False, "not general enough"),
        ("gat_direct", True, ""),
    ]
    with tempfile.TemporaryDirectory(prefix="hadris-gat-probe-") as output:
        for name, passes, diagnostic in cases:
            result = subprocess.run(
                ["rustc", f"+{args.toolchain}", "--edition=2024", str(root / f"{name}.rs"), "-o", str(Path(output) / name)],
                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            )
            if (result.returncode == 0) != passes or diagnostic not in result.stdout:
                raise SystemExit(f"Unexpected {name} result:\n{result.stdout}")
            print(f"{name}: {'compiled' if passes else 'expected rejection confirmed'}")


if __name__ == "__main__":
    main()
