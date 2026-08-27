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
- `@hadris-compliance` is required and accepts `full`, `partial`, `none`,
  `unknown`, or `n/a`.
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
| NTFS:Update-Sequence-Array | `apply_fixups` | unknown | `compliance::fixups_restore_each_sector_trailer` | | Behavior is tested, but authoritative source text was unavailable for this audit. |
| NTFS:Attribute-Record | `AttrIter` | partial | `compliance::attributes_are_bounded_by_the_file_record_used_size` | | Resident and non-resident headers are validated; attribute-list extension records are not resolved. |
| NTFS:Mapping-Pairs | `DataRunDecoder` | unknown | `compliance::data_runs_decode_relative_and_sparse_extents` | | Behavior is tested, but authoritative source text was unavailable for this audit. |
| NTFS:File-Name | `parse_file_name` | partial | `compliance::filenames_decode_utf16_surrogate_pairs` | | Parses references, sizes, flags, namespace, and full UTF-16 names; timestamps and reparse/EA data are not exposed. |
| NTFS:Index-Entry | `parse_index_entries` | partial | `read::large_directory_uses_index_allocation` | | Enumerates filename-index entries but does not expose child-node VCN pointers for keyed B-tree descent. |
| NTFS:Master-File-Table | `NtfsFs::open` | partial | `read::open_blank_volume` | | Reads the base `$MFT` extent and validates file references; attribute-list extents and `$MFTMirr` recovery are not supported. |
| NTFS:Directory-Index | `NtfsDir::entries` | partial | `read::large_directory_uses_index_allocation` | | Honors `$BITMAP`, update sequences, namespaces, and `$UpCase`; attribute-list index extents are not resolved. |
| NTFS:Data-Stream | `FileReader` | partial | `read::read_large_nonresident_file` | | Reads resident, non-resident, sparse, and uninitialized unnamed data; compressed, encrypted, named, and attribute-list streams are unsupported. |

## hadris-udf

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-167:3/7.2 | `DescriptorTag` | partial | `descriptor::tag::tests::validate_bytes_enforces_version_reserved_location_and_crc` | `udf_read` | Core tag invariants are checked, but validation across every descriptor context is not yet established. |
| ECMA-167:3/7.2.1 | `TagIdentifier` | partial | `comprehensive_udf::tag_identifier_conversions_cover_volume_and_file_descriptors` | `udf_read` | Known identifiers are modeled and tested, but context-specific identifier constraints are not all validated. |
| ECMA-167:3/7.1 | `ExtentDescriptor` | partial |  | `udf_read` | The layout is modeled and tested, but all extent semantics are not validated at this layer. |
| ECMA-167:1/7.4 | `EntityIdentifier` | partial |  | `udf_read` | The identifier layout is modeled, but suffix-specific constraints are not all validated. |
| ECMA-167:1/7.2.1 | `CharSpec` | partial | `write::cs0_tests::selects_eight_bit_for_latin1`, `write::cs0_tests::selects_sixteen_bit_for_wide_unicode` | `udf_read` | OSTA CS0 writing is tested, but every ECMA-167 character-set constraint is not validated. |
| ECMA-167:4/14.14.2 | `LongAllocationDescriptor` | partial |  | `udf_read` | The layout is modeled and tested, but all partition-reference and extent semantics are not validated at this layer. |
| ECMA-167:4/14.14.1 | `ShortAllocationDescriptor` | partial |  | `udf_read` | The layout is modeled and tested, but all allocation-length semantics are not validated at this layer. |
| ECMA-167:3/10.2 | `AnchorVolumeDescriptorPointer` | partial | `integration_external::write_tests::test_hadris_udf_has_valid_avdp` | `udf_read` | The descriptor is modeled and tested, but clause-complete validation has not yet been established. |
| ECMA-167:3/10.1 | `PrimaryVolumeDescriptor` | partial | `write::tests::test_roundtrip_basic_verification` | `udf_read` | The descriptor is modeled and round-trip tested, but clause-complete validation has not yet been established. |
| ECMA-167:3/10.5 | `PartitionDescriptor` | partial | `descriptor::partition::tests::partition_descriptor_layout_and_validate` | `udf_read` | The descriptor layout and core fields are validated, but clause-complete semantic validation is not established. |
| ECMA-167:3/10.6 | `LogicalVolumeDescriptor` | partial | `write::tests::test_roundtrip_basic_verification` | `udf_read` | The descriptor is modeled and tested, but clause-complete validation has not yet been established. |
| ECMA-167:3/10.7.2 | `Type1PartitionMap` | partial | `descriptor::logical::tests::type1_partition_maps_parses_embedded_table` | `udf_read` | Type 1 maps are parsed, while validation of every table-level constraint is not yet established. |
| ECMA-167:4/14.1 | `FileSetDescriptor` | partial | `write::tests::test_roundtrip_basic_verification` | `udf_read` | The descriptor is modeled and tested, but clause-complete validation has not yet been established. |

## hadris-iso

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-119:7.5.1 | `convert_l1` | full | `iso::spec::hadris_iso_matches_ecma_119_oracle` | | Level 1 file identifiers include the required file-version separator and version number. |
| ECMA-119:8.2 | `BootRecordVolumeDescriptor` | partial | `iso::boot::test_hadris_multisection_boot_catalog` | `iso_read` | The descriptor locates El Torito data, but all ECMA-119 boot-record semantics are not implemented. |
| ECMA-119:8.3 | `VolumeDescriptorSetTerminator` | partial | `comprehensive_iso::malformed_primary_descriptor_and_terminator_cases_are_rejected` | `iso_read` | The descriptor is emitted and recognized, but the audit has not established validation of every reserved byte. |
| ECMA-119:8.4 | `PrimaryVolumeDescriptor` | partial | `comprehensive_iso::descriptor_sequence_opens_primary_volume_and_root_directory`, `iso::spec::hadris_iso_matches_ecma_119_oracle` | `iso_read` | Core fields are modeled, but reserved fields, character sets, redundant endian values, and semantic constraints are not all validated. |
| ECMA-119:8.5 | `SupplementaryVolumeDescriptor` | partial | | `iso_read` | Joliet SVD is read/written (UCS-2, BMP only); the version-2 "enhanced" form is repurposed as a UDF-bridge signal rather than a conformant ISO 9660:1999 secondary descriptor. |
| ECMA-119:9.1 | `DirectoryRecordHeader` | partial | `directory::tests::directory_record_parse_roundtrip` | `iso_read` | Fixed fields round-trip, but all identifier, flag, and semantic constraints are not yet validated. |
| ECMA-119:9.1 | `DirectoryRecord` | partial | `directory::tests::directory_record_parse_roundtrip` | `iso_read` | Joliet+RRIP coexistence on read may hide one namespace; see crate Known Limitations |
| ECMA-119:9.4 | `PathTableEntryHeader` | partial | `iso::spec::hadris_iso_matches_ecma_119_oracle` | `iso_read` | Both L- and M-type path tables are written and read; the optional secondary path tables are not populated. |
| El-Torito:validation | `BootValidationEntry` | partial | `iso::boot::test_eltorito_boot_catalog_comparison` | `iso_read` | The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation. |
| El-Torito:section-header | `BootSectionHeaderEntry` | partial | `iso::boot::test_hadris_multisection_boot_catalog` | `iso_read` | The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation. |
| El-Torito:section-entry | `BootSectionEntry` | partial | `iso::boot::test_floppy_emulation_media_type_and_default_load_size` | `iso_read` | The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation. |

## hadris-fat

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| FAT:BPB | `RawBpb` | full | `comprehensive_fat::bpb_size_validation_uses_production_reader_and_formatter` | `fat_read` | |
| FAT:FSInfo | `RawFsInfo` | full | `test_write::test_fsinfo_unknown_sentinels_mount_successfully` | `fat_read` | FAT32 free-cluster/next-free tracking |
| FAT:LFN | `RawLfnEntry` | partial | `test_write::lfn_checksum_matches_short_name`, `test_write::lfn_padding_uses_terminator_then_filler`, `test_write::maximum_length_name_spans_clusters`, `test_write::long_name_exceeding_one_cluster_roundtrips_and_deletes` | `fat_read` | This raw on-disk structure is complete, while semantic validation and legacy ANSI fallback behavior are implemented by higher-level LFN readers and writers. |
| FAT:DirEntry | `RawFileEntry` | partial | `test_write::test_lowercase_short_name_uses_nt_case_flags` | `fat_read` | Name/attributes/timestamps/cluster/size and NT case flags (`DIR_NTRes`) are read and written; extended access-time granularity is not modeled. |

## hadris-part

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| MBR:layout | `MasterBootRecord` | unknown | `roundtrip::mbr_write_read_roundtrip` | | Behavior is tested, but authoritative source text was unavailable for this audit. |
| UEFI:GPT-Header | `GptHeader` | unknown | `io_roundtrip::gpt_scheme_sync_write_open_and_detect_roundtrip` | | Behavior is tested, but authoritative source text was unavailable for this audit. |
| UEFI:GPT-Entry | `GptPartitionEntry` | unknown | `roundtrip::gpt_partition_entry_roundtrip` | | Behavior is tested, but authoritative source text was unavailable for this audit. |
