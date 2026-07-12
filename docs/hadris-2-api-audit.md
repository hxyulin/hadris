# Hadris 2.0 Public API Audit and Migration Table

Status: active migration specification

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
| Modes | root sync exports, incomplete async exports | matching `sync` and `async` module shapes | feature model complete; naming ongoing |
| Errors | format-specific aliases and erased I/O | root `Error`/`Result`, categorized corruption/unsupported/options/I/O | category migration ongoing |

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

`FatDir`, `DirectoryEntry`, `FileReader`, and `FileWriter` remain under review.
They will not be renamed until traversal and finish behavior are normalized in
both modes. Experimental exFAT stays outside the stable FAT volume surface for
2.0 until it meets the same validation and async contracts.

The block facade continuously runs the canonical async FAT detection/opening
workflow. Async mutation remains a separate qualification item.

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

## ISO 9660

| Current surface | 2.0 direction |
|---|---|
| `IsoImage` | retain as primary read handle |
| root raw descriptors and high-level handles mixed | move raw disk layouts under `raw` with targeted compatibility exports |
| writer constructors and modification APIs vary | converge on `create`/`finish` and explicit modification options |
| sync-only writers beside async readers | keep capability explicit; do not synthesize async writers |
| directory traversal naming differs from FAT/UDF | audit against `entries`, `find`, and operational file handles |

Category-level detection reports ISO independently from UDF. ISO and UDF now
provide recoverable ownership, and the optical facade implements policy-driven
unified opening with matching sync and async surfaces.

## UDF

| Current surface | 2.0 direction |
|---|---|
| `UdfFs` | canonical `UdfVolume` alias implemented; retain `UdfFs` for compatibility |
| mode-specific descriptor identities | continue separating mode-neutral raw values from I/O handles |
| sync-only formatting/modification | retain explicit sync capability until genuine async implementation |
| modification and writer errors | converge on root `Error` categories and operation context |

## CPIO

| Current surface | 2.0 direction |
|---|---|
| `CpioReader`/`CpioWriter` | retain sequential reader/writer model |
| entry/header mode duplication | move stable header/entry metadata to mode-neutral types where practical |
| writer completion | standardize on consuming `finish` returning the underlying target |
| traversal | keep sequential `next_entry`; do not force filesystem directory traits |

CPIO intentionally does not implement a volume abstraction. Shared archive
traits wait for TAR so that they are based on two real formats.

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
