# Contributing to Hadris

Thanks for contributing. This document covers the day-to-day workflow for
library and CLI changes. The V3 API rules and layering are in
[`docs/v3-api-design.md`](docs/v3-api-design.md).

## Prerequisites

- Rust **1.88+** (see `rust-toolchain.toml` and workspace `rust-version`)
- Python **3.11+** for repository checks, provided by `nix develop`
- Optional: [prek](https://prek.j178.dev/) for local format/lint gates
  (also provided by `nix develop`)

```bash
prek install
prek run --all-files
```

Rust hooks check both the workspace and the detached conformance suite at
`tests/Cargo.toml`, including all suite targets for Clippy.

## Editor setup and search

EditorConfig supplies indentation, UTF-8 and LF defaults. `rustfmt.toml` pins
Rust's 2024 style edition and LF output using stable options supported by the
MSRV; rustfmt owns Rust layout. Git attributes normalize text, including
benchmark CSVs, to LF and treat disk fixtures and fuzz seeds as binary.

VS Code recommends rust-analyzer and EditorConfig, formats Rust on save, and
links both the main workspace and the detached `tests/Cargo.toml` suite. The
Tasks menu offers workspace checks, workspace tests and conformance tests.
Launch the editor from `nix develop` to inherit the native tools. The nightly
`fuzz/Cargo.toml` project is opt-in: add it to `rust-analyzer.linkedProjects`
when working on fuzz targets. Feature-tier checks remain explicit commands so
the editor follows the default hosted configuration.

Generated files are excluded through `.gitignore`; `.rgignore` additionally
omits lockfiles from broad ripgrep searches. Search a lockfile by passing its
path explicitly, for example `rg 'fatfs' tests/Cargo.lock`. Use `rg --hidden`
to include dotfiles while respecting ignore rules. Personal VS Code files are
ignored; shared settings, tasks and extension recommendations are tracked.

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

The full per-crate feature matrix used by the `check-features` job is in
[`scripts/ci-features.json`](scripts/ci-features.json).

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

Hosted CI formatting, linting and tests use Rust **1.97.1**; Rust **1.88.0**
remains the library MSRV. Feature tiers are checked independently from
[`scripts/ci-features.json`](scripts/ci-features.json):

```bash
RUSTUP_TOOLCHAIN=1.88.0 RUSTFLAGS="-D warnings" python3 scripts/check-features.py all
```

### CI coverage

FAT/exFAT and ISO 9660 formal proofs run in the separate daily and manually
triggered [`fat-proofs.yml`](.github/workflows/fat-proofs.yml) and
[`iso-proofs.yml`](.github/workflows/iso-proofs.yml), rather than on every PR.
PR checks validate the proof inventories and runner. Run
`python3 scripts/verify-proofs.py fat` or `python3 scripts/verify-proofs.py iso`
locally with the pinned Kani installation, or add `--group <name>` to select one
group. The [FAT](docs/fat-verification.md) and [ISO](docs/iso-verification.md)
verification guides document proof scopes, reports and schedule limitations.
Scheduled results apply to the tested commit and do not block an earlier merge.

Rust CI always reports the required `Check`, `Test (ubuntu-latest)`, `Format`
and `Clippy` contexts, and runs `Documentation links` (`scripts/check-docs.py`)
on every change. Workflow path globs select expensive jobs:

| Changed inputs | Coverage |
|---|---|
| Library/example sources, fixtures, manifests, toolchain or build-check tooling | Workspace tests, MSRV tiers, embedded builds, Miri, fuzz targets, docs and interoperability |
| Detached `tests/` suite | Suite formatting, Clippy and conformance/interoperability jobs |
| Detached `fuzz/` harnesses | Fuzz targets: build every harness, replay the committed seeds; on pushes to main, also fuzz each target for 10 seconds |
| Native-tool setup or peer helper scripts | Native and interoperability jobs |
| Rustfmt configuration | Workspace and conformance-suite formatting |
| Specification catalogs or checkers | Specification checks |
| Manifests, lockfiles or `deny.toml` | Dependency policy: `cargo deny check` (advisories, bans, licenses, sources) for the root, `tests/` and `fuzz/` workspaces |
| README, website or unrelated prose | Documentation workflow; Rust builds/tests are skipped |

[`supply-chain.yml`](.github/workflows/supply-chain.yml) repeats the
dependency policy weekly, so a new RustSec advisory against a locked
dependency fails without a manifest change. Run it locally with
`cargo deny check` and
`cargo deny --manifest-path tests/Cargo.toml check` (likewise for `fuzz/`).
Push runs on `main` are never cancelled by a later push, so every main
commit gets a complete CI and semver run.

The path-filter action's rules live in `rust.yml`; no custom change-detection
script is needed. Required checks still complete for docs-only PRs, and a
failed scope job fails `Check`. V3 guardrails use workflow-level paths because
they are not required branch-protection contexts. Rust source changes retain
full platform coverage. CodeQL is configured separately through GitHub's
code-scanning settings.

### Performance measurements

The FAT benchmark measures device calls and bytes as well as runtime for the
hosted and embedded drivers. Run it with
`cargo bench -p hadris-fat --bench performance`; see
[`docs/performance.md`](docs/performance.md) for filtering, CSV baselines,
measurement boundaries and embedded flash/stack measurements.

The ISO benchmark runs with
`cargo bench -p hadris-iso --bench performance`; the same
performance guide documents its workloads, cache settings and measurement
boundaries.

The detached conformance harness also runs common V3 filesystem workloads with
per-sample timing and I/O CSV output. Run
`cargo bench --manifest-path tests/Cargo.toml --bench performance`; see
[`tests/README.md`](tests/README.md#v3-performance-harness) for measurement
boundaries and smoke-check settings. The `peers` benchmark compares host
workflows against independent implementations;
[`docs/peer-performance.md`](docs/peer-performance.md) documents the reference
coverage and comparison boundaries.

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

# Strict tool-backed suite through the repository flake (Linux)
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
Every stable crate ships 3.0.0 together; after that each crate versions on
its own (R12 in [`docs/v3-api-design.md`](docs/v3-api-design.md)). A crate
bumps its major only for its own breaking changes, and the umbrella `hadris`
bumps its major when a stable crate it re-exports does. The raw crates
version separately on 0.x (see below). The preview crates `hadris-ntfs` and
`hadris-apfs`, and the internal `hadris-common`, keep 3.x versions but are
exempt from the semver promise: a breaking change to them bumps their minor,
never their major, and users pin them with `~3.0`. Since the umbrella reaches
the previews only through `unstable-*` features, which are outside semver, it
never bumps `hadris`'s major either. The examples are `publish = false`.

The raw crates (`hadris-fat-raw`, `hadris-iso-raw`, `hadris-udf-raw`,
`hadris-cpio-raw`) stay at 0.x and version on their own; a breaking change
bumps their 0.x minor. A format crate re-exports only the raw items its own
public signatures use, never the whole raw crate
(`scripts/check-non-exhaustive.py` rejects `pub use hadris_<format>_raw::*`).
Those re-exported items are frozen for the format crate's major: change them
only additively, and treat any other change as a breaking change of the
format crate. A format crate keeps its raw dependency on one 0.x line for
its whole major: moving to the next 0.y changes the identity of the
re-exported types, so it is a breaking change of the format crate too. [`website/docs/stability.md`](website/docs/stability.md#raw-crates)
lists them.

Changes collect in the undated section at the top of `CHANGELOG.md`, such
as `## [3.0.0-rc.2] - Unreleased`, under Keep a Changelog subsections
(Breaking first, then Added, Changed, Removed, Fixed and Security). To
release, bump the versions of the crates being released and of their
requirements in `[workspace.dependencies]`, and date the `CHANGELOG.md`
section for them, in a PR:

- one crate: `## [hadris-fat 3.1.0] - 2026-10-01`, the default heading for
  `<crate> <version>`;
- a joint release: one section for all of them, such as
  `## [3.0.0-rc.2] - 2026-10-07`, named with the workflow's `notes` input.

Date a section only in its release PR. `scripts/release-plan.py` refuses a
dated workspace section `## [<version>]` whose `v<version>` tag does not
exist, other than the one being released. For 3.0.0, consolidate the
`3.0.0-rc.*` sections into one `## [3.0.0]` entry against 2.5.0, with the
same subsections, and point its compare link at `v2.5.0...v3.0.0`.

After that PR merges, run the `Release` workflow (Actions, Run workflow) on
`main`, first with `mode: dry-run` and
then with `mode: publish`:

- `crates`: `all`, or the crate names separated by spaces, such as
  `hadris-fat hadris`. A crate's unreleased workspace dependencies must be in
  the same run or already on crates.io. With `all`, a crate whose tag
  `<crate>-v<version>` already names an earlier commit of `main` was released
  before at that version and is left out with a notice, so a joint 3.x
  release does not need the unchanged 0.x raw crates bumped. Naming such a
  crate explicitly fails.
- `notes`: empty for per-crate sections, or the section title of a joint
  release.

Every mode first checks that it runs from `main`. `scripts/release-plan.py`
checks the plan in both release modes: the crates exist and are published,
each version is a semantic version, the tag `<crate>-v<version>` does not
name another commit, the crate's `[workspace.dependencies]` requirement
names exactly its version (`X.Y.Z`, `^X.Y.Z`, `=X.Y.Z` or `~X.Y.Z`), and the
CHANGELOG section exists with a date. The release job then looks up the
push runs of Rust CI and, when it ran, V3 guardrails for the commit:
`publish` refuses to continue unless they succeeded, and `dry-run` only
warns, so wait for CI on `main` before publishing. It runs fmt, check and
tests, and packages and verifies every crate with
`cargo publish --dry-run` on the pinned 1.97.1 toolchain, which resolves
unpublished workspace dependencies in the same run. When the plan includes
`hadris-cli`, both modes also build its release binaries (below) from the
commit before anything is published.

`publish` then publishes with `cargo publish --no-verify` (the dry run
verified the same commit), in dependency order, skipping versions already on
crates.io so a failed run can be rerun; any crates.io answer other than
found or not found stops the job instead of being read as "not published".
It then creates and pushes the tags `<crate>-v<version>` and one GitHub
release per tag; versions with a pre-release suffix become pre-releases. A
joint release with `notes` set that includes `hadris` also creates a
workspace tag `v<version>` and a `Hadris <version>` GitHub release using the
umbrella crate's version and the shared notes. When stable, the workspace
release `v<version>` is marked latest in a joint release, and otherwise
`hadris-v<version>` when `hadris` is released. Both tag formats are checked
before publication, and reruns skip releases that already exist. It then
rebuilds the documentation site, which versions itself from the
`hadris-vX.Y.Z` tags. Never create release tags by hand.

Only the `publish` job reads crates.io credentials, and it runs in the
`release` deployment environment. Restrict that environment to `main` and
give it a required reviewer in the repository settings. It uses crates.io
trusted publishing when the repository variable
`CRATES_IO_TRUSTED_PUBLISHING` is `true` (every published crate must list
`release.yml` and the `release` environment as its trusted publisher), and
otherwise the `CARGO_REGISTRY_TOKEN` secret. A crate's first release still
needs the token, since trusted publishing applies only to existing crates.

Releasing `hadris-cli` also builds the unified `hadris` binary for Linux
x86-64/ARM64 (static musl), macOS Intel/Apple Silicon (macOS 11+) and
Windows x86-64. Every native build runs the APFS CLI regressions and checks
the version and APFS command help. Archives contain the binary, README and
MIT license in `hadris-cli-<version>-<target>/`; Windows uses ZIP and other
targets use TGZ. Each archive has a matching `.sha256` checksum. All target
builds must succeed before any crate is published; after the GitHub
releases exist, the same archives are attached to the per-crate release,
once the CLI tag is confirmed to name the commit they were built from.

To backfill an existing CLI release, run `Release` with `mode: cli-assets`
and `cli-tag: hadris-cli-v3.0.0-rc.1`. This mode only builds and uploads
assets; it does not publish crates or create tags. Set
`cli-workspace-tag: v3.0.0-rc.1` to mirror assets onto the workspace release
for older crates published without binstall metadata. The tags must have the
same version and point to the same commit. Uploads preserve existing binaries
and can resume when an archive or its checksum is missing. Future published
CLI manifests carry explicit binstall metadata for the per-crate release.

RC2 removes the standalone `hadris-apfs-cli` workspace package. APFS commands
ship in `hadris-cli`; use `hadris apfs`.

Check a plan locally before opening the release PR:

```bash
scripts/release-plan.py --notes 3.0.0-rc.2 all
scripts/release-plan.py --github-plan --notes 3.0.0-rc.2 all
scripts/release-plan.py hadris-fat hadris
cargo +1.97.1 publish --dry-run -p hadris-fat -p hadris
```

The 2.x releases on `main` used one workspace version and `vX.Y.Z` tags.

## Pull requests

Branch from `main` and target it for V3 changes. The `v2` branch preserves the
2.x maintenance line. `next` is retired from the development workflow.

1. Keep changes focused; prefer small PRs over mixed refactors.
2. Update crate READMEs / rustdoc when public APIs or CLI commands change.
3. Add a note for user-visible work to the undated top section of
   [CHANGELOG.md](CHANGELOG.md), under its Breaking, Added, Changed, Removed,
   Fixed or Security subsection.
4. Do not commit secrets or large binary fixtures unless they are intentional
   corpus seeds under `fuzz/corpus/`.
5. PRs to `main` also run the V3 guardrails in
   `.github/workflows/v3-guardrails.yml`, and a finding fails the PR:
   `scripts/check-non-exhaustive.py` and `scripts/check-semver.sh`, which needs
   `cargo +stable install cargo-semver-checks`.
6. `scripts/check-semver.sh` checks each library crate against its latest
   3.x release tag (`<crate>-vX.Y.Z`), or against the target branch before
   the crate's 3.0.0. While a released version is unchanged, a PR must be a
   compatible minor change, release candidates included. A deliberate break
   bumps the crate's version in the same PR, to the next pre-release or
   major. A version that is not released yet, such as `3.0.0-rc.2` while
   only `3.0.0-rc.1` is tagged, is compared against the previous
   pre-release and its breaking changes are reported as warnings without
   failing the PR.

## Safety and fuzzing

- When touching `unsafe`, LFN/UTF-16, or disk-byte to `&str` paths, add a
  regression test and run the targeted Miri job from the `miri` job in
  [`.github/workflows/rust.yml`](.github/workflows/rust.yml):

  CI pins Miri to `nightly-2026-10-10`:

  ```bash
  rustup toolchain install nightly-2026-10-10 --component miri,rust-src
  export RUSTUP_TOOLCHAIN=nightly-2026-10-10
  cargo miri test -p hadris-common --lib
  cargo miri test -p hadris-fat-raw --lib
  cargo miri test -p hadris-iso-raw --lib
  cargo miri test -p hadris-iso --lib -- name:: rock_ridge::
  cargo miri test -p hadris-fs --lib name::
  cargo miri test -p hadris-part --lib
  cargo miri test -p hadris-ntfs --lib
  ```

- Fuzz harnesses live under [`fuzz/`](fuzz/). Rust CI's `Fuzz targets` job
  builds them on `nightly-2026-09-04`, replays the committed seeds with
  `-runs=0`, and on pushes to main fuzzes each target for 10 seconds and
  uploads any crash input as the `fuzz-artifacts` artifact; long runs stay
  local.
  Replay corpora with `cargo +nightly fuzz run <target> -- -runs=0` after
  parser fixes, and turn each crash into a normal unit or integration test in
  the owning crate.

## Spec annotations

When changing on-disk layouts or public parse/format entry points for a
standard section, follow the annotation convention and sync the coverage table
in [`docs/spec-coverage.md`](docs/spec-coverage.md#annotation-convention).

- `full` needs `@hadris-tests`; `@hadris-fuzz` is optional additional coverage.
- `partial` needs `@hadris-note` describing the gap.
- `@hadris-fuzz` names a discovery tool; CI only builds the fuzz targets and
  replays their committed seeds, so it is not test evidence.

CI runs the grammar + table-sync check:

```bash
python3 scripts/check-spec-annotations.py --self-test
python3 scripts/check-spec-annotations.py
python3 scripts/check-compliance-catalog.py --self-test
python3 scripts/check-compliance-catalog.py
```

## Docs

Workspace members inherit Clippy checks for unsafe safety comments and the
`# Safety`, `# Errors` and `# Panics` rustdoc sections from the root
`Cargo.toml`. Explain the conditions and partial effects of each operation.
These checks run with the normal Clippy command and fail CI under `-D warnings`.
Clippy does not find every possible panic, so review indexing and arithmetic
on caller-controlled values as well.

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

The repository's Python scripts need Python 3.11 or newer; `nix develop`
provides one, or run them with `uv run --python 3.12 scripts/check-docs.py`.

Keep the documentation layers focused: the root README is the project
overview, the website contains concepts and workflows, crate READMEs cover
package selection and features, and rustdoc documents individual APIs. Prefer
linking to compiled examples over duplicating snippets that can drift.
Crate README Rust blocks compile as doctests: each library's `lib.rs` includes
its README in a `ReadmeDoctests` item, built with the crate's default
features, so `cargo test --doc` covers them (mark a block `rust,ignore` when it
cannot compile on its own). Snippets in `website/docs`, the migration guide and
the `hadris-macros` README, whose example needs `hadris-io`, are not compiled;
keep them close to a compiled example or doctest.

Public API compatibility is checked by `scripts/check-semver.sh`, using
`cargo-semver-checks` on stable Rust. Its default feature selection includes
stable optional features and excludes conventional unstable feature names.
The MSRV feature matrix and sync/async contract tests remain separate checks.
Review feature-dependent shapes and mode parity when changing shared APIs:
semver compares revisions, not feature configurations or I/O modes.

```bash
cargo +stable install cargo-semver-checks --locked
scripts/check-semver.sh
```

No textual API snapshots need refreshing. An untagged version may still change;
a tagged release candidate must remain compatible unless its version is bumped.
Backward-compatible APIs are welcome with documentation, feature-tier coverage
and tests.

Every published library crate enables `#![cfg_attr(docsrs, feature(doc_cfg))]`
and sets `rustdoc-args = ["--cfg", "docsrs"]` in `[package.metadata.docs.rs]`,
so docs.rs labels feature-gated items. Check a docs.rs build with
`RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc --workspace --no-deps --all-features`.
Each crate ships `LICENSE-MIT` as a link to the root license, and a crate
README must live in its crate directory so its relative links resolve on
crates.io; `scripts/check-docs.py` checks both.

## License

By contributing, you agree that your contributions are licensed under the
[MIT license](LICENSE-MIT).
