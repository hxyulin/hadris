# Hadris 2.0 Roadmap

Status: active design and implementation roadmap

Hadris 2.0 is the opportunity to make the workspace coherent as it grows from a
collection of format crates into a storage toolkit. The release should establish
consistent APIs, dependable feature combinations, reusable storage primitives,
and category-level entry points without hiding format-specific capabilities.

## Design principles

- Group formats by access model: block storage, optical media, and sequential
  archives.
- Keep foundational interfaces below format crates and category facades above
  them, avoiding dependency cycles.
- Share abstractions only when at least two implementations have the same
  semantics; similar terminology alone is not enough.
- Prefer small capability-oriented interfaces over one universal filesystem
  trait.
- Keep raw on-disk structures available, but separate them clearly from safe,
  high-level operations.
- Treat `no_std`, allocation, synchronous I/O, and asynchronous I/O as explicit,
  tested API tiers.
- Use the major release to remove or rename poor APIs. Add migration aliases only
  when they materially help users.

## Phase 1: Reliable feature model

Make every advertised feature tier compile independently and define which
combinations are supported.

- Repair ISO and UDF generation when synchronous and asynchronous APIs are
  enabled together.
- Make `cargo check --all-features` meaningful for every library and the
  workspace.
- Verify default, no-default, `std`, `alloc`, `sync`, `async`, `read`, and `write`
  tiers where applicable.
- Encode the supported matrix in CI so a new facade cannot silently combine
  incompatible dependencies.
- Keep feature forwarding consistent through category and umbrella crates.

Completion means the documented matrix is enforced by CI with warnings denied.

## Phase 2: Hadris 2.0 API conventions

Inventory the public surface of FAT, partitioning, ISO, UDF, CPIO, CD composition,
I/O, and common utilities. Define and then apply shared conventions for:

- lifecycle verbs: `detect`, `open`, `create`, and `format`;
- handle names and ownership of underlying readers, writers, and devices;
- options, builders, defaults, validation, and finish/flush behavior;
- file, directory, archive-entry, path, timestamp, and metadata vocabulary;
- iteration, lookup, traversal, extraction, and mutation;
- error categories, source errors, corruption reporting, and unsupported features;
- synchronous and asynchronous module organization;
- raw disk representations versus validated high-level values;
- read-only and writable capability boundaries.

The audit will produce an explicit rename/removal/migration table. Public examples
and compile tests become acceptance tests for the conventions.

Status: the cross-crate audit and migration table is active in
[`hadris-2-api-audit.md`](hadris-2-api-audit.md). FAT, partitioning, optical
opening/writing, ISO/UDF creation and modification, and CPIO writer completion
now follow the canonical ownership and lifecycle conventions. Remaining audit
work focuses on traversal vocabulary, raw-layout organization, and final error
taxonomy before the API freeze.

## Phase 3: Foundational storage interfaces

Add a core crate tentatively named `hadris-storage`. It sits above `hadris-io`
and below block-oriented format crates.

Initial responsibilities:

- logical block geometry and checked block/byte conversions;
- read-only and writable block-device capabilities;
- aligned block reads and writes with explicit buffer-size validation;
- bounded device/partition views with overflow-safe addressing;
- adapters for seekable byte streams and in-memory storage;
- consistent device errors that preserve the underlying I/O error;
- optional caching or instrumentation adapters only after the base interface is
  stable.

The abstraction must not assume 512-byte sectors. Format-specific concepts such
as FAT clusters, ISO logical sectors, and partition-table policy remain in their
own crates unless multiple consumers prove a shared semantic model.

## Phase 4: Category facade crates

Thin facades now establish the category boundaries before unified behavior is
added:

- `hadris-block`: storage primitives, partitioning, FAT, and future block
  filesystems;
- `hadris-optical`: ISO, UDF, hybrid disc composition, and future optical formats;
- `hadris-archive`: CPIO, future TAR formats, and sequential archive utilities.

Facades may later add detection and common read-only wrappers. They must preserve
access to the concrete format type so specialized features are not erased.

Status: the three facades exist and the `hadris` umbrella delegates its feature
forwarding and public category namespaces to them. `hadris-block` also exposes
the format-neutral `hadris-storage` primitives and lightweight, non-consuming
FAT/partition-table detection. Common cross-format capability traits remain
future work and should follow a second format in each relevant category.

Optical multi-format detection is implemented and specified in
[`hadris-2-optical-detection.md`](hadris-2-optical-detection.md). It reports ISO,
UDF, or both for bridge images rather than forcing a single classification.
Policy-driven unified ISO/UDF opening is also implemented in both I/O modes;
see [`hadris-2-optical-opening.md`](hadris-2-optical-opening.md).

The former bridge layout collision is fixed: `CdWriter` output is continuously
opened through both ISO and UDF readers, in addition to lightweight detection.

The accepted block opening and partition traversal design lives in
[`hadris-2-block-opening-api.md`](hadris-2-block-opening-api.md). It chooses a
borrowed `OpenVolume` initially so failed opens never take ownership away from the
caller, and requires a checked bounded `PartitionView` before opening filesystems
inside partitioned disks.

Status: the initial slice is implemented for FAT in both API modes. Checked
partition views, MBR/GPT entry adapters, concrete escape hatches, source
recovery, and nested-volume integration tests are in place. Runtime async tests
now cover bounded reads, detection, unified opening, mismatches, and recovery. A
second filesystem remains the next evidence needed before defining common volume
capabilities.

## Phase 5: Capability-oriented high-level APIs

Explore narrow interfaces such as `Volume`, `Directory`, `File`, `Entry`,
`ReadDirectory`, `MutableVolume`, `ArchiveReader`, `ArchiveWriter`, and
`FormatDetector`. Avoid requiring every format to implement operations it cannot
support naturally.

Dynamic wrappers such as `DetectedVolume` or `AnyFilesystem` should expose only a
well-defined common subset and provide an escape hatch to concrete formats.

## Phase 6: Migration and release quality

- Publish a Hadris 1.x to 2.0 migration guide.
- Maintain API snapshots to detect unintended public changes.
- Compile every documented example in CI.
- Test external-tool interoperability and malformed-input behavior.
- Continuously qualify malformed block, optical, partition-table, and CPIO
  inputs in both I/O modes, including non-destructive detection contracts.
- Document stability levels for experimental formats and APIs.
- Require a clean default workspace build, the supported feature matrix, and
  category-facade integration tests before the 2.0 release.

## Current execution order

1. Fix and lock down the feature matrix.
2. Complete the public-API inventory and 2.0 conventions.
3. Integrate the initial `hadris-storage` geometry, capability, and stream-adapter
   slice into block-format experiments.
4. Normalize existing format APIs against the conventions.
5. Design category-level detection without erasing concrete format access.
6. Add unified capability wrappers where at least two implementations have
   proven compatible semantics.

Update this document when a design decision changes the sequence or scope. More
detailed specifications may live beside it, but this remains the project-level
source of truth for the 2.0 effort.
