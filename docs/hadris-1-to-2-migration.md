# Migrating from Hadris 1.x to 2.0

Hadris 2.0 reorganizes the project into a storage ecosystem with consistent
feature flags, explicit synchronous and asynchronous namespaces, category-level
detection and opening, and recoverable writer lifecycles. This guide targets
`2.0.0-rc.1`.

Most common FAT and UDF renames have deprecated aliases, but new code should use
the canonical 2.0 names. The aliases remain available for migration, but are
not the preferred V2 surface.

## Choose the right package

Applications can depend at three levels:

- Format crates such as `hadris-fat`, `hadris-iso`, `hadris-udf`,
  `hadris-part`, and `hadris-cpio` expose the complete format-specific API.
- `hadris-block`, `hadris-optical`, and `hadris-archive` group related formats,
  detection, and opening without erasing the concrete leaf APIs.
- `hadris` is the umbrella crate. It re-exports the category facades through
  `hadris::block`, `hadris::optical`, and `hadris::archive`.

Two formerly shared utility families are now independent packages:

- Use `hadris-path` for allocation-free virtual path parsing. The old
  `hadris_common::path` surface remains as a deprecated forwarding layer.
- Use `hadris-fixed` for fixed-capacity bytes, UTF-8, and endian-aware UTF-16.
  `hadris_common::types::file::FixedFilename` and
  `hadris_common::str::utf16::FixedUtf16Str` are deprecated aliases.

Start with the narrowest package that meets the application. Moving to a
category facade or the umbrella later retains access to the same concrete
implementations.

```toml
[dependencies]
hadris-fat = "2.0.0-rc.1"

# Or select several categories through the umbrella:
hadris = { version = "2.0.0-rc.1", features = ["block", "optical"] }
```

## Update feature flags

Hadris 2.0 separates platform, I/O mode, and operation capabilities:

| Feature | Meaning |
|---|---|
| `std` | Standard-library integration; implies `alloc`, not `sync` |
| `alloc` | Allocation-backed APIs without requiring `std` |
| `sync` | APIs under `crate::sync` |
| `async` | APIs under `crate::async` |
| `read` | Parsing and read-only operations |
| `write` | Creation and mutation where the crate supports it |

Enabling `sync` and `async` together exposes both namespaces. Neither takes
precedence, and `std` no longer silently selects synchronous I/O. Default
features still provide each crate's documented ergonomic hosted configuration.

For explicit or `no_std` configurations, select every required dimension:

```toml
[dependencies]
hadris-fat = {
    version = "2.0.0-rc.1",
    default-features = false,
    features = ["alloc", "read", "write", "sync"]
}
```

Use explicit namespaces in new code:

```rust,ignore
let sync_volume = hadris_fat::sync::FatVolume::open(sync_source)?;
let async_volume = hadris_fat::r#async::FatVolume::open(async_source).await?;
```

Root-level synchronous re-exports remain in several leaf crates for 1.x
compatibility, but should not be used by code intended to compile with both
modes.

Writing is not uniformly available in both modes:

- FAT and CPIO support their implemented synchronous and asynchronous read/write
  APIs.
- ISO and UDF writing and modification remain synchronous.
- The ISO/UDF hybrid writer in `hadris-cd` is synchronous.
- Removed async writer surfaces that did not compile or performed blocking I/O
  have no 2.0 compatibility shim.

Features such as FAT `cache`, `tool`, and `unstable-exfat` also remain
synchronous.

## Adopt the canonical names

### FAT

| 1.x name | 2.0 name |
|---|---|
| `FatFs` | `FatVolume` |
| `FatFsBuilder` | `FatVolumeBuilder` |
| `FatFsReadExt` | `FatVolumeReadExt` |
| `FatFsWriteExt` | `FatVolumeWriteExt` |
| `FormatOptions` | `FatFormatOptions` |
| `FileEntry::size()` | `FileEntry::len()` and `is_empty()` |

The old names remain aliases during migration. `FatDir`, `DirectoryEntry`,
`FileReader`, and `FileWriter` retain their names.

Builder and formatting setters now use field-style names:

```rust,ignore
// 1.x
let options = FormatOptions::new(size)
    .with_label("MY VOLUME")
    .with_sector_size(sector_size)
    .with_fat_type(fat_type);

// 2.0
let options = FatFormatOptions::new(size)
    .volume_label("MY VOLUME")
    .sector_size(sector_size)
    .fat_type(fat_type);
```

The equivalent replacements also apply to `with_sectors_per_cluster`,
`with_fat_copies`, `with_media_type`, `with_hidden_sectors`, and
`with_volume_id`. `FatVolumeBuilder` replaces `with_time_provider`,
`with_oem_converter`, and `with_fat_cache` with `time_provider`,
`oem_converter`, and `fat_cache`.

### Partition tables

| 1.x name or pattern | 2.0 replacement |
|---|---|
| `DiskPartitionScheme` | `PartitionTable` |
| `DiskPartitionScheme::read_from` | `sync::partition_table::open` or async equivalent |
| `detect_scheme_from_mbr` for normal opening | `sync::partition_table::detect` or async equivalent |
| `size_bytes()` with an implicit 512-byte sector | `byte_len(logical_block_size)` |
| manual partition offset arithmetic | bounded `PartitionView` helpers from `hadris-block` |

The concrete `MasterBootRecord`, `GptHeader`, and partition-entry types remain
available. Use `checked_end_lba` when values originate from untrusted media.

### ISO 9660

`IsoImage` remains the primary reader. Existing raw descriptor exports are not
being moved for 2.0.

Use `IsoDir::read_entries` for collection-oriented traversal in either mode,
with `IsoDir::find` and `IsoImage::find_path` for lookup. Writing remains under
the synchronous API.

The canonical writer lifecycle returns the output:

```rust,ignore
// 1.x: output target was discarded.
IsoImageWriter::format_new(output, files, options)?;

// 2.0
let output = IsoImageWriter::create(output, files, options)?;
```

`InputTree` replaces legacy filesystem-source and input-file aliases. Writer
options use field-style names such as `bootstrap`, `rock_ridge`, `joliet`,
`extensions`, and `hybrid_boot`.

### UDF

`UdfVolume` is the canonical volume name; `UdfFs` remains a compatibility alias.
Use `hadris_udf::sync` or `hadris_udf::async` explicitly.

Formatting now returns both the target and sector count:

```rust,ignore
// 1.x
let sectors = UdfWriter::format(output, &root, options)?;

// 2.0
let created = UdfWriter::create(output, &root, options)?;
let output = created.target;
let sectors = created.sectors_written;
```

Modification uses consuming `finish` to recover the image target. The old
`commit` method remains as a deprecated forwarding API.

### CPIO and hybrid optical images

Use the owning `CpioArchiveWriter` and call `finish`, which returns the target in
both modes. CPIO remains a sequential archive API with `next_entry`; it is not
forced into a filesystem volume abstraction.

For ISO/UDF bridge images, `OpticalImageWriter` and `OpticalImageOptions` are the
canonical names for the existing `CdWriter` and `CdOptions` types. Use
field-style option setters and consuming `finish`; `CdWriter::write` remains
deprecated.

## Use the 2.0 lifecycle

The same verbs now communicate the same intent across formats:

- `detect(&mut source)` cheaply probes and restores the source position.
- `open(source)` validates an existing filesystem, image, or partition table.
- `create(target, options)` creates a logical object and returns its target.
- `format(target, options)` initializes a complete filesystem where that term is
  the natural operation.
- `finish(self)` commits buffered metadata and recovers the target.
- `into_inner(self)` recovers an owned source from an open handle where
  supported.

Do not omit `finish` from write state machines that require it. For example, a
FAT `FileWriter` commits its directory-entry size and timestamps during
`finish`.

## Use category detection and opening

Category facades provide non-destructive detection while retaining concrete
format handles. They intentionally do not impose a lowest-common-denominator
filesystem trait.

For whole block volumes:

```rust,ignore
use hadris_block::sync::OpenVolume;

let mut image = std::fs::File::open("volume.img")?;
let mut volume = OpenVolume::open(&mut image, 512)?;

if let Some(fat) = volume.as_fat_mut() {
    for entry in fat.root_dir().entries() {
        let entry = entry?;
        println!("{}", entry.name());
    }
}

drop(volume); // releases the borrowed source
```

Partition tables are layouts, not `OpenVolume` variants. Detect and open the
table, create a bounded partition view, then detect or open the filesystem
inside that view.

For optical images:

```rust,ignore
let image = hadris_optical::sync::OpenOpticalImage::open(
    &mut source,
    hadris_optical::OpenPolicy::PreferUdf,
)?;
```

Detection reports ISO 9660 and UDF independently. On bridge images,
`PreferUdf` falls back to ISO, `PreferIso9660` falls back to UDF, and
`Iso9660`/`Udf` require the requested namespace. Use `as_iso9660`, `as_udf`, or
`into_inner` to access the concrete handle or recover the source. The async
opener has the same shape under `hadris_optical::r#async`.

## Update error handling

Category facades expose root `Error` and `Result<T>` types. They distinguish
unknown input, unsupported recognized formats, partitioned disks, detection
mismatches, concrete format validation failures, and I/O failures.

Leaf crates retain descriptive format errors and compatibility aliases where
format context is useful. Code should match only the cases it can act on and
include a fallback for non-exhaustive category errors.

Detection is not validation. A detected FAT, ISO, UDF, MBR, or GPT structure can
still fail during `open`, and that concrete validation error is preserved.

## Treat exFAT as an unstable preview

Stable Hadris 2.0 supports FAT12, FAT16, and FAT32. exFAT is available only
through the leaf crate's opt-in `unstable-exfat` feature:

```toml
hadris-fat = {
    version = "2.0.0-rc.1",
    features = ["unstable-exfat"]
}
```

The preview is synchronous, allocation-backed, excluded from the stable public
API snapshot, and may change in a minor release. It is not re-exported by the
`hadris` umbrella and is not opened by `hadris-block::OpenVolume`.
Category-level detection can report `FatVariant::ExFat`, but unified opening
returns `UnsupportedFormat`.

## Update command names

The canonical 2.0 executable family is:

| Purpose | Canonical binary | Compatibility alias |
|---|---|---|
| FAT | `hadris-fat` | `fatutil` |
| ISO 9660 | `hadris-iso` | `hadris-iso-cli` |
| UDF | `hadris-udf` | `hadris-udf-cli` |
| CPIO | `hadris-cpio` | `cpioutil` |
| ISO/UDF bridge | `hadris-cd` | none |

Common operations use `info`, `ls`, `tree`, `cat`, `extract`, `create`, and
`verify` where meaningful for the format. CPIO keeps `list` as an alias for
`ls`. There is no supported umbrella `hadris` executable in 2.0; use the
specialized tools.

## Migration checklist

1. Change dependency requirements to `2.0.0-rc.1`.
2. Make platform, mode, and operation features explicit when disabling defaults.
3. Move I/O calls to `sync` or `async` namespaces.
4. Replace deprecated types and fluent setters with canonical names.
5. Recover created or modified targets through `create`, `finish`, or
   `into_inner`.
6. Replace manual probing and partition arithmetic with category detection,
   openers, and bounded views where appropriate.
7. Update error matches for category `Error` types and non-exhaustive enums.
8. Keep exFAT behind `unstable-exfat`, or remain on stable FAT12/16/32.
9. Replace legacy executable names with the canonical `hadris-*` family.
