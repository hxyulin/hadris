#!/usr/bin/env python3
"""Differential sweep: run hadris' fs_dump over corpus inputs and compare the
canonical listing against reference tools (ntfsls, cpio, mtools, bsdtar).

Usage:
    differential_sweep.py [--target T]... [--corpus-root fuzz/corpus]
                          [--out fuzz/artifacts/differential]

Only mountable inputs are compared: if fs_dump prints nothing, the input is
counted as skipped-unmountable. A reference tool that crashes (signal) or
times out on an input fs_dump parsed fine is reported as 'ref-error'.

Mismatches are written to <out>/<target>/<hash>.{input,hadris.txt,ref.txt},
deduplicated by the sha1 of the (hadris, ref) output pair.
"""

import argparse
import hashlib
import shutil
import subprocess
import sys
from pathlib import Path

TIMEOUT_S = 10
MAX_FILES_PER_TARGET = 500
MAX_INPUT_SIZE = 8 * 1024 * 1024

TARGET_TO_FMT = {
    "fat_read": "fat",
    "exfat_read": "exfat",
    "ntfs_read": "ntfs",
    "iso_read": "iso",
    "cpio_read": "cpio",
    # udf_read / part_read have no reference adapter; skipped silently.
}


def norm_path(path, fmt):
    p = path.strip()
    if p.startswith("./"):
        p = p[2:]
    p = p.lstrip("/")
    if fmt in ("fat", "exfat", "iso"):
        p = p.lower()
    if fmt == "iso" and ";" in p:
        base, _, ver = p.rpartition(";")
        if ver.isdigit():
            p = base
    return p


def parse_hadris(text, fmt):
    """fs_dump output -> set of (kind, path, size)."""
    entries = set()
    for line in text.splitlines():
        if line.startswith("dir "):
            entries.add(("dir", norm_path(line[4:], fmt), None))
        elif line.startswith("file "):
            parts = line.split(" ", 3)
            if len(parts) == 4:
                entries.add(("file", norm_path(parts[3], fmt), int(parts[1])))
    return entries


def run(cmd, stdin_path=None):
    """Returns (rc, stdout_text). rc is -1 on timeout."""
    stdin = open(stdin_path, "rb") if stdin_path else subprocess.DEVNULL
    try:
        proc = subprocess.run(
            cmd,
            stdin=stdin,
            capture_output=True,
            timeout=TIMEOUT_S,
        )
        return proc.returncode, proc.stdout.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return -1, ""
    except OSError:
        return -2, ""
    finally:
        if stdin_path and not stdin.closed:
            stdin.close()


class RefResult:
    def __init__(self, status, entries=None, raw=""):
        # status: "ok" | "crash" (signal/timeout) | "reject" (clean nonzero exit)
        self.status = status
        self.entries = entries or set()
        self.raw = raw


def ref_ntfsls(path, fmt):
    rc, out = run(["ntfsls", "-R", str(path)])
    if rc in (-1, -2) or rc < 0:
        return RefResult("crash", raw=out)
    if rc != 0:
        return RefResult("reject", raw=out)
    entries = set()
    for line in out.splitlines():
        p = norm_path(line, fmt)
        if p:
            entries.add(("entry", p, None))
    return RefResult("ok", entries, out)


def ref_cpio(path, fmt):
    rc, out = run(["cpio", "-itv", "--quiet"], stdin_path=path)
    if rc in (-1, -2) or rc < 0:
        return RefResult("crash", raw=out)
    if rc != 0:
        return RefResult("reject", raw=out)
    entries = set()
    for line in out.splitlines():
        fields = line.split()
        if len(fields) < 9:
            continue
        kind = "dir" if fields[0].startswith("d") else "file"
        size = int(fields[4]) if fields[4].isdigit() else None
        name = norm_path(" ".join(fields[8:]), fmt)
        entries.add((kind, name, size if kind == "file" else None))
    return RefResult("ok", entries, out)


def ref_mdir(path, fmt):
    # Root directory only (no recursion): compare depth-1 entries, paths only.
    # -a includes hidden entries, which hadris also lists (spec-valid).
    rc, out = run(["mdir", "-b", "-a", "-i", str(path), "::"])
    if rc in (-1, -2) or rc < 0:
        return RefResult("crash", raw=out)
    if rc != 0:
        return RefResult("reject", raw=out)
    entries = set()
    for line in out.splitlines():
        name = line.strip()
        if not name:
            continue
        kind = "dir" if name.endswith("/") else "file"
        name = name.rstrip("/")
        if name.startswith("::"):
            name = name[2:]
        entries.add((kind, norm_path(name, fmt), None))
    return RefResult("ok", entries, out)


def ref_bsdtar(path, fmt):
    rc, out = run(["bsdtar", "-tf", str(path)])
    if rc in (-1, -2) or rc < 0:
        return RefResult("crash", raw=out)
    if rc != 0:
        return RefResult("reject", raw=out)
    if not out.strip():
        # libarchive exits 0 with no listing when its format bid is too low
        # to recognize the input — that means "not an archive", not "empty
        # filesystem", so the input is not comparable.
        return RefResult("reject", raw=out)
    entries = set()
    for line in out.splitlines():
        name = line.strip()
        if not name:
            continue
        kind = "dir" if name.endswith("/") else "file"
        p = norm_path(name.rstrip("/"), fmt)
        if p:
            entries.add((kind, p, None))
    return RefResult("ok", entries, out)


ADAPTERS = {
    "ntfs_read": ("ntfsls", ref_ntfsls, "path"),
    "cpio_read": ("cpio", ref_cpio, "path+kind+size"),
    "fat_read": ("mdir", ref_mdir, "root-path+kind"),
    "exfat_read": ("mdir", ref_mdir, "root-path+kind"),
    "iso_read": ("bsdtar", ref_bsdtar, "path+kind"),
}


def project(entries, fmt, mode):
    """Reduce (kind, path, size) tuples to the comparison mode of the adapter."""
    out = set()
    for kind, path, size in entries:
        if mode == "path":
            out.add(path)
        elif mode == "path+kind":
            out.add((kind, path))
        elif mode == "root-path+kind":
            if path and "/" not in path:
                out.add((kind, path))
        else:
            out.add((kind, path, size))
    return out


def build_fs_dump(fuzz_dir):
    proc = subprocess.run(
        ["cargo", "+nightly", "build", "--bin", "fs_dump"],
        cwd=fuzz_dir,
        capture_output=True,
    )
    if proc.returncode != 0:
        sys.exit(
            "differential_sweep: failed to build fs_dump:\n"
            + proc.stderr.decode("utf-8", "replace")
        )
    binary = fuzz_dir / "target" / "debug" / "fs_dump"
    if not binary.exists():
        sys.exit(f"differential_sweep: fs_dump binary not found at {binary}")
    return binary


def main():
    repo_root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", action="append", default=None,
                        help="restrict to this target (repeatable)")
    parser.add_argument("--corpus-root", default=str(repo_root / "fuzz" / "corpus"))
    parser.add_argument("--out", default=str(repo_root / "fuzz" / "artifacts" / "differential"))
    args = parser.parse_args()

    corpus_root = Path(args.corpus_root)
    out_root = Path(args.out)
    fs_dump = build_fs_dump(repo_root / "fuzz")

    targets = args.target if args.target else sorted(TARGET_TO_FMT)
    for target in targets:
        if target not in TARGET_TO_FMT:
            # udf_read / part_read: no reference adapter, skip silently.
            if target not in ("udf_read", "part_read", "fat_ops"):
                print(f"{target}: unknown target, skipped", file=sys.stderr)
            continue
        fmt = TARGET_TO_FMT[target]
        tool, adapter, mode = ADAPTERS[target]
        if not shutil.which(tool):
            print(f"{target}: reference tool '{tool}' not found, skipping target")
            continue
        target_dir = corpus_root / target
        if not target_dir.is_dir():
            print(f"{target}: no corpus dir at {target_dir}, skipping")
            continue

        files = [f for f in target_dir.iterdir() if f.is_file()]
        files.sort(key=lambda f: f.stat().st_mtime, reverse=True)
        files = [f for f in files if f.stat().st_size <= MAX_INPUT_SIZE]
        files = files[:MAX_FILES_PER_TARGET]

        compared = mismatches = ref_errors = unmountable = rejected = subset = 0
        diverged = 0
        out_dir = out_root / target
        for f in files:
            rc, hadris_text = run([str(fs_dump), fmt, str(f)])
            if rc != 0 or not hadris_text.strip():
                unmountable += 1
                continue
            hadris = project(parse_hadris(hadris_text, fmt), fmt, mode)
            ref = adapter(f, fmt)
            if ref.status == "crash":
                ref_errors += 1
            elif ref.status == "reject":
                rejected += 1
                continue
            else:
                ref_set = project(ref.entries, fmt, mode)
                if ref_set == hadris:
                    compared += 1
                    continue
                elif hadris < ref_set:
                    # hadris returned strictly less than the reference: expected on
                    # corrupt inputs, where hadris rejects/stops at the first bad
                    # entry while reference tools tolerate or resync. Informational
                    # only — a wrong entry or wrong size would break the subset
                    # relation and count as a mismatch.
                    compared += 1
                    subset += 1
                    continue
                elif mode == "root-path+kind":
                    # FAT/exFAT: inclusion differences on corrupt attribute
                    # bytes are tolerance differences in both directions
                    # (reserved-bit heuristics, strict name validation), so
                    # only a kind conflict on a shared path is a real signal.
                    ref_paths = {p for _, p in ref_set}
                    hadris_paths = {p for _, p in hadris}
                    conflict = any(
                        (("dir", p) in hadris) != (("dir", p) in ref_set)
                        for p in ref_paths & hadris_paths
                    )
                    compared += 1
                    if not conflict:
                        diverged += 1
                        continue
                    mismatches += 1
                else:
                    compared += 1
                    mismatches += 1
            # Report (mismatch or ref-error): dedupe by output pair.
            key = hashlib.sha1(
                hadris_text.encode() + b"\0" + ref.raw.encode()
            ).hexdigest()[:16]
            out_dir.mkdir(parents=True, exist_ok=True)
            stem = out_dir / key
            if not (stem.with_suffix(".input")).exists():
                shutil.copyfile(f, stem.with_suffix(".input"))
                stem.with_suffix(".hadris.txt").write_text(hadris_text)
                label = "ref-error" if ref.status == "crash" else "mismatch"
                stem.with_suffix(".ref.txt").write_text(
                    f"# {label}: {tool} on {f.name}\n" + ref.raw
                )
        print(
            f"{target}: compared {compared}, mismatches {mismatches}, "
            f"hadris-subset {subset}, diverged {diverged}, "
            f"ref-errors {ref_errors}, ref-rejected {rejected}, "
            f"skipped-unmountable {unmountable}"
        )


if __name__ == "__main__":
    main()
