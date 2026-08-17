# Agent Instructions

Hadris is a Rust workspace containing filesystem and disk utility implementations.
The project emphasizes no-std compatibility, dual sync/async support, and comprehensive extension support.

### Code Style

- Limit the amount of comments you put in the code to a strict minimum. You should almost never add comments, except sometimes on non-trivial code, function definitions if the arguments aren't self-explanatory, and class definitions and their members.
- Do not use emoji.

### Workflow

- Keep changes focused and run targeted crate tests while iterating.
- Before finishing, run `cargo fmt --all -- --check`, `RUSTFLAGS="-D warnings" cargo check --workspace`, and tests covering the affected crate or behavior.
- For changes to I/O, error handling, or feature-gated code, also check the affected crate without default features using the relevant CI feature tier.
- Preserve `no_std` compatibility and dual sync/async support. Use `hadris-io` abstractions instead of `std::io` in shared library code.
- When touching `unsafe` code or disk-byte-to-string conversions, add a regression test and run the relevant targeted Miri test.
- Update rustdoc or README documentation for public API changes and `CHANGELOG.md` for user-visible changes.
- Consult `CONTRIBUTING.md` and `.github/workflows/rust.yml` for specialized checks rather than duplicating their full command matrices here.
