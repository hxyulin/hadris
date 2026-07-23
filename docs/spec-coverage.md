# Spec coverage

Maintainer audit index for standards-facing types in Hadris.
Not a public marketing matrix.

**Design:** [`docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`](superpowers/specs/2026-07-09-spec-compliance-program-design.md)

**CI:** `python3 scripts/check-spec-annotations.py` (tag grammar + every `@hadris-spec` id must appear below).

**How to update**

1. `rg '@hadris-spec' crates/`
2. Sync rows below (one primary row per annotated item).
3. Prefer `partial` + Notes over claiming `full`.
4. Re-run `python3 scripts/check-spec-annotations.py`.

Fuzz columns name targets under `fuzz/` (local only — not PR CI).

## hadris-ntfs

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| NTFS:Boot-Sector | `RawNtfsBootSector` | partial | `compliance::open_rejects_invalid_sector_size` | | Core geometry and locations validated; reserved fields, checksum, and backup-boot recovery are not |
| NTFS:Update-Sequence-Array | `apply_fixups` | full | `compliance::fixups_restore_each_sector_trailer` | | FILE and INDX sector trailers validated and restored |
| NTFS:Attribute-Record | `AttrIter` | partial | `compliance::attributes_are_bounded_by_the_file_record_used_size` | | Resident/non-resident records validated; `$ATTRIBUTE_LIST` extension records unresolved |
| NTFS:Mapping-Pairs | `DataRunDecoder` | full | `compliance::data_runs_decode_relative_and_sparse_extents` | | Signed relative LCNs, sparse runs, termination, and malformed encodings covered |
| NTFS:File-Name | `parse_file_name` | partial | `compliance::filenames_decode_utf16_surrogate_pairs` | | Core fields and full UTF-16 names parsed; timestamps and reparse/EA data not exposed |
| NTFS:Index-Entry | `parse_index_entries` | partial | `read::large_directory_uses_index_allocation` | | Filename-index enumeration only; child VCNs are not exposed for keyed descent |
| NTFS:Master-File-Table | `NtfsFs::open` | partial | `read::open_blank_volume` | | Base `$MFT` extent and sequence checks; no attribute-list extents or `$MFTMirr` recovery |
| NTFS:Directory-Index | `NtfsDir::entries` | partial | `read::large_directory_uses_index_allocation` | | `$INDEX_ROOT`, active `$INDEX_ALLOCATION`, `$BITMAP`, namespaces, and `$UpCase`; no attribute-list extents |
| NTFS:Data-Stream | `FileReader` | partial | `read::read_large_nonresident_file` | | Resident/non-resident/sparse/uninitialized unnamed data; no compression, encryption, named streams, or attribute-list extents |

## hadris-udf

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-167:3/7.2 | `DescriptorTag` | full | `comprehensive_udf::test_tag_structure` | `udf_read` | |
| ECMA-167:3/7.2.1 | `TagIdentifier` | full | `comprehensive_udf::test_descriptor_tag_ids` | `udf_read` | |
| ECMA-167:3/7.1 | `ExtentDescriptor` | full | `comprehensive_udf::test_extent_descriptor` | `udf_read` | |
| ECMA-167:1/7.4 | `EntityIdentifier` | full | `comprehensive_udf::test_partition_contents` | `udf_read` | |
| ECMA-167:1/7.2.1 | `CharSpec` | full | | `udf_read` | |
| ECMA-167:4/14.14.2 | `LongAllocationDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |
| ECMA-167:4/14.14.1 | `ShortAllocationDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |
| ECMA-167:3/10.2 | `AnchorVolumeDescriptorPointer` | full | `comprehensive_udf::test_avdp_structure` | `udf_read` | |
| ECMA-167:3/10.1 | `PrimaryVolumeDescriptor` | full | | `udf_read` | |
| ECMA-167:3/10.5 | `PartitionDescriptor` | full | `descriptor::partition::tests::partition_descriptor_layout_and_validate` | `udf_read` | Vertical-slice unit test |
| ECMA-167:3/10.6 | `LogicalVolumeDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |
| ECMA-167:3/10.7.2 | `Type1PartitionMap` | full | `descriptor::logical::tests::type1_partition_maps_parses_embedded_table` | `udf_read` | |
| ECMA-167:4/14.1 | `FileSetDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |

## hadris-iso

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-119:8.2 | `BootRecordVolumeDescriptor` | full | `xorriso_boot::test_hadris_multisection_boot_catalog` | `iso_read` | Locates the El Torito boot catalog |
| ECMA-119:8.3 | `VolumeDescriptorSetTerminator` | full | `comprehensive_iso::test_pvd_standard_identifier` | `iso_read` | |
| ECMA-119:8.4 | `PrimaryVolumeDescriptor` | full | `comprehensive_iso::test_pvd_standard_identifier` | `iso_read` | |
| ECMA-119:8.5 | `SupplementaryVolumeDescriptor` | partial | | `iso_read` | Joliet SVD (UCS-2, BMP only); version-2 EVD repurposed as a UDF-bridge signal, not conformant ISO 9660:1999 |
| ECMA-119:9.1 | `DirectoryRecordHeader` | full | `directory::tests::directory_record_parse_roundtrip` | `iso_read` | Fixed fields; covered by parse roundtrip |
| ECMA-119:9.1 | `DirectoryRecord` | partial | `directory::tests::directory_record_parse_roundtrip` | `iso_read` | Joliet+RRIP coexistence on read may hide one namespace; records written in collation order |
| ECMA-119:9.4 | `PathTableEntryHeader` | partial | | `iso_read` | L- and M-type tables written/read; optional secondary path tables not populated |
| El-Torito:validation | `BootValidationEntry` | full | `xorriso_boot::test_eltorito_boot_catalog_comparison` | `iso_read` | |
| El-Torito:section-header | `BootSectionHeaderEntry` | full | `xorriso_boot::test_hadris_multisection_boot_catalog` | `iso_read` | |
| El-Torito:section-entry | `BootSectionEntry` | full | `xorriso_boot::test_floppy_emulation_media_type_and_default_load_size` | `iso_read` | Named floppy/HDD emulation media types |

## hadris-fat

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| FAT:BPB | `RawBpb` | full | `comprehensive_fat::test_valid_sector_sizes` | `fat_read` | |
| FAT:FSInfo | `RawFsInfo` | full | `comprehensive_fat::test_fsinfo_free_cluster_unknown` | `fat_read` | FAT32 free-cluster/next-free tracking |
| FAT:LFN | `RawLfnEntry` | partial | `comprehensive_fat::test_lfn_builder_sequence`, `test_write::lfn_cluster_boundary_tests` | `fat_read` | Raw layout and cross-cluster read/write are covered; semantic validation and legacy ANSI fallback behavior are handled above the raw structure |
| FAT:DirEntry | `RawFileEntry` | partial | `test_write::test_lowercase_short_name_uses_nt_case_flags` | `fat_read` | Short-name entry incl. NT `DIR_NTRes` case flags (lowercase 8.3 round-trip); extended access-time granularity not modeled |

## hadris-part

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| MBR:layout | `MasterBootRecord` | full | `roundtrip::mbr_write_read_roundtrip` | | 512-byte MBR incl. protective/hybrid MBR support |
| UEFI:GPT-Header | `GptHeader` | full | `io_roundtrip::gpt_scheme_sync_write_open_and_detect_roundtrip` | | Primary/backup header validation |
| UEFI:GPT-Entry | `GptPartitionEntry` | full | `roundtrip::gpt_partition_entry_roundtrip` | | |
