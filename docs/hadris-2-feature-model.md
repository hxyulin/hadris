# Hadris 2.0 Feature Model

Status: accepted specification; implemented across current leaves and facades

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

No known feature-composition blockers remain in the current format leaves or
umbrella facade. Core support crates and future category meta-crates should adopt
the same contract before 2.0.

## Implementation status

### Virtual paths

`hadris-path` provides an independent `no_std`, allocation-free lexical path
core with optional allocation-backed normalization. It has no filesystem I/O or
format dependencies. Current FAT, exFAT, ISO, UDF, RRIP, and layout consumers
share its component and separator semantics while retaining format-specific
name matching.

### Fixed-capacity data

`hadris-fixed` provides an independent allocation-free `FixedBytes` and
`FixedStr` core plus endian-aware fixed UTF-16 storage. Its `alloc` feature adds
owned UTF-16 decoding, while `bytemuck` enables zero-copy use of the fixed-width
UTF-16 representations. It has no filesystem or I/O dependencies.

### UDF pilot

UDF now follows the contract for its implemented capabilities:

- `std` no longer enables `sync`;
- sync-only, async-only, combined, default, and all-feature builds pass with
  warnings denied;
- descriptor, directory, and file model types are owned by their API mode, so a
  value can never accidentally cross between nominally distinct sync and async
  descriptor types through a crate-root compatibility re-export;
- write and modification APIs are currently exported only by `sync`, because the
  implementation is synchronous. Async write will be added only with genuinely
  asynchronous I/O operations.

The compatibility root re-exports still select sync whenever sync is enabled.
New 2.0 code should use `hadris_udf::sync` or `hadris_udf::async` explicitly.

### ISO

ISO now follows the same contract:

- `std` no longer enables `sync`, while the default feature set selects `sync`
  explicitly;
- bare, sync-only, async-only, hosted async-only, combined, default, and
  all-capability builds pass with warnings denied;
- synchronous iterators and their imports are confined to the sync module;
- write, modification, and El-Torito writer APIs are exported only by `sync`;
- both API modes can be documented and compiled in the same build.

ISO async reading now includes collection-oriented directory traversal and
lookup through `IsoDir::read_entries`, `IsoDir::find`, and
`IsoImage::find_path`, with nested-file integration coverage through the
optical facade. A genuine async writer remains a future feature, not a 2.0
feature-composition or release blocker.

### FAT

FAT now follows the shared contract:

- `std` no longer enables `sync`, while defaults select `sync` explicitly;
- no-std sync, async-only, hosted async read/write, combined read/write, default,
  and all-capability configurations pass with warnings denied;
- device-specific seek errors are erased only when entering the non-generic
  public `FatError`, preserving generic storage implementations;
- sync-only formatter tests are not instantiated against the async API;
- `cache`, `tool`, and the root-level `unstable-exfat` preview remain explicitly
  sync-only capabilities and therefore imply `sync`; the preview is excluded
  from the stable V2 API snapshot, while the core FAT read/write surface
  supports either or both modes.

Async FAT detection, opening, nested traversal, multi-cluster file-content
read/write, truncation, duplicate rejection, and source recovery have runtime
integration coverage through the block facade. Further filesystem capabilities
may still be added before 2.0, but the current async mutation surface is
release-qualified.

### Partition tables

`hadris-part` now follows the shared contract:

- `std` no longer enables `sync`, while defaults select `sync` explicitly;
- bare, no-std sync, async-only, hosted async read/write, combined, default, and
  all-capability builds pass with warnings denied;
- the same mode-neutral MBR/GPT/hybrid types back both I/O modules, avoiding the
  nominal-type duplication encountered during the UDF pilot;
- all-feature builds use the current `rand` trait API for GUID generation.

Partition-table async I/O has runtime MBR, GPT, and hybrid write/open round
trips, corruption and truncation checks, non-destructive detection coverage,
and an end-to-end GPT partition view opened as FAT through `hadris-block`.

### CPIO

`hadris-cpio` now follows the shared contract:

- `std` no longer enables `sync`, while defaults select `sync` explicitly;
- bare, no-std sync, async-only, hosted async read/write, combined, default, and
  all-capability builds pass with warnings denied;
- entry/header representation remains local to each generated mode today, but no
  shared root type resolves through a mode-dependent compatibility re-export, so
  both modules coexist without nominal-type leakage;
- both streaming reads and archive writes are generated for sync and async I/O.

Dedicated async archive write/read recovery and malformed-header tests now
qualify the runtime surface in addition to the combined-mode compile checks.

### Hybrid CD writer and umbrella facade

The final optical leaf and the existing umbrella now have explicit contracts:

- `hadris-cd` no longer advertises an async writer that delegated to sync-only
  ISO/UDF writers and never compiled; it is an explicitly sync-only capability;
- `hadris-cd` defaults select `std` and `sync` separately, and no-feature or
  std-only configurations compile without exposing a writer;
- umbrella optional dependencies disable leaf defaults, preventing a selected
  format from silently choosing an I/O mode;
- umbrella `std`, `alloc`, `read`, `write`, `sync`, and `async` features forward
  independently to whichever leaves are enabled;
- the umbrella default preserves the hosted synchronous read/write experience
  with FAT, ISO, and CPIO;
- `cd` deliberately enables its sync-only writer. Consequently the full
  `optical` bundle contains that sync API even in a configuration that also asks
  for async ISO/UDF APIs.

Facade CI covers no-format, no-std async block, hosted async optical, dual-mode
archive, and all-capability configurations.

### Category facades

The standalone `hadris-block`, `hadris-optical`, and `hadris-archive` crates now
apply the same contract:

- optional leaf dependencies disable their defaults;
- platform, I/O-mode, and capability features forward only to selected leaves;
- each concrete leaf is re-exported without wrapping or erasing its API;
- `hadris-block` additionally exposes the format-neutral `hadris-storage`
  primitives and forwards I/O modes to its optional detection API;
- `hadris-optical` exposes an optional multi-format detector that compiles with
  either I/O mode and does not require enabling the ISO or UDF leaf itself;
- the top-level `hadris` crate delegates to these facades instead of maintaining
  a second leaf-dependency and forwarding layer;
- the optical facade documents and preserves the CD writer's sync-only exception.

CI checks empty, async-only, dual-mode, default, and all-capability category
configurations, as well as the umbrella's individual and aggregate routes.

## Ecosystem research

Hadris is not the only Rust project to offer synchronous and asynchronous APIs,
but the implementations use several materially different models:

- [`reqwest`](https://docs.rs/reqwest/latest/reqwest/blocking/) and
  [`zbus`](https://docs.rs/zbus/latest/zbus/blocking/) have async cores and
  optional blocking wrappers. This is a good ergonomic fit for network clients,
  but their own documentation warns about blocking wrappers inside async runtimes.
  Hadris must not conceal an executor or spawn threads merely to provide its sync
  filesystem API.
- [`embedded-storage`](https://docs.rs/embedded-storage/latest/embedded_storage/)
  publishes synchronous and asynchronous storage traits as
  closely related crates. Its separation supports small `no_std` dependency
  graphs, but would make a single Hadris format supporting both modes harder to
  discover and configure.
- [`async-compression`](https://docs.rs/async-compression/latest/async_compression/)
  shares mode-neutral compression codecs and places runtime I/O adapters in
  feature-gated modules. This is the closest architectural analogue to the desired
  Hadris layering: share parsing/format algorithms, isolate I/O adapters.
- [`maybe-async`](https://docs.rs/maybe-async/latest/maybe_async/) demonstrates the
  same source-transformation technique as Hadris,
  including explicit sync-only and async-only regions. Its feature model selects
  one generated mode at a time, whereas Hadris intentionally supports both modes
  in one build.

The chosen Hadris model remains: compile both explicit API modules when requested,
share mode-neutral algorithms and wire types where practical, transform only the
thin I/O-dependent implementation, and never let a root compatibility re-export
determine internal type identity.

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
