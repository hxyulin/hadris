# Hadris 2.0 API Conventions

Status: working specification and audit baseline

## Public API shape

Every format should distinguish four layers:

1. `raw`: byte-layout-compatible structures and constants;
2. validated format values and metadata;
3. mode-specific I/O under `sync` and `async`;
4. optional high-level builders, mutation tools, and diagnostics.

Raw types must not perform hidden I/O. High-level APIs must not require callers to
construct raw descriptors for ordinary operations.

## Naming and lifecycle

- `detect(source)` performs a cheap, non-destructive format probe.
- `open(source)` opens and validates an existing structure.
- `create(target, options)` creates a new logical object without implying a full
  device format unless that is the format's natural operation.
- `format(target, options)` initializes a complete filesystem or image.
- `read_*` and `write_*` perform explicit data transfer.
- `finish(self)` finalizes buffered metadata and returns the underlying target.
- `flush(&mut self)` makes pending writes visible without consuming the handle.

Constructors use `new` only when they cannot fail and do not perform I/O.

## Options and builders

- Use `<Operation>Options` for configuration values and implement `Default` when
  there is a safe, unsurprising default.
- Use `<Type>Builder` only when construction is incremental or validates several
  dependent choices.
- Fluent setters use the field name (`volume_id`, `block_size`), not a mixture of
  `with_*` and bare names.
- Validate options before the first write whenever possible.

## Handles and ownership

- Primary handles use format names such as `FatVolume`, `IsoImage`, `UdfVolume`,
  `CpioReader`, and `PartitionTable`.
- Read-only and writable behavior are expressed through capabilities and trait
  bounds, not duplicated type names unless their state machines differ.
- Owning handles provide `into_inner`; borrowed access uses `get_ref` and
  `get_mut` where safe.
- Operations that can leave metadata incomplete must require `finish` and clearly
  define drop behavior.

## Entries, paths, and traversal

- `Entry` is immutable metadata about one directory or archive member.
- `File` and `Directory` are operational handles, not metadata records.
- Common accessors are `name`, `kind`, `len`, `is_empty`, `metadata`, and
  `timestamps`.
- Use `u64` for byte lengths and offsets in high-level APIs.
- Iterators yield `Result<Entry, Error>` when traversal performs I/O or parsing.
- Provide `entries()` for synchronous iteration and an equivalent explicit async
  cursor/stream API; do not give methods different semantics solely by mode.
- Paths are relative within an image/archive unless an API explicitly accepts a
  host path. Image paths and host filesystem paths use distinct types or clearly
  distinct parameter names.

## Errors

- Each public crate exposes `Error` and `Result<T>` at its root; format-qualified
  aliases may remain only when ambiguity requires them.
- Preserve the originating I/O error when generic bounds permit it.
- Separate malformed/corrupt input, unsupported-but-valid features, invalid user
  options, out-of-bounds access, and underlying I/O failures.
- Validation errors include the relevant offset, block, field, or descriptor when
  available.
- Panics are reserved for violated internal invariants, not malformed media.

## Sync and async

- Mode-specific I/O lives under `crate::sync` and `crate::async`.
- Equivalent operations keep the same type and method names in both modes.
- Mode-neutral types live at the crate root or under `raw`/`types` and compile
  once.
- Root sync re-exports may exist for migration ergonomics but internal code must
  use explicit module paths.
- An API is not advertised as async if its body performs blocking host I/O.

## Initial audit findings

- FAT uses `FatFs`, while UDF uses `UdfFs` and ISO uses `IsoImage`; the `Fs`
  abbreviation and volume/image vocabulary need normalization.
- Opening is relatively consistent, but creation varies among `format_new`,
  formatter objects, writer constructors, and `write` methods.
- FAT directory iteration mixes `entries`, `next_entry`, synchronous `Iterator`,
  and mode-specific behavior in the same source.
- ISO exposes many raw structures at its root alongside high-level APIs, making
  the intended entry points difficult to identify.
- UDF shared entry types depend on mode-specific descriptor identities.
- CPIO's sequential `Reader`/`Writer` model is appropriately different from a
  filesystem and should not be forced into volume traits.
- CD and CPIO builders mix `with_*` and field-name fluent setters.
- Error aliases vary (`Result`, `FatError`, `UdfResult`, `CdResult`,
  `IsoModifyResult`) and several crates erase underlying I/O errors.
- Byte lengths use `usize`, `u32`, and `u64` inconsistently at high-level
  boundaries.

## Migration policy

The detailed audit will maintain a rename/removal table before implementation.
For 2.0, prefer direct replacement over long-lived duplicate APIs. Temporary
deprecated aliases are appropriate only for common, mechanically migratable
names and should identify the exact replacement in their diagnostic message.
