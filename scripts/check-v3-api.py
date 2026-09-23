#!/usr/bin/env python3
"""V3 public API guardrails built on cargo-public-api (R3 and sync/async parity).

  subset  For each library crate, every item in the minimal-feature public API
          must appear verbatim in the --all-features public API, and the
          larger build must not add fields or variants to a struct or enum
          that the minimal build already has. Features may add items but
          never change the shape of an existing one (R3).
  parity  For crates exposing both `sync` and `async` modules, report public
          items that exist in only one mode, from the --all-features API.

Needs cargo-public-api 0.52.0. Unless RUSTUP_TOOLCHAIN is already set, it runs
under the nightly pinned by the public-api CI job (see scripts/check-public-api.sh).

Limits: the comparison is textual on `cargo public-api -sss` output, so auto
trait and blanket impls are not compared. Parity pairs items by their path
after deleting the mode module segment (`sync`, `async`, `r#async`; hadris-io's
`sync_api`/`async_api` become one name) and dropping `async` keywords and
`impl Future<Output = T>` wrappers. rustdoc prints the members of sync items
under the crate root when the crate does `pub use sync::*`, so a root path
whose first segment is also a direct child of `crate::sync` counts as sync;
an unrelated root item that shares such a name is misattributed. Items that differ only in their signature are listed separately and do not
count as findings.

Exits 1 when anything is reported.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PINNED_NIGHTLY = "nightly-2026-09-04"

# Smallest feature set per crate that still builds: `sync` alone where the
# crate has it, otherwise the smallest useful tier in .github/workflows/rust.yml.
MINIMAL_FEATURES = {
    "hadris": "sync",
    "hadris-block": "sync",
    "hadris-cd": "std,sync",
    "hadris-common": "sync",
    "hadris-cpio": "sync",
    "hadris-fat": "sync",
    "hadris-fs": "",
    "hadris-io": "sync",
    "hadris-iso": "sync",
    "hadris-macros": "",
    "hadris-ntfs": "sync",
    "hadris-optical": "sync",
    "hadris-part": "sync",
    "hadris-storage": "sync",
    "hadris-udf": "sync",
}

MODE_MODULE = re.compile(r"\b(hadris\w*(?:::\w+)*?)::(sync|r#async|async)(?![\w#])")
MODE_API = re.compile(r"\b(hadris\w*(?:::\w+)*?)::(sync|async)_api\b")
ASYNC_KW = re.compile(r"\basync\s+")
FUTURE = re.compile(r"impl core::future::future::Future<Output = ")
ITEM = re.compile(
    r'^pub (?P<quals>(?:(?:const|unsafe|async|extern "[^"]*"|fn|struct|enum|trait|mod|type|static|use|union|macro|auto) )*)'
    r"(?=[\w#])"
)


def item_path(line: str, start: int) -> str:
    """Path starting at `start`, with generic arguments removed."""
    out, depth, k = [], 0, start
    while k < len(line):
        ch = line[k]
        if ch == "<":
            depth += 1
        elif ch == ">" and depth:
            depth -= 1
        elif depth == 0:
            if ch.isalnum() or ch in "_#":
                out.append(ch)
            elif line.startswith("::", k):
                out.append("::")
                k += 1
            else:
                break
        k += 1
    return "".join(out)


def public_api(crate: str, features: str | None) -> list[str]:
    cmd = ["cargo", "public-api", "-p", crate]
    if features is None:
        cmd.append("--all-features")
    else:
        cmd.append("--no-default-features")
        if features:
            cmd += ["--features", features]
    cmd += ["-sss", "--color", "never"]
    env = dict(os.environ)
    env.setdefault("RUSTUP_TOOLCHAIN", PINNED_NIGHTLY)
    proc = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr[-4000:])
        raise RuntimeError(f"cargo public-api failed for {crate} ({' '.join(cmd[3:-4])})")
    return [line for line in proc.stdout.splitlines() if line.strip()]


def item_key(line: str) -> str:
    if line.startswith("impl"):
        return line
    m = ITEM.match(line)
    if not m:
        return line
    quals = " ".join(q for q in m.group("quals").split() if q != "async")
    return f"{quals} {item_path(line, m.end())}".strip()


def unwrap_futures(line: str) -> str:
    while m := FUTURE.search(line):
        depth, k = 1, m.end()
        while k < len(line) and depth:
            depth += {"<": 1, ">": -1}.get(line[k], 0)
            k += 1
        line = line[: m.start()] + line[m.end() : k - 1] + line[k:]
    return line


def normalize(line: str) -> str:
    line = MODE_MODULE.sub(r"\1", line)
    line = MODE_API.sub(r"\1::mode_api", line)
    line = line.replace("embedded_io_async::", "embedded_io::")
    line = unwrap_futures(line)
    return ASYNC_KW.sub("", line)


def line_scope(line: str) -> str:
    if line.startswith("impl"):
        if " for " in line:
            return line.rsplit(" for ", 1)[-1]
        k, depth = len("impl"), 0
        while k < len(line):
            depth += {"<": 1, ">": -1}.get(line[k], 0)
            k += 1
            if depth == 0 and line[k - 1] in "> ":
                break
        return line[k:].lstrip()
    m = ITEM.match(line)
    return item_path(line, m.end()) if m else line


def line_mode(line: str, crate_ident: str, sync_names: set[str]) -> str | None:
    scope = line_scope(line)
    found = MODE_MODULE.search(scope)
    if found:
        return "sync" if found.group(2) == "sync" else "async"
    m = re.match(rf"{crate_ident}::(\w+)", scope)
    if m and m.group(1) in sync_names:
        return "sync"
    return None


def check_subset(crates: list[str]) -> dict[str, int]:
    counts = {}
    for crate in crates:
        features = MINIMAL_FEATURES[crate]
        small = public_api(crate, features)
        large = public_api(crate, None)
        large_set = set(large)
        by_key: dict[str, list[str]] = {}
        for line in large:
            by_key.setdefault(item_key(line), []).append(line)
        small_set = set(small)
        missing = [line for line in small if line not in large_set]
        small_keys = set(map(item_key, small))
        small_types = {key.split(" ", 1)[1] for key in small_keys if key.startswith(("struct ", "enum ", "union "))}
        grown = [
            line
            for line in large
            if line not in small_set
            and ITEM.match(line)
            and " " not in (key := item_key(line))
            and key not in small_keys
            and key.rsplit("::", 1)[0] in small_types
        ]
        counts[crate] = len(missing) + len(grown)
        label = features or "none"
        print(
            f"== {crate}: minimal [{label}] {len(small)} items, all-features {len(large)} items, "
            f"{len(missing)} changed or removed, {len(grown)} members added to existing types"
        )
        for line in missing:
            print(f"  - {line}")
            for other in by_key.get(item_key(line), []):
                print(f"    + {other}")
        for line in grown:
            print(f"  + {line}")
    return counts


def check_parity(crates: list[str]) -> dict[str, int]:
    counts = {}
    for crate in crates:
        api = public_api(crate, None)
        ident = crate.replace("-", "_")
        sync_names = {
            m.group(1)
            for line in api
            if not line.startswith("pub use ") and (m := re.match(rf"{ident}::sync::(\w+)", line_scope(line)))
        }
        modes: dict[str, dict[str, set[str]]] = {"sync": {}, "async": {}}
        for line in api:
            mode = line_mode(line, ident, sync_names)
            if mode:
                norm = normalize(line)
                modes[mode].setdefault(item_key(norm), set()).add(norm)
        sync, asyn = modes["sync"], modes["async"]
        if not sync or not asyn:
            present = "sync" if sync else "async" if asyn else "neither"
            print(f"== {crate}: skipped, public API has {present} mode only")
            continue
        only_sync = sorted(sync.keys() - asyn.keys())
        only_async = sorted(asyn.keys() - sync.keys())
        differ = sorted(k for k in sync.keys() & asyn.keys() if sync[k] != asyn[k])
        counts[crate] = len(only_sync) + len(only_async)
        print(
            f"== {crate}: {len(only_sync)} sync-only, {len(only_async)} async-only, "
            f"{len(differ)} with differing signatures"
        )
        for key in only_sync:
            print(f"  sync only:  {key}")
        for key in only_async:
            print(f"  async only: {key}")
        for key in differ:
            print(f"  signature:  {key}")
            for line in sorted(sync[key] - asyn[key]):
                print(f"    sync:  {line}")
            for line in sorted(asyn[key] - sync[key]):
                print(f"    async: {line}")
    return counts


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("check", choices=("subset", "parity"))
    parser.add_argument("-p", "--package", action="append", choices=sorted(MINIMAL_FEATURES), help="limit to these crates")
    args = parser.parse_args()

    crates = args.package or sorted(MINIMAL_FEATURES)
    counts = check_subset(crates) if args.check == "subset" else check_parity(crates)

    print(f"\n{args.check} findings per crate:")
    for crate, count in sorted(counts.items()):
        print(f"  {crate}: {count}")
    total = sum(counts.values())
    print(f"  total: {total}")
    return 1 if total else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except RuntimeError as err:
        print(f"error: {err}", file=sys.stderr)
        sys.exit(2)
