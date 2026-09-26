#!/usr/bin/env python3
"""Flash and stack of the embedded API on bare-metal targets.

Builds `examples/firmware` at opt-level s with fat LTO and
`-Z emit-stack-sizes`, then reports for each binary its flash (text,
rodata and data), the size of the driver state from `-Z print-type-sizes`,
the largest stack frame in Hadris code, and the worst-case stack from mount
and from `_start`. Worst-case stack follows the
direct calls `llvm-objdump` shows; indirect calls (the clock, the fold and
the code page) and functions built without stack sizes (compiler builtins)
count as 0 and are listed.

Needs a nightly toolchain with the `llvm-tools` component and the targets.

usage: scripts/firmware-size.py [--check] [target...]

With --check, fails when a budget of docs/v3/actions.md is exceeded:
NF-STACK-01 (mount under 2 KB of stack, driver state under 2 KB) and
NF-STACK-02 (no frame over 1 KB in Hadris code), or when the FAT logger's flash on thumbv7em grows past
FLASH_CEILING. NF-FLASH-01's 20 KB target is a tracked goal, not a 3.0
requirement (Q16); the ceiling only guards against growth.
"""

import os
import re
import struct
import subprocess
import sys
from pathlib import Path

TARGETS = ["thumbv6m-none-eabi", "thumbv7em-none-eabihf", "riscv32imc-unknown-none-elf"]
BINS = ["fat-log", "fat", "fat-unicode", "fat-async", "exfat"]
MOUNT = re.compile(r"hadris_example_firmware::sync::mount_(ex)?fat")
STATE = re.compile(r"type: `hadris_fat::(?:exfat::)?embedded::[^`]+::(?:Ex)?Fat<(?:'[^,]+, )?hadris_example_firmware::Card>`: (\d+) bytes")
FLASH_TARGET = "thumbv7em-none-eabihf"
FLASH_TARGET_BYTES = 20 * 1024
FLASH_CEILING = 44 * 1024
MOUNT_LIMIT = 2048
STATE_LIMIT = 2048
FRAME_LIMIT = 1024

ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = ROOT / "target" / "firmware"


def run(args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, **kwargs).stdout


def tool(name):
    sysroot = run(["rustc", "--print", "sysroot"]).strip()
    host = re.search(r"host: (\S+)", run(["rustc", "-vV"])).group(1)
    path = Path(sysroot) / "lib" / "rustlib" / host / "bin" / name
    if not path.exists():
        sys.exit(f"{path} is missing: rustup component add llvm-tools")
    return str(path)


def build(target):
    """Builds each binary and returns its driver state size from `-Z print-type-sizes`."""
    env = dict(os.environ)
    env.update(
        RUSTFLAGS="-Z emit-stack-sizes",
        CARGO_PROFILE_RELEASE_OPT_LEVEL="s",
        CARGO_PROFILE_RELEASE_LTO="fat",
        CARGO_PROFILE_RELEASE_CODEGEN_UNITS="1",
        CARGO_PROFILE_RELEASE_PANIC="abort",
        CARGO_PROFILE_RELEASE_DEBUG="false",
    )
    state = {}
    for name in BINS:
        out = subprocess.run(
            ["cargo", "rustc", "-q", "--release", "-p", "hadris-example-firmware", "--bin", name,
             "--target", target, "--target-dir", str(TARGET_DIR), "--", "-Z", "print-type-sizes"],
            check=True, env=env, cwd=ROOT, capture_output=True, text=True,
        ).stdout
        sizes = [int(m) for m in STATE.findall(out)]
        state[name] = max(sizes) if sizes else None
    return state


def sections(data):
    """(name, flags, type, size, bytes) of each section of a 32-bit ELF."""
    if data[:4] != b"\x7fELF" or data[4] != 1:
        sys.exit("expected a 32-bit ELF")
    shoff, = struct.unpack_from("<I", data, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", data, 0x2E)
    headers = [struct.unpack_from("<IIIIIIIIII", data, shoff + i * shentsize) for i in range(shnum)]
    strtab = headers[shstrndx]
    out = []
    for name, kind, flags, _, offset, size, *_ in headers:
        start = strtab[4] + name
        label = data[start:data.index(b"\0", start)].decode()
        out.append((label, flags, kind, size, data[offset:offset + size]))
    return out


def flash(secs):
    alloc, write, execute, nobits = 0x2, 0x1, 0x4, 8
    text = sum(s[3] for s in secs if s[1] & alloc and s[1] & execute)
    rodata = sum(s[3] for s in secs if s[1] & alloc and not s[1] & (write | execute) and s[2] != nobits)
    data = sum(s[3] for s in secs if s[1] & alloc and s[1] & write and s[2] != nobits)
    bss = sum(s[3] for s in secs if s[1] & alloc and s[2] == nobits)
    return text, rodata, data, bss


def frames(secs):
    """Stack frame size by function address, from `.stack_sizes`."""
    raw = next((s[4] for s in secs if s[0] == ".stack_sizes"), b"")
    out, at = {}, 0
    while at < len(raw):
        address, = struct.unpack_from("<I", raw, at)
        at += 4
        size = shift = 0
        while True:
            byte = raw[at]
            at += 1
            size |= (byte & 0x7F) << shift
            shift += 7
            if byte < 0x80:
                break
        out[address & ~1] = size
    return out


def names(path):
    """Demangled name by function address."""
    out = {}
    for line in run([tool("llvm-nm"), "-C", "--defined-only", path]).splitlines():
        parts = line.split(" ", 2)
        if len(parts) == 3 and parts[1] in "tT":
            out.setdefault(int(parts[0], 16) & ~1, parts[2])
    return out


def calls(path):
    """Direct callees and whether there are indirect calls, by address."""
    header = re.compile(r"^([0-9a-f]+) <(.+)>:$")
    insn = re.compile(r"^\s+[0-9a-f]+:\s+(\S+)\s*(.*)$")
    target = re.compile(r"<([^>+]+)(\+0x[0-9a-f]+)?>")
    addresses, edges, indirect = {}, {}, set()
    current = None
    lines = run([tool("llvm-objdump"), "-d", "--no-show-raw-insn", path]).splitlines()
    for line in lines:
        if m := header.match(line):
            current = int(m.group(1), 16) & ~1
            addresses[m.group(2)] = current
            edges.setdefault(current, set())
    for line in lines:
        if m := header.match(line):
            current = int(m.group(1), 16) & ~1
            continue
        m = insn.match(line)
        if not m or current is None:
            continue
        mnemonic, operands = m.groups()
        if mnemonic in ("blx", "jalr") and "<" not in operands:
            indirect.add(current)
        for name, _ in target.findall(operands):
            callee = addresses.get(name)
            if callee is not None and callee != current:
                edges[current].add(callee)
    return edges, indirect


def worst(edges, frame):
    memo, recursive = {}, set()

    def visit(node, path):
        if node in memo:
            return memo[node]
        if node in path:
            recursive.add(node)
            return 0
        path.add(node)
        deepest = max((visit(callee, path) for callee in edges.get(node, ())), default=0)
        path.discard(node)
        memo[node] = frame.get(node, 0) + deepest
        return memo[node]

    return visit, recursive


def api(name):
    """Whether `name` is Hadris library code rather than the example's."""
    return "hadris_fat" in name and not name.startswith("hadris_example_firmware::")


def builtin(name):
    """Whether `name` is a compiler builtin: memory and integer helpers."""
    return name.startswith(("__", "compiler_builtins::", "mem")) or name in ("memcpy", "memmove", "memset", "memcmp")


def measure(path):
    data = Path(path).read_bytes()
    secs = sections(data)
    frame = frames(secs)
    label = names(path)
    edges, indirect = calls(path)
    visit, recursive = worst(edges, frame)
    by_name = {name: address for address, name in label.items()}
    start = by_name.get("_start")
    mount = [a for a, n in label.items() if MOUNT.search(n)]
    hadris = [(size, label.get(a, hex(a))) for a, size in frame.items() if api(label.get(a, ""))]
    unknown = sorted({
        re.sub(r" \(\.llvm\.\d+\)$", "", label.get(c, hex(c)))
        for e in edges.values() for c in e
        if c not in frame and not builtin(label.get(c, ""))
    })
    return {
        "flash": flash(secs),
        "start": visit(start, set()) if start is not None else None,
        "mount": max((visit(a, set()) for a in mount), default=None),
        "frame": max(hadris, default=(0, "-")),
        "indirect": len(indirect),
        "recursive": sorted(label.get(a, hex(a)) for a in recursive if api(label.get(a, ""))),
        "unknown": unknown,
    }


def main():
    args = sys.argv[1:]
    check = "--check" in args
    targets = [a for a in args if a != "--check"] or TARGETS
    rows, failures, open_items, notes = [], [], [], set()
    for target in targets:
        state = build(target)
        for name in BINS:
            path = TARGET_DIR / target / "release" / name
            r = measure(str(path))
            text, rodata, data, bss = r["flash"]
            total = text + rodata + data
            rows.append(
                f"| {target} | {name} | {total} | {text} | {rodata} | {data + bss} | {state[name]} "
                f"| {r['mount'] if r['mount'] is not None else '-'} | {r['start']} "
                f"| {r['frame'][0]} |"
            )
            notes.update(r["unknown"])
            if r["recursive"]:
                failures.append(f"{target} {name}: recursion through {', '.join(r['recursive'])}")
            if state[name] is None or state[name] >= STATE_LIMIT:
                failures.append(f"{target} {name}: driver state {state[name]}, budget under {STATE_LIMIT} (NF-STACK-01)")
            if r["mount"] is not None and r["mount"] >= MOUNT_LIMIT:
                failures.append(f"{target} {name}: mount stack {r['mount']} >= {MOUNT_LIMIT} (NF-STACK-01)")
            if r["frame"][0] > FRAME_LIMIT:
                failures.append(f"{target} {name}: {r['frame'][1]} frame {r['frame'][0]} > {FRAME_LIMIT} (NF-STACK-02)")
            if target == FLASH_TARGET and name == "fat-log":
                if total >= FLASH_CEILING:
                    failures.append(f"{target} {name}: flash {total} >= {FLASH_CEILING} (NF-FLASH-01 ceiling)")
                elif total >= FLASH_TARGET_BYTES:
                    open_items.append(f"{target} {name}: flash {total}, NF-FLASH-01 targets {FLASH_TARGET_BYTES}")
    report = [
        "| Target | Binary | Flash | Text | Rodata | Static RAM | Driver state | Mount stack | Worst stack | Largest frame |",
        "|---|---|---|---|---|---|---|---|---|---|",
        *rows,
        "",
        "Bytes; opt-level s, fat LTO. Static RAM is data and bss; driver state is the size of "
        "`Fat` or `ExFat` with 4 file slots. Mount stack is the sync mount path; worst stack runs "
        "from `_start` and includes the driver state the session keeps on its stack (and "
        "the pinned future for fat-async). Stack follows direct calls; indirect calls and "
        "compiler builtins count as 0"
        + (", as do " + ", ".join(sorted(notes)) if notes else "") + ".",
    ]
    if open_items:
        report += ["", "Open:", *(f"- {o}" for o in open_items)]
    if failures:
        report += ["", "Budget failures:", *(f"- {f}" for f in failures)]
    text = "\n".join(report)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as out:
            out.write("## Embedded flash and stack\n\n" + text + "\n")
    if check and failures:
        sys.exit(1)


if __name__ == "__main__":
    main()
