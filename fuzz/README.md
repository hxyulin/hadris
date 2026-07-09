# Fuzzing

Coverage-guided fuzz harnesses for the untrusted-input parsers, one per reader.
These are **local / developer tools** — they are intentionally **not** run in CI
(nightly + long-running; corpus replay belongs in a developer workflow or a
separate scheduled job outside the PR gate).

| Target      | Entry point                          | Exercises |
|-------------|--------------------------------------|-----------|
| `cpio_read` | `CpioReader::next_entry_alloc` + data | newc header / `namesize` / `filesize` parsing |
| `fat_read`  | `FatFs::open` + recursive read       | BPB, FAT chain, directory + LFN parsing, file reads |
| `iso_read`  | `IsoImage::open` + recursive read    | volume descriptors, directory records, RRIP, multi-extent reads |
| `udf_read`  | `UdfFs::open` + recursive read       | anchor/VDS/FSD, File Entry, allocation descriptors, FIDs |

**The invariant:** feeding *arbitrary bytes* into a reader must only ever return
an `Err` or succeed — never panic, abort, or OOM. A crash found here is a bug in
the reader, not the harness.

## Running

```bash
cargo install cargo-fuzz            # one-time
rustup toolchain install nightly    # cargo-fuzz needs nightly

cargo +nightly fuzz run cpio_read                      # fuzz until a crash / Ctrl-C
cargo +nightly fuzz run cpio_read -- -max_total_time=60 # time-boxed
cargo +nightly fuzz run cpio_read -- -runs=0           # replay committed corpus only, then exit
```

Replay every committed corpus after pulling or before a release:

```bash
for t in cpio_read fat_read iso_read udf_read; do
  cargo +nightly fuzz run "$t" -- -runs=0
done
```

## When a crash is found

cargo-fuzz writes the crashing input to `artifacts/<target>/crash-<hash>`.

1. Reproduce: `cargo +nightly fuzz run <target> artifacts/<target>/crash-<hash>`
2. Fix the reader so that input returns an `Err`.
3. Lock in the regression: copy the artifact into the target's seed corpus so it
   is replayed on every local `-runs=0` pass:
   ```bash
   cp artifacts/<target>/crash-<hash> corpus/<target>/
   ```

## Notes

- The seed corpus is committed (`corpus/`); build artifacts and discovered
  crashes are git-ignored until you promote them into `corpus/`.
- The directory walks are depth-guarded (64 levels) so a self-referential
  directory terminates instead of looping — that guard lives in the harness, not
  the libraries.
- `cpio_read` is seeded with the two allocation-DoS inputs that motivated this
  setup; they now replay cleanly.
- Prefer adding a focused unit/integration regression under `crates/*/tests`
  for crashes that should gate PRs; keep fuzzing for discovery.
