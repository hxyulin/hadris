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
standard section, add or update `@hadris-spec` tags (see
[`docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`](docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md))
and sync [`docs/spec-coverage.md`](docs/spec-coverage.md).

- `full` needs `@hadris-tests` and/or `@hadris-fuzz`.
- `partial` needs `@hadris-note` describing the gap.
- Fuzz targets are local discovery tools, not CI gates.

CI runs the grammar + table-sync check (never `cargo fuzz`):

```bash
python3 scripts/check-spec-annotations.py --self-test
python3 scripts/check-spec-annotations.py
```

## Docs

```bash
cargo doc --workspace --no-deps --document-private-items
```

Feature-gated items should use `#[cfg_attr(docsrs, doc(cfg(...)))]` where the
crate already enables `docsrs` (see `hadris-part`, `hadris-fat`).

## License

By contributing, you agree that your contributions are licensed under the
[MIT license](LICENSE-MIT).
