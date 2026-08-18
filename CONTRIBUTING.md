# Contributing to Hadris

Thanks for contributing. This document covers the day-to-day workflow for
library and CLI changes. Deeper architecture notes live in [CLAUDE.md](CLAUDE.md).

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
cargo test --workspace

# Match CI warning policy
RUSTFLAGS="-D warnings" cargo check --workspace

# No-std / feature tiers (examples)
RUSTFLAGS="-D warnings" cargo check -p hadris-fat --no-default-features --features "read,sync"
RUSTFLAGS="-D warnings" cargo check -p hadris-iso --no-default-features --features "read,sync"
```

See [CLAUDE.md](CLAUDE.md) for the full per-crate feature matrix used in CI.

## Package versions

Each package declares its own version in its `Cargo.toml`; the workspace does
not impose a shared version. Update only the packages being released and keep
their requirements in `[workspace.dependencies]` aligned.

When several unpublished versions depend on one another, publish in dependency
order: `hadris-macros`/`hadris-io`/`hadris-path`; then
`hadris-common`/`hadris-storage`/`hadris-part`; then format crates; then category
facades and `hadris-cd`; then the `hadris` umbrella and CLI packages. Cargo
validates dependent packages against crates.io, so each prerequisite version
must be available before packaging the next layer.

The `Release` GitHub Actions workflow automates this ordering for coordinated
workspace releases. Run it from `main` in `dry-run` mode first, then rerun it in
`publish` mode. Publishing requires a `CARGO_REGISTRY_TOKEN` repository secret.
The workflow derives the tag and GitHub release notes from the matching
`CHANGELOG.md` section and can safely resume after a partially completed
crates.io publication.

## Pull requests

1. Keep changes focused; prefer small PRs over mixed refactors.
2. Update crate READMEs / rustdoc when public APIs or CLI commands change.
3. Add a `[Unreleased]` note in [CHANGELOG.md](CHANGELOG.md) for user-visible work.
4. Do not commit secrets or large binary fixtures unless they are intentional
   corpus seeds under `fuzz/corpus/`.

## Safety and fuzzing

- When touching `unsafe`, LFN/UTF-16, or disk-byte → `&str` paths, run the
  targeted Miri jobs documented in [CLAUDE.md](CLAUDE.md).
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
cargo doc --workspace --no-deps --document-private-items
python3 scripts/check-docs.py

# Task-oriented documentation site
cd website
npm ci
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
are welcome in the 2.x series when their documentation, feature-matrix tier,
and tests land with them.

Feature-gated items should use `#[cfg_attr(docsrs, doc(cfg(...)))]` where the
crate already enables `docsrs` (see `hadris-part`, `hadris-fat`).

## License

By contributing, you agree that your contributions are licensed under the
[MIT license](LICENSE-MIT).
