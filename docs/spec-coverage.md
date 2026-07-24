# Spec coverage

Maintainer audit index for standards-facing types in Hadris.
Not a public marketing matrix.

**CI:** `python3 scripts/check-spec-annotations.py` (tag grammar + every `@hadris-spec` id must appear below).

## Annotation convention

Place one annotation block on each standards-facing on-disk type or public
parse/format entry point:

```rust
/// @hadris-spec ECMA-167:3/10.5
/// @hadris-compliance full
/// @hadris-tests comprehensive_udf::partition_descriptor
/// @hadris-fuzz udf_read
```

- `@hadris-spec` is required and uses one stable `DOCUMENT:section` identifier.
- `@hadris-compliance` is required and accepts `full`, `partial`, `none`, or
  `n/a`.
- `full` requires at least one runnable `@hadris-tests` function. Fuzzing is
  supplementary robustness evidence, not proof of conformance.
- `partial` requires an `@hadris-note` describing the gap.
- `@hadris-tests` names runnable test functions; `@hadris-fuzz` names a target
  under `fuzz/`. CI verifies that cited functions and targets exist. Fuzz
  targets are local discovery tools, not CI jobs.
- Annotate spec-facing layouts and entry points, not private helpers or every
  call site.

**How to update**

1. `rg '@hadris-spec' crates/`
2. Sync rows below (one primary row per annotated item).
3. Prefer `partial` + Notes over claiming `full`.
4. Re-run `python3 scripts/check-spec-annotations.py`.

Fuzz columns name targets under `fuzz/` (local only — not PR CI).

## hadris-ntfs

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| NTFS:Boot-Sector | `RawNtfsBootSector` | partial | `compliance::open_rejects_invalid_sector_size` | | Core geometry and locations are validated; reserved fields, checksum, and backup-boot recovery are not. |
| NTFS:Update-Sequence-Array | `apply_fixups` | full | `compliance::fixups_restore_each_sector_trailer` | | FILE and INDX sector trailers validated and restored |
| NTFS:Attribute-Record | `AttrIter` | partial | `compliance::attributes_are_bounded_by_the_file_record_used_size` | | Resident and non-resident headers are validated; attribute-list extension records are not resolved. |
| NTFS:Mapping-Pairs | `DataRunDecoder` | full | `compliance::data_runs_decode_relative_and_sparse_extents` | | Signed relative LCNs, sparse runs, termination, and malformed encodings covered |
| NTFS:File-Name | `parse_file_name` | partial | `compliance::filenames_decode_utf16_surrogate_pairs` | | Parses references, sizes, flags, namespace, and full UTF-16 names; timestamps and reparse/EA data are not exposed. |
| NTFS:Index-Entry | `parse_index_entries` | partial | `read::large_directory_uses_index_allocation` | | Enumerates filename-index entries but does not expose child-node VCN pointers for keyed B-tree descent. |
| NTFS:Master-File-Table | `NtfsFs::open` | partial | `read::open_blank_volume` | | Reads the base `$MFT` extent and validates file references; attribute-list extents and `$MFTMirr` recovery are not supported. |
| NTFS:Directory-Index | `NtfsDir::entries` | partial | `read::large_directory_uses_index_allocation` | | Honors `$BITMAP`, update sequences, namespaces, and `$UpCase`; attribute-list index extents are not resolved. |
| NTFS:Data-Stream | `FileReader` | partial | `read::read_large_nonresident_file` | | Reads resident, non-resident, sparse, and uninitialized unnamed data; compressed, encrypted, named, and attribute-list streams are unsupported. |

## hadris-udf

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-167:3/7.2 | `DescriptorTag` | full | `descriptor::tag::tests::validate_bytes_enforces_version_reserved_location_and_crc` | `udf_read` | Version, reserved byte, location, checksum, CRC length, and descriptor CRC fail closed |
| ECMA-167:3/7.2.1 | `TagIdentifier` | full | `comprehensive_udf::test_descriptor_tag_ids` | `udf_read` | |
| ECMA-167:3/7.1 | `ExtentDescriptor` | full | `comprehensive_udf::test_extent_descriptor` | `udf_read` | |
| ECMA-167:1/7.4 | `EntityIdentifier` | full | `comprehensive_udf::test_partition_contents` | `udf_read` | |
| ECMA-167:1/7.2.1 | `CharSpec` | full | `write::cs0_tests::selects_eight_bit_for_latin1`, `write::cs0_tests::selects_sixteen_bit_for_wide_unicode` | `udf_read` | |
| ECMA-167:4/14.14.2 | `LongAllocationDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |
| ECMA-167:4/14.14.1 | `ShortAllocationDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |
| ECMA-167:3/10.2 | `AnchorVolumeDescriptorPointer` | full | `integration_external::write_tests::test_hadris_udf_has_valid_avdp` | `udf_read` | |
| ECMA-167:3/10.1 | `PrimaryVolumeDescriptor` | full | `write::tests::test_roundtrip_basic_verification` | `udf_read` | |
| ECMA-167:3/10.5 | `PartitionDescriptor` | full | `descriptor::partition::tests::partition_descriptor_layout_and_validate` | `udf_read` | Vertical-slice unit test |
| ECMA-167:3/10.6 | `LogicalVolumeDescriptor` | full | `write::tests::test_roundtrip_basic_verification` | `udf_read` | |
| ECMA-167:3/10.7.2 | `Type1PartitionMap` | full | `descriptor::logical::tests::type1_partition_maps_parses_embedded_table` | `udf_read` | |
| ECMA-167:4/14.1 | `FileSetDescriptor` | full | `write::tests::test_roundtrip_basic_verification` | `udf_read` | |

## hadris-iso

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-119:8.2 | `BootRecordVolumeDescriptor` | full | `xorriso_boot::test_hadris_multisection_boot_catalog` | `iso_read` | Locates the El Torito boot catalog |
| ECMA-119:8.3 | `VolumeDescriptorSetTerminator` | full | `comprehensive_iso::test_volume_descriptor_set_terminator` | `iso_read` | |
| ECMA-119:8.4 | `PrimaryVolumeDescriptor` | full | `comprehensive_iso::test_pvd_standard_identifier` | `iso_read` | |
| ECMA-119:8.5 | `SupplementaryVolumeDescriptor` | partial | | `iso_read` | Joliet SVD is read/written (UCS-2, BMP only); the version-2 "enhanced" form is repurposed as a UDF-bridge signal rather than a conformant ISO 9660:1999 secondary descriptor. |
| ECMA-119:9.1 | `DirectoryRecordHeader` | full | `directory::tests::directory_record_parse_roundtrip` | `iso_read` | Fixed fields; covered by parse roundtrip |
| ECMA-119:9.1 | `DirectoryRecord` | partial | `directory::tests::directory_record_parse_roundtrip` | `iso_read` | Joliet+RRIP coexistence on read may hide one namespace; see crate Known Limitations |
| ECMA-119:9.4 | `PathTableEntryHeader` | partial | | `iso_read` | Both L- and M-type path tables are written and read; the optional secondary path tables are not populated. |
| El-Torito:validation | `BootValidationEntry` | full | `xorriso_boot::test_eltorito_boot_catalog_comparison` | `iso_read` | |
| El-Torito:section-header | `BootSectionHeaderEntry` | full | `xorriso_boot::test_hadris_multisection_boot_catalog` | `iso_read` | |
| El-Torito:section-entry | `BootSectionEntry` | full | `xorriso_boot::test_floppy_emulation_media_type_and_default_load_size` | `iso_read` | Named floppy/HDD emulation media types |

## hadris-fat

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| FAT:BPB | `RawBpb` | full | `comprehensive_fat::test_valid_sector_sizes` | `fat_read` | |
| FAT:FSInfo | `RawFsInfo` | full | `comprehensive_fat::test_fsinfo_free_cluster_unknown` | `fat_read` | FAT32 free-cluster/next-free tracking |
| FAT:LFN | `RawLfnEntry` | partial | `comprehensive_fat::test_lfn_builder_sequence`, `test_write::maximum_length_name_spans_clusters`, `test_write::long_name_exceeding_one_cluster_roundtrips_and_deletes` | `fat_read` | This raw on-disk structure is complete, while semantic validation and legacy ANSI fallback behavior are implemented by higher-level LFN readers and writers. |
| FAT:DirEntry | `RawFileEntry` | partial | `test_write::test_lowercase_short_name_uses_nt_case_flags` | `fat_read` | Name/attributes/timestamps/cluster/size and NT case flags (`DIR_NTRes`) are read and written; extended access-time granularity is not modeled. |

## hadris-part

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| MBR:layout | `MasterBootRecord` | full | `roundtrip::mbr_write_read_roundtrip` | | 512-byte MBR incl. protective/hybrid MBR support |
| UEFI:GPT-Header | `GptHeader` | full | `io_roundtrip::gpt_scheme_sync_write_open_and_detect_roundtrip` | | Primary/backup header validation |
| UEFI:GPT-Entry | `GptPartitionEntry` | full | `roundtrip::gpt_partition_entry_roundtrip` | | |
