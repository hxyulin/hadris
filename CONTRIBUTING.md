# Contributing to Hadris

Thanks for contributing. This document covers the day-to-day workflow for
library and CLI changes. The V3 API rules and layering are in
[`docs/v3-api-design.md`](docs/v3-api-design.md).

## Prerequisites

- Rust **1.88+** (see `rust-toolchain.toml` and workspace `rust-version`)
- Optional: [pre-commit](https://pre-commit.com/) for local fmt/clippy gates

```bash
pre-commit install
pre-commit install --hook-type pre-push
```

## Build and test

```bash
# Workspace (default features)
cargo check --workspace
cargo test --workspace --tests
cargo test --workspace --all-features --doc

# Match CI warning policy
RUSTFLAGS="-D warnings" cargo check --workspace

# No-std / feature tiers (examples)
RUSTFLAGS="-D warnings" cargo check -p hadris-fat --no-default-features --features "sync,write"
RUSTFLAGS="-D warnings" cargo check -p hadris-iso --no-default-features --features "sync"
```

The full per-crate feature matrix used in CI is the `check-features` job in
[`.github/workflows/rust.yml`](.github/workflows/rust.yml).

The `cross` job builds the `no_std` tiers for `thumbv6m-none-eabi`,
`thumbv7em-none-eabihf` and `riscv32imc-unknown-none-elf`; targets without
compare-and-swap skip the `alloc` tiers:

```bash
rustup target add thumbv6m-none-eabi thumbv7em-none-eabihf riscv32imc-unknown-none-elf
RUSTFLAGS="-D warnings" scripts/check-targets.sh
```

The `firmware-size` job builds `examples/firmware` for the same targets at
opt-level `s` with fat LTO and reports flash, the driver state, the mount
stack, the worst-case stack and the largest frame of each binary. It fails
when the mount stack or the driver state reaches 2 KB (NF-STACK-01), a
Hadris frame exceeds 1 KB (NF-STACK-02) or the FAT logger grows past its
flash ceiling. It needs the pinned nightly for `-Z emit-stack-sizes` and
`-Z print-type-sizes`:

```bash
rustup toolchain install nightly-2026-09-04 --component llvm-tools \
  --target thumbv6m-none-eabi,thumbv7em-none-eabihf,riscv32imc-unknown-none-elf
RUSTUP_TOOLCHAIN=nightly-2026-09-04 scripts/firmware-size.py --check
```

### Conformance and interoperability suite

Specification conformance and peer interoperability tests live in the
standalone `tests/` package (`hadris-tests`). Like `fuzz/`, it is detached from
the workspace so it can grow independently of the library crates and be
extracted later. `src/` holds the shared harness, the per-format semantic
models, the raw-image oracles, and one adapter per implementation (Hadris,
rust-fatfs, mtools, xorriso, mkisofs, and the native kernels). `suite/` holds
the tests, addressed as `<format>::<topic>::<name>`. See
[`tests/README.md`](tests/README.md) for the layout.

The oracles are the ground truth. Hadris is measured as one adapter among its
peers, and a Hadris-to-Hadris round trip is never evidence on its own.

```bash
# Hosted suite. Missing command-line peer tools are skipped locally.
cargo test --manifest-path tests/Cargo.toml

# One format or topic. Filters match substrings, so `fat::` also selects
# `exfat::`; CI runs the two separately.
cargo test --manifest-path tests/Cargo.toml fat:: -- --skip exfat::
cargo test --manifest-path tests/Cargo.toml exfat::
cargo test --manifest-path tests/Cargo.toml iso::boot::

# Strict tool-backed suite through the repository flake
nix develop -c env HADRIS_REQUIRE_EXTERNAL_TOOLS=1 \
  cargo test --manifest-path tests/Cargo.toml

# Match the CI lint and format gates for the suite
cargo fmt --manifest-path tests/Cargo.toml --all -- --check
cargo clippy --manifest-path tests/Cargo.toml --all-targets -- -D warnings
```

| Variable | Effect |
|----------|--------|
| `HADRIS_TESTS_KEEP=1` | Retain images and peer artifacts under the report directory |
| `HADRIS_TESTS_REPORT_DIR=<dir>` | Report root; defaults to `tests/target/reports` |
| `HADRIS_TESTS_SEED=<u64>` | Replay one generated FAT trace |
| `HADRIS_TESTS_NATIVE_MOUNT=1` | Enable privileged kernel mount tests |
| `HADRIS_REQUIRE_EXTERNAL_TOOLS=1` | Fail instead of skipping when a peer tool is missing |

#### FAT conformance

The FAT12/16/32 specification suite runs by default. Its ground truth is a
deterministic filesystem model plus a test-only raw-image oracle derived from
the FAT on-disk rules. The oracle independently checks geometry, mirrored FATs,
reserved entries, FAT32 metadata, cluster chains, cross-links, directory
records, LFN sequences and checksums, unique short aliases, names, attributes,
and contents.

The same harness measures mtools, dosfstools, and the independent Rust `fatfs`
crate against that ground truth. External-tool mismatches and crashes are
reported rather than treated as authoritative failures. The peer reports use
short, isolated scenarios so one failed operation does not mask the remainder
of a generated trace. The repository flake supplies mtools and dosfstools.

Besides the operation traces, every implementation is scored on rejection
scenarios (case-insensitive duplicates, moves into a directory's own subtree,
reserved characters, `.` and `..` names, over-long names) where the operation
must fail and the image must be unchanged, and on limit exercises sized from
the image geometry: filling a fixed FAT12/16 root directory to its last slot,
exhausting the data region and reclaiming it, and writing an extent that
spans most of the volume. The Hadris side of these runs in the hosted suite
under `fat::limits::`; a Hadris failure there is a library bug, not a peer
measurement.

```bash
cargo test --manifest-path tests/Cargo.toml \
  fat::spec::fat_spec_conformance -- --exact --nocapture

cargo test --manifest-path tests/Cargo.toml \
  fat::peers::fatfs_accuracy_report -- --ignored --exact --nocapture

nix develop -c cargo test --manifest-path tests/Cargo.toml \
  fat::peers::mtools_accuracy_report -- --ignored --exact --nocapture
```

The semantic model compares exact displayed paths, entry kinds, file contents,
stable FAT attributes, and the volume label. Timestamps are intentionally
excluded. Peer scores and failure details are written to
`tests/target/reports/fat/mtools-accuracy.txt` and
`tests/target/reports/fat/fatfs-accuracy.txt`. The specification suite and
the `fatfs` report run in well under a minute after compilation; the mtools
report spawns a process per operation and takes about two minutes.

Native platform qualification uses `mkfs.fat`, `fsck.fat`, and the kernel
`vfat` driver on Linux, and `newfs_msdos`, `fsck_msdos`, `hdiutil`, and the
kernel FAT driver on macOS.

```bash
# Native formatter and checker
cargo test --manifest-path tests/Cargo.toml \
  fat::native::native_platform_tools -- --ignored --nocapture

# Native writable mount; Linux needs root or passwordless sudo for mount/umount
HADRIS_TESTS_NATIVE_MOUNT=1 cargo test --manifest-path tests/Cargo.toml \
  fat::native::native_mount_roundtrip -- --ignored --nocapture --test-threads=1
```

The mount test operates only on temporary image copies and always attempts to
unmount them during cleanup. macOS AppleDouble `._*` files remain structurally
validated but are excluded from the semantic tree comparison as platform
metadata.

#### exFAT conformance

The exFAT slice reuses the FAT operation model and adapter trait with its own
raw-image oracle, and runs the Hadris driver through the same generic
`FileSystem` adapter. The hosted tests are under `exfat::spec::` and
`exfat::limits::`. `exfat::native::` checks Hadris images with exfatprogs
`fsck.exfat` (from the `PATH`, or the `hadris-exfatprogs` Docker image) and
macOS `fsck_exfat`, and writes to volumes made by `mkfs.exfat` and
`newfs_exfat`. The tests skip when no checker is available, unless
`HADRIS_REQUIRE_EXTERNAL_TOOLS=1` is set.

```bash
cargo test --manifest-path tests/Cargo.toml exfat::

# macOS kernel driver, on temporary image copies
HADRIS_TESTS_NATIVE_MOUNT=1 cargo test --manifest-path tests/Cargo.toml \
  exfat::native::native_mount_roundtrip -- --ignored --nocapture
```

#### ISO conformance

The ISO suite uses a test-only raw-image oracle derived from the ECMA-119:1987
primary-volume rules. It independently checks the descriptor sequence,
redundant endian fields, volume bounds, both path tables, directory hierarchy,
record bounds and padding, Level 1 identifiers, file extents, and file content.

The fast Hadris writer/reader check runs in the hosted suite:

```bash
cargo test --manifest-path tests/Cargo.toml \
  iso::spec::hadris_iso_matches_ecma_119_oracle -- --exact
```

The repository flake supplies xorriso/libisofs and the original cdrtools
`mkisofs`. The peer report checks images in both directions where the tool
supports them:

```bash
nix develop -c cargo test --manifest-path tests/Cargo.toml \
  iso::peers::external_iso_tool_accuracy_report -- --ignored --exact --nocapture
```

Native read qualification uses the Linux kernel ISO driver, macOS `hdiutil`,
or Windows `Mount-DiskImage`. Linux requires root or passwordless `sudo` for
`mount` and `umount`. The macOS producer report also measures `hdiutil
makehybrid`; peer deviations are reported rather than treated as ground truth.

```bash
HADRIS_TESTS_NATIVE_MOUNT=1 cargo test --manifest-path tests/Cargo.toml \
  iso::native::native_iso_reader_accuracy_report -- --ignored --exact --nocapture

cargo test --manifest-path tests/Cargo.toml \
  iso::native::native_iso_producer_accuracy_report -- --ignored --exact --nocapture
```

These peer reports are manual and are not part of CI. Test images and mount
points are temporary, and each native adapter attempts to detach or unmount
before returning. Summaries are written to `external-tools-accuracy.txt`,
`macos-hdiutil-accuracy.txt`, or `native-<os>-accuracy.txt` under
`tests/target/reports/iso/` even when a peer deviates from the specification.

## Package versions and releases

Each package declares its own version in its `Cargo.toml`, and
`[workspace.dependencies]` holds the requirement every other crate uses.
Every crate ships 3.0.0 together; after that each crate versions on its own
(R12 in [`docs/v3-api-design.md`](docs/v3-api-design.md)). A crate bumps its
major only for its own breaking changes, and the umbrella `hadris` bumps its
major when a crate it re-exports does. `hadris-fat-raw` versions separately
from 0.1.0. The examples are `publish = false`.

To release, bump the versions of the crates being released and of their
requirements in `[workspace.dependencies]`, and add a dated
`CHANGELOG.md` section for them, in a PR:

- one crate: `## [hadris-fat 3.1.0] - 2026-10-01`, the default heading for
  `<crate> <version>`;
- a joint release: one section for all of them, such as
  `## [3.0.0-rc.1] - 2026-10-01`, named with the workflow's `notes` input.

After that PR merges, run the `Release` workflow (Actions, Run workflow) on
`next`, or on `main` once 3.x lives there, first with `mode: dry-run` and
then with `mode: publish`:

- `crates`: `all`, or the crate names separated by spaces, such as
  `hadris-fat hadris`. A crate's unreleased workspace dependencies must be in
  the same run or already on crates.io.
- `notes`: empty for per-crate sections, or the section title of a joint
  release.

`scripts/release-plan.py` checks the plan in both modes: the crates exist
and are published, each version is a semantic version, the tag
`<crate>-v<version>` does not name another commit, and the CHANGELOG
section exists with a date. `dry-run` then runs fmt, check and tests, and
packages and verifies every crate with `cargo +stable publish --dry-run`,
which resolves unpublished workspace dependencies in the same run.
`publish` also publishes with `cargo +stable publish`, in dependency order,
skipping versions already on crates.io so a failed run can be rerun. It
then creates and pushes the tags `<crate>-v<version>`, one GitHub release
per tag (pre-releases for versions with a pre-release suffix; the umbrella
`hadris` is marked latest), and rebuilds the documentation site, which
versions itself from the `hadris-vX.Y.Z` tags. Never create release tags by
hand. Publishing needs the `CARGO_REGISTRY_TOKEN` repository secret.

Check a plan locally before opening the release PR:

```bash
scripts/release-plan.py --notes 3.0.0-rc.1 all
scripts/release-plan.py hadris-fat hadris
cargo +stable publish --dry-run -p hadris-fat -p hadris
```

The 2.x releases on `main` used one workspace version and `vX.Y.Z` tags.

## Pull requests

1. Keep changes focused; prefer small PRs over mixed refactors.
2. Update crate READMEs / rustdoc when public APIs or CLI commands change.
3. Add a `[Unreleased]` note in [CHANGELOG.md](CHANGELOG.md) for user-visible work.
4. Do not commit secrets or large binary fixtures unless they are intentional
   corpus seeds under `fuzz/corpus/`.
5. PRs to `main` and `next` also run the V3 guardrails in
   `.github/workflows/v3-guardrails.yml`, and a finding fails the PR:
   `scripts/check-non-exhaustive.py`, `scripts/check-v3-api.py subset` and
   `scripts/check-v3-api.py parity` (under the pinned nightly of the public
   API job), and `scripts/check-semver.sh`, which needs
   `cargo +stable install cargo-semver-checks`.
6. `scripts/check-semver.sh` checks each library crate against its latest
   3.x release tag (`<crate>-vX.Y.Z`), or against the target branch before
   the crate's 3.0.0. While the version is unchanged, a PR must be a
   compatible minor change, release candidates included. A deliberate break
   bumps the crate's version in the same PR: the next release candidate
   (`3.0.0-rc.2`) before 3.0.0, the next major after it.

## Safety and fuzzing

- When touching `unsafe`, LFN/UTF-16, or disk-byte to `&str` paths, add a
  regression test and run the targeted Miri job from the `miri` job in
  [`.github/workflows/rust.yml`](.github/workflows/rust.yml):

  ```bash
  cargo +nightly miri test -p hadris-common --lib
  cargo +nightly miri test -p hadris-fat-raw --lib
  cargo +nightly miri test -p hadris-iso --lib -- raw:: name:: rock_ridge::
  cargo +nightly miri test -p hadris-part --lib
  cargo +nightly miri test -p hadris-ntfs --lib
  ```

- Fuzz harnesses under [`fuzz/`](fuzz/) are **local tools** (not part of PR CI).
  Replay corpora with `cargo +nightly fuzz run <target> -- -runs=0` after
  parser fixes; prefer a normal unit/integration test for PR-gating regressions.

## Spec annotations

When changing on-disk layouts or public parse/format entry points for a
standard section, follow the annotation convention and sync the coverage table
in [`docs/spec-coverage.md`](docs/spec-coverage.md#annotation-convention).

- `full` needs `@hadris-tests`; `@hadris-fuzz` is optional additional coverage.
- `partial` needs `@hadris-note` describing the gap.
- Fuzz targets are local discovery tools, not CI gates.

CI runs the grammar + table-sync check (never `cargo fuzz`):

```bash
python3 scripts/check-spec-annotations.py --self-test
python3 scripts/check-spec-annotations.py
python3 scripts/check-compliance-catalog.py --self-test
python3 scripts/check-compliance-catalog.py
```

## Docs

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo test --workspace --all-features --doc
python3 scripts/check-docs.py

# Task-oriented documentation site
cd website
npm ci
npm run build

# With every released version (needs full history and tags)
npm run versions
npm run build
```

Keep the documentation layers focused: the root README is the project
overview, the website contains concepts and workflows, crate READMEs cover
package selection and features, and rustdoc documents individual APIs. Prefer
linking to compiled examples over duplicating snippets that can drift.

Public APIs are snapshot-tested under their all-feature configurations. After
an intentional additive or breaking API change, review the diff and refresh the
baseline with:

```bash
scripts/check-public-api.sh update
```

The snapshot is a review aid, not a feature freeze. Backward-compatible APIs
are welcome in a minor release when their documentation, feature-matrix tier,
and tests land with them.

Feature-gated items should use `#[cfg_attr(docsrs, doc(cfg(...)))]` where the
crate already enables `docsrs` (see `hadris-part`, `hadris`).

## License

By contributing, you agree that your contributions are licensed under the
[MIT license](LICENSE-MIT).
