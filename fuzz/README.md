# Fuzzing

Coverage-guided fuzz harnesses for the untrusted-input parsers, one per reader,
plus one stateful writer harness. These are **local / developer tools** — they
are intentionally **not** run in CI (nightly + long-running; corpus replay
belongs in a developer workflow or a separate scheduled job outside the PR
gate).

| Target      | Entry point                          | Exercises |
|-------------|--------------------------------------|-----------|
| `cpio_read` | `CpioReader::next_entry_alloc` + data | newc header / `namesize` / `filesize` parsing |
| `fat_read`  | `FatFs::open` + recursive read       | BPB, FAT chain, directory + LFN parsing, file reads |
| `exfat_read`| `ExFatVolume::open` + recursive read | boot region, entry sets, FAT/no-FAT chains, upcase |
| `ntfs_read` | `NtfsFs::open` + recursive read      | boot sector, MFT records, attributes, index walks |
| `part_read` | `PartitionTable::read_from`          | MBR / GPT detection and entry parsing |
| `iso_read`  | `IsoImage::open` + recursive read    | volume descriptors, directory records, RRIP, multi-extent reads |
| `udf_read`  | `UdfVolume::open` + recursive read   | anchor/VDS/FSD, File Entry, allocation descriptors, FIDs |
| `fat_ops`   | format + fuzz-driven create/write/delete/rename ops | FAT write path vs a shadow model, verified after remount |

**The invariant:** feeding *arbitrary bytes* into a reader must only ever return
an `Err` or succeed — never panic, abort, or OOM. A crash found here is a bug in
the reader, not the harness. The read harnesses also carry self-consistency
oracles (failures tagged `ORACLE:`): re-resolved names must match, repeated
reads must agree, and re-iterated listings must be stable. `fat_ops` asserts
that the on-disk tree after a remount matches a shadow model of every
successful operation.

## Running

```bash
cargo install cargo-fuzz            # one-time
rustup toolchain install nightly    # cargo-fuzz needs nightly

cargo +nightly fuzz run cpio_read                      # fuzz until a crash / Ctrl-C
cargo +nightly fuzz run cpio_read -- -max_total_time=60 # time-boxed
cargo +nightly fuzz run cpio_read -- -runs=0           # replay corpus only, then exit
```

Replay every corpus after pulling or before a release:

```bash
for t in cpio_read fat_read exfat_read ntfs_read part_read iso_read udf_read fat_ops; do
  cargo +nightly fuzz run "$t" -- -runs=0
done
```

## Seeds

`corpus/` is git-ignored (fuzzer-grown corpora are large binary churn that live
on the fuzzing machine). Regenerate the seed images with:

```bash
fuzz/scripts/gen-seeds.sh
```

It builds real images with the tools available on an Ubuntu fuzz machine
(`mkfs.vfat`, `mkntfs`, `cpio`), falls back to python3-crafted images
(minimal FAT12 with root entries, MBR/GPT disks with valid CRCs, minimal
ISO9660), and copies repo fixtures (`test-images/`, crate test fixtures) for
exFAT/ISO/UDF where no host tool exists. The script is idempotent: seeds use
fixed names, and fuzzer-grown corpus entries are never touched.

## Fuzzing fleet

```bash
fuzz/scripts/fuzz-fleet.sh [session-name]   # default session: fuzz
```

Launches one tmux window per target, each running
`cargo +nightly fuzz run <t> -- -fork=4 -ignore_crashes=1 -rss_limit_mb=2048
-max_len=<per-target> -len_control=0 -use_value_profile=1`. Re-running kills
and replaces the session. Attach with `tmux attach -t fuzz`; crashes land in
`fuzz/artifacts/<target>/`.

## Differential testing

`src/bin/fs_dump.rs` is a normal binary (auto-discovered, not a fuzz target)
that prints a canonical, sorted listing of an image —
`file <size> <fnv1a64-of-first-4KiB> <path>`, `dir <path>`, or
`<index> <start_lba> <size_sectors>` for partition tables — and prints nothing
(exit 0) when the image does not mount:

```bash
cargo +nightly build --bin fs_dump
./target/debug/fs_dump <fat|exfat|ntfs|iso|udf|cpio|part> <image>
```

`scripts/differential_sweep.py` sweeps corpus inputs (newest 500 per target,
≤ 8 MiB), dumps each with `fs_dump`, and compares the listing against a
reference tool: `ntfsls -R` for NTFS, `cpio -itv` for cpio, `mdir -b -i` for
FAT/exFAT (root only), `bsdtar -tf` for ISO. Missing reference tools are
skipped with one notice; UDF/partition have no adapter and are skipped
silently. Mismatches — and reference-tool crashes on inputs we parsed fine —
are written to `artifacts/differential/<target>/`, deduplicated by the sha1 of
the (hadris, reference) output pair.

```bash
python3 fuzz/scripts/differential_sweep.py                 # all targets
python3 fuzz/scripts/differential_sweep.py --target cpio_read
```

## When a crash is found

cargo-fuzz writes the crashing input to `artifacts/<target>/crash-<hash>`.

1. Reproduce: `cargo +nightly fuzz run <target> artifacts/<target>/crash-<hash>`
2. Minimize: `cargo +nightly fuzz tmin <target> artifacts/<target>/crash-<hash>`
3. Fix the reader so that input returns an `Err` (or satisfies the oracle).
4. Keep the artifact in `corpus/<target>/` so it replays on every local
   `-runs=0` pass.

## Notes

- The directory walks are depth-guarded (64 levels) with a flat work budget so
  a self-referential directory graph terminates instead of fanning out — that
  guard lives in the harnesses, not the libraries.
- Prefer adding a focused unit/integration regression under `crates/*/tests`
  for crashes that should gate PRs; keep fuzzing for discovery.
- Sync the grown corpus between machines with rsync and minimize periodically
  with `cargo +nightly fuzz cmin <target>`.
