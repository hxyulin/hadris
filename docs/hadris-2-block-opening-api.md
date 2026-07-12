# Hadris 2.0 Block Detection and Opening API

Status: implemented initial FAT slice

This document specifies the first category-level opening API for `hadris-block`.
It intentionally starts with FAT volumes while leaving room for additional block
filesystems. Partition tables are modeled as disk layouts, not filesystems.

## Decisions

- Detection and opening remain separate operations. A convenience `open` may
  perform both, but the detected format is always representable independently.
- The first unified volume handle borrows its source. Existing leaf openers
  consume their source and cannot return it on validation failure; borrowing
  guarantees that callers retain their device after every error.
- A partition table is not an `OpenVolume` variant. It describes bounded regions
  in which volumes may be detected and opened.
- The common wrapper is a concrete non-exhaustive enum, not a trait object. This
  works without allocation, exposes supported formats at compile time, and
  provides lossless access to concrete handles.
- The initial wrapper does not define a common filesystem trait. Common
  operations will be added only after FAT and another filesystem demonstrate
  compatible semantics.
- Sync and async modules expose the same type and operation names. Mode-neutral
  format and layout types live in `hadris_block::detect`.
- A mismatch between a supplied detection result and the actual media is an open
  error from the concrete format. The category layer does not silently probe a
  second format.

## Mode-neutral detection types

The existing detection vocabulary remains the source of truth:

```rust
#[non_exhaustive]
pub enum BlockFormat {
    Fat(FatVariant),
    PartitionTable(PartitionTableKind),
}

#[non_exhaustive]
pub enum FatVariant { Fat12, Fat16, Fat32, ExFat }

#[non_exhaustive]
pub enum PartitionTableKind { Mbr, Gpt, Hybrid }
```

`None` means unknown. I/O failure and truncated metadata remain errors rather than
being collapsed into unknown.

## Unified volume handle

The sync API will be introduced under `hadris_block::sync`:

```rust
#[non_exhaustive]
pub enum OpenVolume<'a, S> {
    Fat(hadris_fat::sync::FatFs<hadris_io::sync::Borrowed<'a, S>>),
}

impl<'a, S> OpenVolume<'a, S>
where
    S: hadris_io::sync::Read
        + hadris_io::sync::Seek<Error = <S as hadris_io::sync::Read>::Error>,
{
    pub fn open(source: &'a mut S, logical_block_size: u32)
        -> Result<Self>;

    pub fn open_detected(source: &'a mut S, format: FatVariant)
        -> Result<Self>;

    pub const fn format(&self) -> FatVariant;
    pub fn as_fat(&self)
        -> Option<&hadris_fat::sync::FatFs<hadris_io::sync::Borrowed<'a, S>>>;
    pub fn as_fat_mut(&mut self)
        -> Option<&mut hadris_fat::sync::FatFs<hadris_io::sync::Borrowed<'a, S>>>;
    pub fn into_fat(self)
        -> core::result::Result<
            hadris_fat::sync::FatFs<hadris_io::sync::Borrowed<'a, S>>,
            Self,
        >;
    pub fn into_inner(self) -> &'a mut S;
}
```

The async module has the same names and shapes, using
`hadris_io::async::{Read, Seek}` and `hadris_fat::async::FatFs`. Its `open` and
`open_detected` functions are async.

The explicit `Borrowed` adapter is required by `hadris-io` to avoid trait
coherence conflicts between standard and embedded I/O implementations. It is an
implementation detail of the concrete escape hatch; `into_inner` still returns
the caller's original `&mut S`.

`open` detects the source, rejects unknown and partitioned disks, then delegates
to `open_detected`. `open_detected` accepts only a filesystem format, preventing
callers from accidentally treating a partition table as a volume.

The enum is `#[non_exhaustive]` from its introduction so adding another
filesystem does not break callers. Named accessors provide convenient concrete
access without requiring exhaustive matching.

## Errors

`hadris-block` will expose a category error rather than erase the reason for a
failed operation:

```rust
#[non_exhaustive]
pub enum Error {
    Io(hadris_io::Error),
    UnknownFormat,
    PartitionedDisk(PartitionTableKind),
    UnsupportedFormat(BlockFormat),
    DetectedFormatMismatch {
        detected: FatVariant,
        opened: FatVariant,
    },
    Fat(hadris_fat::FatError),
}

pub type Result<T> = core::result::Result<T, Error>;
```

The exact I/O variant will follow the error capabilities available from
`hadris-io`; generic source errors should be preserved where practical. exFAT
detection is always available, but opening returns `UnsupportedFormat` unless
the facade's future `exfat` capability is enabled and the implementation is
ready for category-level use.

The mismatch check compares the BPB-derived variant reported during detection
with `FatFs::fat_type()` after full validation. A mismatch is reported rather
than silently trusting either layer.

## Whole-volume workflow

```rust,no_run
use hadris_block::sync::OpenVolume;

let mut file = std::fs::File::open("volume.img")?;
let mut volume = OpenVolume::open(&mut file, 512)?;

if let Some(fat) = volume.as_fat_mut() {
    for entry in fat.root_dir().entries() {
        // Use the complete concrete FAT API.
    }
}

drop(volume); // releases the borrow
// `file` remains owned by the caller, including after an open error.
# Ok::<(), Box<dyn std::error::Error>>(())
```

Callers that already detected the source avoid duplicate probing:

```rust,no_run
use hadris_block::detect::{BlockFormat, sync::detect};
use hadris_block::sync::OpenVolume;

let mut file = std::fs::File::open("volume.img")?;
match detect(&mut file, 512)? {
    Some(BlockFormat::Fat(kind)) => {
        let volume = OpenVolume::open_detected(&mut file, kind)?;
        // use `volume`
    }
    Some(BlockFormat::PartitionTable(kind)) => {
        println!("partitioned disk: {kind:?}");
    }
    None => println!("unknown block format"),
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Partitioned-disk workflow

Partition support is a second layer built from three distinct types:

```text
source device
    -> detected partition-table kind
    -> parsed concrete partition table
    -> bounded partition view
    -> detect/open volume within that view
```

The prerequisite storage API is a bounded byte-stream adapter:

```rust
pub struct PartitionView<'a, S> {
    source: &'a mut S,
    byte_offset: u64,
    byte_len: u64,
    position: u64,
}

impl<'a, S> PartitionView<'a, S> {
    pub fn new(source: &'a mut S, byte_offset: u64, byte_len: u64)
        -> hadris_storage::Result<Self>;
    pub const fn len(&self) -> u64;
    pub const fn is_empty(&self) -> bool;
    pub const fn position(&self) -> u64;
    pub fn into_inner(self) -> &'a mut S;
}
```

It implements the appropriate Hadris `Read` and `Seek` traits, translates
partition-relative offsets with checked arithmetic, and rejects access outside
the partition. A block-addressed counterpart may follow, but the byte-stream
adapter is required first because current FAT openers consume `Read + Seek`.

Partition table parsing continues to return the concrete `hadris-part` types.
The category crate will provide helpers that convert a selected MBR or GPT entry
into a checked `PartitionView`; it will not introduce a lossy universal
partition-entry representation yet.

```rust,ignore
let table = hadris_block::part::sync::read_partition_table(&mut disk, 512)?;
let mut partition = table.view(&mut disk, partition_index)?;
let volume = hadris_block::sync::OpenVolume::open(&mut partition, 512)?;
```

Only one mutable partition view may exist for a source at a time. This follows
Rust borrowing, needs no reference counting or allocation, and prevents
overlapping mutable access. Concurrent or shared device access belongs in an
explicit user-provided synchronization adapter.

## Deferred common volume API

`OpenVolume` initially offers only format inspection, concrete accessors, and
source recovery. The following names are reserved for evaluation after a second
filesystem is integrated:

- `Volume` for mode-neutral metadata and root-directory access;
- `Directory`, `Entry`, and `File` capability traits;
- `ReadVolume` and `MutableVolume` only if trait separation proves necessary;
- a dynamic or enum-backed `AnyVolume` only if callers need runtime storage of
  heterogeneous open volumes.

We will not normalize FAT paths, timestamps, mutation, or iteration through the
wrapper until those semantics can be compared with another block filesystem.

## Implementation sequence

1. [x] Add `PartitionView` to `hadris-storage`, with checked sync/async I/O.
2. [x] Add `FatFs::into_inner` in both generated modes.
3. [x] Add the `hadris-block` category `Error` and `Result` types.
4. [x] Implement sync `OpenVolume` for FAT and test source recovery,
   detection/open mismatch, and concrete escape hatches.
5. [x] Implement the structurally equivalent async API.
6. [x] Add helpers from concrete MBR/GPT entries to `PartitionView`.
7. [x] Add integration coverage for FAT inside MBR and GPT partition views.
8. [x] Add a dedicated runtime async adapter test harness covering bounded
   partition reads, detection position restoration, opening, mismatch errors,
   and source recovery.
9. [ ] Revisit common volume capabilities only after another block filesystem is
   available.
