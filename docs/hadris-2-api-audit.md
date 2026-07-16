# Hadris 2.0 Public API Audit and Migration Table

Status: accepted 2.0 baseline; additive feature work may continue before freeze

This is the authoritative inventory of high-level public API changes planned for
Hadris 2.0. Raw on-disk representations are audited separately when their
format modules are normalized; this table focuses on lifecycle, handles,
builders, traversal, ownership, and errors.

## Cross-crate conventions

| Area | 1.x pattern | 2.0 convention | Status |
|---|---|---|---|
| Probe | mixed detection and opening | `detect(&mut source)` is cheap and restores position | block and partition implemented |
| Existing object | `open`, `read_from`, constructors | `open` validates an existing logical object | block/FAT/partition implemented |
| New object | `new`, writer constructors | `create` creates; `format` initializes a complete volume/image | format-by-format migration |
| Ownership | inconsistent or unavailable | `get_ref`, `get_mut`, `into_inner` where representation permits | FAT, ISO, and UDF `into_inner` implemented |
| Builders | mixture of `with_*` and field names | fluent setters use field names | FAT implemented |
| Length | `size: usize/u32/u64` | `len() -> u64`, plus `is_empty` | FAT entries implemented |
| Modes | root sync exports, incomplete async exports | matching `sync` and `async` module shapes | feature model and baseline naming complete |
| Errors | format-specific aliases and erased I/O | category facades use root `Error`/`Result`; leaf crates retain descriptive errors where format context matters | baseline complete |

Lexical virtual-path parsing now lives in the independent `hadris-path` crate.
Its allocation-free `VPath`/`Components` core is shared by FAT, exFAT, ISO,
UDF, RRIP, and metadata-layout traversal. Format crates retain responsibility
for case folding, on-disk name decoding, symlinks, and entry lookup errors.
`hadris_common::path` remains a deprecated forwarding surface for migration.

Fixed-capacity storage now lives in the independent `hadris-fixed` crate.
`FixedBytes` represents arbitrary bytes, `FixedStr` preserves valid UTF-8, and
`FixedUtf16Le`/`FixedUtf16Be` make byte order explicit. FAT and ISO consume the
byte-oriented type directly. The former `hadris_common::types::file::FixedFilename`
and `hadris_common::str::utf16::FixedUtf16Str` names remain deprecated aliases.

## FAT

| Existing API | Canonical 2.0 API | Compatibility |
|---|---|---|
| `FatFs` | `FatVolume` | retained alias during migration |
| `FatFsBuilder` | `FatVolumeBuilder` | retained alias |
| `FatFsReadExt` | `FatVolumeReadExt` | retained alias |
| `FatFsWriteExt` | `FatVolumeWriteExt` | retained alias |
| `FormatOptions` | `FatFormatOptions` | retained alias |
| `with_time_provider` | `time_provider` | deprecated forwarding method |
| `with_oem_converter` | `oem_converter` | deprecated forwarding method |
| `with_fat_cache` | `fat_cache` | deprecated forwarding method |
| `with_label` | `volume_label` | deprecated forwarding method |
| `with_sector_size` | `sector_size` | deprecated forwarding method |
| `with_fat_type` | `fat_type` | deprecated forwarding method |
| remaining `with_*` format setters | matching field name | deprecated forwarding methods |
| `FileEntry::size() -> usize` | `len() -> u64`, `is_empty()` | deprecated forwarding method |

`FatDir`, `DirectoryEntry`, `FileReader`, and `FileWriter` retain their current
names for 2.0. They describe concrete FAT handles, have matching sync/async
shapes, and a rename would add migration cost without clarifying ownership or
lifecycle. The `unstable-exfat` leaf-crate preview stays outside the stable FAT
volume surface and API snapshot for 2.0 until it meets the same validation and
I/O-mode contracts.

The block facade continuously runs canonical async FAT detection, opening,
nested directory creation, multi-cluster content write/read, truncation,
traversal, duplicate rejection, and source-recovery workflows.

## Partition tables

| Existing API | Canonical 2.0 API | Compatibility |
|---|---|---|
| `DiskPartitionScheme` | `PartitionTable` | retained alias |
| `DiskPartitionScheme::read_from` | `sync::partition_table::open` / async equivalent | extension trait retained |
| `detect_scheme_from_mbr` | `sync::partition_table::detect` / async equivalent | low-level helper retained |
| implicit 512-byte `size_bytes` | `byte_len(logical_block_size)` | old method deprecated |
| unchecked `end_lba` | `checked_end_lba` for untrusted arithmetic | old method retained |
| manual partition offset arithmetic | `hadris-block` MBR/GPT `PartitionView` helpers | implemented |

`MasterBootRecord`, `GptHeader`, and partition entries remain concrete validated
format types. Raw disk-layout structs will not be hidden behind one lossy entry
enum.

The synchronous lifecycle is continuously qualified for MBR, primary/backup
GPT, and hybrid write-to-open roundtrips; non-destructive detection; CRC-backed
validation; and truncated or corrupt GPT rejection.

The asynchronous lifecycle is qualified independently for MBR, GPT, and hybrid
write-to-open roundtrips; non-destructive detection; truncated and corrupt
table rejection; and an end-to-end GPT partition view opened as a FAT volume.

## ISO 9660

| Current surface | 2.0 direction |
|---|---|
| `IsoImage` | retain as primary read handle |
| root raw descriptors and high-level handles mixed | retain 1.x exports for 2.0; any future `raw` move must begin as additive aliases and receive a separate compatibility review |
| writer constructors and modification APIs vary | canonical `create` returns the target; modifier `finish` returns the target; legacy methods deprecated |
| sync-only writers beside async readers | keep capability explicit; do not synthesize async writers |
| directory traversal naming differs from FAT/UDF | audit against `entries`, `find`, and operational file handles |

Category-level detection reports ISO independently from UDF. ISO and UDF now
provide recoverable ownership, and the optical facade implements policy-driven
unified opening with matching sync and async surfaces.

`IsoDir::read_entries` is the collection-oriented traversal operation shared by
both modes. `IsoDir::find` and `IsoImage::find_path` provide matching
collection-based lookup without requiring a synchronous iterator. Async facade
tests traverse nested ISO and UDF directories, read file contents, exercise both
bridge-image policies, and recover the source.

## UDF

| Current surface | 2.0 direction |
|---|---|
| `UdfFs` | canonical `UdfVolume` alias implemented; retain `UdfFs` for compatibility |
| mode-specific descriptor identities | continue separating mode-neutral raw values from I/O handles |
| sync-only formatting/modification | retain explicit sync capability until genuine async implementation |
| formatter/modifier lifecycle | `create` and modifier `finish` recover the target; legacy `format`/`commit` retained as deprecated forwarding APIs |
| modification and writer errors | operation modules expose canonical `Error`/`Result`; descriptive aliases retained |

## CPIO

| Current surface | 2.0 direction |
|---|---|
| stateless `CpioWriter` | canonical owning `CpioArchiveWriter`; legacy writer retained |
| entry/header mode duplication | move stable header/entry metadata to mode-neutral types where practical |
| writer completion | consuming `finish` returning the underlying target implemented in both modes |
| traversal | keep sequential `next_entry`; do not force filesystem directory traits |

CPIO intentionally does not implement a volume abstraction. Shared archive
traits wait for TAR so that they are based on two real formats.

Malformed-input qualification covers invalid CPIO magic and truncated headers
in both modes. At category boundaries, block and optical unknown inputs return
their facade error types and detection restores the caller's stream position;
recognized-but-invalid ISO and FAT/GPT inputs retain their format-specific
error variants for diagnosis.

## Hybrid optical composition

| Current surface | 2.0 direction |
|---|---|
| `CdWriter::write` and mixed `with_*` setters | canonical consuming `finish` plus field-style setters; deprecated forwarding APIs retained |
| “CD” naming for DVD/bridge images | canonical `OpticalImageWriter`/`OpticalImageOptions` aliases implemented; retain existing names for compatibility |
| sync-only ISO/UDF bridge construction | remain explicitly sync-only |
| bridge ISO/UDF validation | layout collision fixed; both concrete readers continuously tested |

## Migration policy

- Canonical names are used in all new documentation and examples.
- Deprecated forwarding methods identify their exact replacement.
- Type aliases remain temporarily when migration is mechanical and zero-cost.
- No deprecated alias is used by library internals.
- Aliases may be removed at the final 2.0 API freeze if the pre-release cycle
  provides sufficient migration time.
- Raw layout moves require explicit compatibility review because downstream code
  may use them for forensic and bootloader applications.
- This baseline is not a feature freeze. New capabilities may land before the
  final 2.0 release when they follow these conventions and extend the feature
  matrix, API snapshot, documentation examples, and integration coverage.
