# Hadris 2.0 Feature Model

Status: draft specification; implementation in progress

## Goals

- `sync` and `async` select API modes, not platform capabilities.
- `std` and `alloc` select platform capabilities and must not implicitly choose an
  API mode.
- `read`, `write`, and format extensions select behavior independently where the
  implementation permits it.
- Enabling sync and async together exposes both modules and does not select one by
  precedence.
- `--all-features` compiles for every library crate.

## Standard feature contract

- `default`: the crate's documented ergonomic hosted configuration.
- `std`: standard-library integration; implies `alloc`, but not `sync`.
- `alloc`: allocation-backed APIs without requiring `std`.
- `sync`: `crate::sync` using `hadris_io::sync` traits.
- `async`: `crate::async` using `hadris_io::async` traits.
- `read`: parsing and read-only high-level APIs.
- `write`: creation and mutation APIs; dependencies on `read`, `alloc`, or `std`
  must be stated by the individual format.

For compatibility, a format crate may re-export `sync` at its root when `sync` is
enabled. Code shared by both modes must not resolve mode-specific types through
those root re-exports.

## Required checks

Each applicable crate must pass with warnings denied:

1. default features;
2. `--no-default-features`;
3. each of `sync` and `async` independently with the minimum read tier;
4. `sync,async` together;
5. hosted read/write configurations for each mode;
6. `--all-features`;
7. the documented no-std tiers.

The umbrella and category facades additionally test every leaf and category
feature independently.

## Current blockers

- ISO and UDF currently make `std` imply `sync`, preventing a hosted async-only
  configuration.
- UDF compiles descriptor modules once per mode while shared `dir` and `file`
  modules refer to descriptors through crate-root sync re-exports. With both modes
  enabled, async code receives sync descriptor values and fails nominal type
  checks.
- UDF write code contains synchronous-only implementations inside files included
  by the async module.
- ISO has imports and fields used only by one mode without matching gates, and its
  write/modify surface is not yet genuinely async-capable.
- Existing no-std warning-denied checks expose independent ISO dead-code/import
  issues and FAT generic error-type mismatches.

## Repair sequence

1. Separate mode-neutral raw/on-disk types from mode-specific I/O operations.
2. Make shared high-level types import mode-neutral types directly, never through
   `crate` re-exports whose meaning changes with features.
3. Gate genuinely sync-only write or modification APIs explicitly until an async
   implementation exists; do not publish a generated async signature backed by
   synchronous method bodies.
4. Remove `sync` implications from `std`, then set the desired mode explicitly in
   each crate's default feature list.
5. Add the matrix to CI and only then forward both modes through facades.

This repair may intentionally remove previously advertised async write APIs that
never compiled or behaved asynchronously. Such removals are acceptable for 2.0
and must be listed in the migration guide.
