#!/usr/bin/env python3
"""V3 public API guardrails built on cargo-public-api (R3 and sync/async parity).

  subset  For each library crate, every item in the minimal-feature public API
          must appear verbatim in the --all-features public API, and the
          larger build must not add fields or variants to a struct or enum
          that the minimal build already has. Features may add items but
          never change the shape of an existing one (R3).
  parity  For crates exposing both `sync` and `async` modules, report public
          items that exist in only one mode, from the --all-features API, and
          the same between `async` and `local`, when the crate has both.
          Differences listed in PARITY_ALLOWED are printed with their reason
          and not counted.

Needs cargo-public-api 0.52.0. Unless RUSTUP_TOOLCHAIN is already set, it runs
under the nightly pinned by the public-api CI job (see scripts/check-public-api.sh).

Limits: the comparison is textual on `cargo public-api -sss` output, so auto
trait and blanket impls are not compared. Parity pairs items by their path
after deleting the mode module segment (`sync`, `async`, `r#async`,
`local`; a segment counts only when a path segment follows it, so a
method named `sync` is not a mode; hadris-io's `sync_api`/`async_api` become
one name) and dropping `async` keywords, `impl Future<Output = T>` wrappers
and the `Send` bounds `r#async` adds. rustdoc prints the members of sync items
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
    "hadris-cd": "sync",
    "hadris-common": "",
    "hadris-cpio": "sync",
    "hadris-fat": "sync",
    "hadris-fat-raw": "sync",
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

# Differences between modes that are intended. Each pattern is searched in
# "<mode> only: <item>"; the reason is printed next to the item.
PARITY_ALLOWED: dict[str, list[tuple[str, str]]] = {
    "hadris-fs": [
        (r"^sync only: fn hadris_fs::(extract_to_host|import_from_host)$", "host helpers are sync and std only"),
        (r"^sync only: .*(Iterator for hadris_fs::ReadDir|hadris_fs::ReadDir::(next|Item)$)", "async ReadDir has next_entry; no Iterator exists for it"),
        (r"^sync only: impl<F: hadris_fs::FileSystem> (alloc|core)::io::", "std::io impls on File are blocking"),
    ],
    "hadris-io": [
        (r"^local only: .*\bFromEmbedded\b", "embedded-io-async futures are not Send"),
        (r"^(sync|local) only: .*\b(alloc::rc::Rc|Rc<)", "Rc is not Send, so async has no Rc impls"),
        (r"hadris_io::legacy::", "legacy V2 traits, deleted by the last format port"),
    ],
}

MODE_MODULE = re.compile(r"\b(hadris\w*(?:::\w+)*?)::(sync|r#async|async|local)(?=::)")
MODE_API = re.compile(r"\b(hadris\w*(?:::\w+)*?)::(sync|async)_api\b")
ASYNC_KW = re.compile(r"\basync\s+")
FUTURE = re.compile(r"impl core::future::future::Future<Output = ")
SEND_BOUND = re.compile(r" \+ core::marker::(?:Send|Sync)\b|(?<=>)core::marker::Send\b")
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
    line = SEND_BOUND.sub("", line)
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
        return {"sync": "sync", "local": "local"}.get(found.group(2), "async")
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


def compare_modes(name_a: str, a: dict[str, set[str]], name_b: str, b: dict[str, set[str]], allowed) -> int:
    """Print the items in only one of two modes; return the number not allowed."""
    findings = 0
    width = max(len(name_a), len(name_b)) + len(" only:")
    for name, keys in ((name_a, sorted(a.keys() - b.keys())), (name_b, sorted(b.keys() - a.keys()))):
        for key in keys:
            reason = allowed(name, key)
            label = f"{name} only:".ljust(width)
            if reason:
                print(f"  {label} {key}  [allowed: {reason}]")
            else:
                findings += 1
                print(f"  {label} {key}")
    for key in sorted(k for k in a.keys() & b.keys() if a[k] != b[k]):
        print(f"  signature:  {key}")
        for line in sorted(a[key] - b[key]):
            print(f"    {name_a}:  {line}")
        for line in sorted(b[key] - a[key]):
            print(f"    {name_b}: {line}")
    return findings


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
        modes: dict[str, dict[str, set[str]]] = {"sync": {}, "async": {}, "local": {}}
        for line in api:
            mode = line_mode(line, ident, sync_names)
            if mode:
                norm = normalize(line)
                modes[mode].setdefault(item_key(norm), set()).add(norm)
        sync, asyn, local = modes["sync"], modes["async"], modes["local"]
        if not sync or not asyn:
            present = "sync" if sync else "async" if asyn else "neither"
            print(f"== {crate}: skipped, public API has {present} mode only")
            continue
        rules = [(re.compile(pattern), reason) for pattern, reason in PARITY_ALLOWED.get(crate, [])]

        def allowed(mode: str, key: str) -> str | None:
            text = f"{mode} only: {key}"
            return next((reason for pattern, reason in rules if pattern.search(text)), None)

        print(f"== {crate}: sync vs async")
        findings = compare_modes("sync", sync, "async", asyn, allowed)
        if local:
            print(f"== {crate}: async vs local")
            findings += compare_modes("async", asyn, "local", local, allowed)
        counts[crate] = findings
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
