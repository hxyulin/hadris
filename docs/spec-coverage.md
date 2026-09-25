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
| NTFS:Boot-Sector | `BootSector` | partial | `crafted::open_rejects_bad_boot_sectors`, `raw::tests::boot_sector_reads_its_fields` | `ntfs_read` | Geometry and locations are validated; the checksum and the backup boot sector are not used. |
| NTFS:Boot-Sector | `record_size` | partial | `record::tests::record_sizes_decode_both_encodings` | `ntfs_read` | Sizes above 4096 bytes are refused as unsupported. |
| NTFS:Update-Sequence-Array | `apply_fixups` | unknown | `record::tests::fixups_restore_each_stride`, `record::tests::fixups_reject_a_short_count`, `record::tests::fixups_use_512_byte_strides_on_4k_records` | `ntfs_read` | Behavior is tested, but authoritative source text was unavailable for this audit. |
| NTFS:Attribute-Record | `Attrs` | partial | `record::tests::attributes_are_bounded_by_the_used_size`, `record::tests::attributes_stop_at_the_end_marker`, `record::tests::attributes_need_an_end_marker` | `ntfs_read` | Resident and non-resident headers are validated within one record; the attributes of extension records are reached through `$ATTRIBUTE_LIST`. |
| NTFS:Attribute-List | `list_entry` | partial | `record::tests::list_entries_are_bounded`, `crafted::attribute_lists_join_extension_records`, `crafted::attribute_list_gaps_fail`, `crafted::index_roots_in_extension_records_are_followed`, `read::attribute_lists_on_a_real_volume` | `ntfs_read` | Streams, names and indexes are followed into extension records; `$MFT` may have at most 32 extents. |
| NTFS:Mapping-Pairs | `Runs` | unknown | `record::tests::runs_decode_relative_and_sparse_extents`, `record::tests::runs_reject_malformed_encodings` | `ntfs_read` | Behavior is tested, but authoritative source text was unavailable for this audit. |
| NTFS:File-Name | `file_name` | partial | `record::tests::file_names_parse_and_bound_the_name` | `ntfs_read` | Parses the parent reference, namespace and full UTF-16 name; the flags, the copies of times and sizes and the reparse tag are not used. |
| NTFS:Index-Entry | `index_entry` | partial | `record::tests::index_entries_are_bounded_by_the_node`, `read::large_directory_lists_every_entry` | `ntfs_read` | Every node is enumerated; child-node pointers are not followed for a keyed descent. |
| NTFS:Master-File-Table | `NtfsFs::mount` | partial | `read::open_blank_volume`, `crafted::open_rejects_bad_boot_sectors`, `crafted::fragmented_mft_is_followed` | `ntfs_read` | Reads `$MFT` from its base record and extension records and checks file references; `$MFTMirr` recovery is not supported. |
| NTFS:Directory-Index | `NtfsFs::lookup` | partial | `crafted::names_fold_case_through_upcase`, `read::large_directory_lists_every_entry` | `ntfs_read` | Walks every index node instead of descending the B-tree by key. |
| NTFS:Data-Stream | `NtfsFs::read_at` | partial | `read::files_read_back`, `crafted::streams_past_the_volume_fail`, `crafted::attribute_lists_join_extension_records` | `ntfs_read` | Reads resident, non-resident, sparse and partly initialized streams, also across extension records; compressed and encrypted streams are unsupported. |
| NTFS:Named-Streams | `NtfsFs::streams` | partial | `crafted::named_streams_are_listed_and_read`, `read::named_streams_read_back` | `ntfs_read` | Lists and reads named `$DATA` attributes; other attribute types are not exposed as streams. |

## hadris-udf

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-167:2/9.1 | `VolumeStructureDescriptor` | partial | `errors::malformed_volumes_are_refused`, `roundtrip::every_tree_reads_back` | `udf_read` | BEA01, NSR02 or NSR03 and TEA01 are written, after ISO 9660 descriptors in a bridge volume; the reader requires an NSR descriptor inside an extended area. |
| ECMA-167:3/10.2 | `AnchorVolumeDescriptorPointer` | partial | `roundtrip::every_tree_reads_back`, `errors::reserve_sequence_and_backup_anchor_are_used` | `udf_read` | Written at block 256 and at N-256; the reader looks at 256, N-256 and N-1 for each logical block size and falls back to the reserve sequence. |
| ECMA-167:3/10.1 | `PrimaryVolumeDescriptor` | partial | `roundtrip::every_tree_reads_back`, `read::prevailing_descriptors_win` | `udf_read` | The prevailing descriptor by sequence number is used; its volume identifier is read, the other fields are written but not checked. |
| ECMA-167:3/10.5 | `PartitionDescriptor` | partial | `roundtrip::every_tree_reads_back`, `read::prevailing_descriptors_win` | `udf_read` | The prevailing descriptor for each partition number is used; the start and length bound every extent; contents other than NSR02 and NSR03 are refused. |
| ECMA-167:3/10.6 | `LogicalVolumeDescriptor` | partial | `roundtrip::every_tree_reads_back`, `errors::unsupported_partition_maps_are_refused` | `udf_read` | The logical block size must be 512 to 4096 bytes and agree with the anchor; type 1 partition maps are read and other map types are refused as unsupported. |
| ECMA-167:3/10.7.2 | `Type1PartitionMap` | partial | `read::prevailing_descriptors_win`, `errors::unsupported_partition_maps_are_refused` | `udf_read` | Up to eight type 1 maps are read; type 2 maps (virtual, sparable and metadata partitions) are refused as unsupported. |
| ECMA-167:3/10.10 | `LogicalVolumeIntegrityDescriptor` | partial | `roundtrip::every_tree_reads_back` | `udf_read` | Written closed with the next unique id, the partition size, and the file and directory counts; the reader takes the free space of a closed descriptor for the volume statistics. |
| ECMA-167:4/14.1 | `FileSetDescriptor` | partial | `roundtrip::every_tree_reads_back`, `errors::malformed_volumes_are_refused` | `udf_read` | The first descriptor of the file set sequence gives the root directory; later file sets and the system stream directory are not read. |
| ECMA-167:4/14.6 | `IcbTag` | partial | `roundtrip::every_tree_reads_back` | `udf_read` | Strategy 4 entries are read and written; the file type, allocation type and setuid, setgid and sticky flags are used, and indirect entries of strategy 4096 are not followed. |
| ECMA-167:4/14.9 | `FileEntry` | partial | `roundtrip::every_tree_reads_back`, `roundtrip::metadata_reads_back` | `udf_read` | Read with the file type, size, owner, permissions, link count and times; written with short allocation descriptors of at most 1 GiB each. |
| ECMA-167:4/14.17 | `ExtendedFileEntry` | partial | `read::extended_file_entries_read` | `udf_read` | Read like a file entry, with the creation time; the stream directory is not read. |
| ECMA-167:4/14.4 | `FileIdentifierDescriptor` | partial | `roundtrip::every_tree_reads_back`, `read::identifiers_cross_extent_boundaries` | `udf_read` | Identifiers are read across block and extent boundaries; deleted and parent identifiers are skipped in listings, and the tag location is written as the block holding the first byte but not checked on read. |
| ECMA-167:4/14.5 | `AllocationExtentDescriptor` | partial | `read::allocation_descriptors_of_every_form_read_back` | `udf_read` | Followed from continuation extents of every allocation descriptor form, with a bound on the chain length; not written. |
| ECMA-167:4/14.16.1 | `PathComponent` | partial | `name::tests::symlink_targets_round_trip` | `udf_read` | Root, parent, current and named components are read and written; component versions are ignored. |
| ECMA-167:3/7.2.1 | `tag` | partial | `raw::tests::tag_seal_and_parse_roundtrip` | `udf_read` | Every identifier of ECMA-167 parts 3 and 4 is named; the reader checks the identifier each context requires. |
| ECMA-167:3/7.2 | `Tag` | partial | `raw::tests::tag_seal_and_parse_roundtrip`, `errors::malformed_volumes_are_refused` | `udf_read` | The checksum, CRC, identifier, version, reserved byte and location are checked for every volume, file set, file entry and allocation extent descriptor; identifier descriptors are not checked for their location. |
| ECMA-167:3/7.1 | `ExtentAd` | partial |  | `udf_read` | Extents of the volume descriptor sequences and the integrity sequence are followed; their bounds are checked against the device. |
| ECMA-167:4/14.14.1 | `ShortAd` | partial | `read::allocation_descriptors_of_every_form_read_back` | `udf_read` | The four extent types are read; allocated-not-recorded and unallocated extents read as zeros, and a continuation leads to an allocation extent descriptor. |
| ECMA-167:4/14.14.2 | `LongAd` | partial | `read::allocation_descriptors_of_every_form_read_back` | `udf_read` | The partition reference is checked against the logical volume's partition maps; the implementation use bytes are written for UDF 2.00 and later and ignored on read. |
| ECMA-167:4/14.14.3 | `ExtAd` | partial | `read::allocation_descriptors_of_every_form_read_back` | `udf_read` | Read like long allocation descriptors; the recorded and information lengths are not used. |
| ECMA-167:1/7.4 | `EntityId` | partial |  | `udf_read` | The domain identifier's revision suffix is read; other suffixes are written but not checked. |
| ECMA-167:1/7.2.1 | `CharSpec` | partial | `name::tests::cs0_round_trips_latin1_and_utf16` | `udf_read` | Written as OSTA Compressed Unicode, the only character set UDF allows; the reader does not check it. |
| ECMA-167:1/7.3 | `Timestamp` | partial | `time::tests::timestamps_round_trip` | `udf_read` | Coordinated and local times with an offset convert to and from `hadris_fs::DateTime`; agreement times and times without an offset read as UTC. |

## hadris-iso

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-119:7.5.1 | `convert_l1` | full | `iso::spec::hadris_iso_matches_ecma_119_oracle` | | Level 1 file identifiers include the required file-version separator and version number. |
| ECMA-119:8.2 | `BootRecordVolumeDescriptor` | partial | `iso::boot::test_hadris_multisection_boot_catalog` | `iso_read` | The descriptor locates El Torito data, but all ECMA-119 boot-record semantics are not implemented. |
| ECMA-119:8.3 | `VolumeDescriptorSetTerminator` | partial | `errors::malformed_images_are_refused` | `iso_read` | The descriptor is emitted and recognized, and its body must be zero; the audit has not established validation of every other rule. |
| ECMA-119:8.4 | `PrimaryVolumeDescriptor` | partial | `errors::malformed_images_are_refused`, `roundtrip::every_tree_reads_back`, `iso::spec::hadris_iso_matches_ecma_119_oracle` | `iso_read` | Core fields and their redundant endian copies are validated on read, but reserved fields, character sets and semantic constraints are not all validated. |
| ECMA-119:8.5 | `SupplementaryVolumeDescriptor` | partial | `roundtrip::every_tree_reads_back` | `iso_read` | Joliet SVDs (UCS-2, BMP only) and version-2 enhanced descriptors are read and written; the escape sequences recognized are the three Joliet levels. |
| ECMA-119:9.1 | `DirectoryRecordHeader` | partial | `raw::directory::tests::directory_record_parse_roundtrip` | `iso_read` | Fixed fields round-trip, but all identifier, flag, and semantic constraints are not yet validated. |
| ECMA-119:9.1 | `DirectoryRecord` | partial | `raw::directory::tests::directory_record_parse_roundtrip`, `raw::directory::tests::directory_record_rejects_invalid_bounds_and_endian_copy` | `iso_read` | Records are validated for length, padding and redundant fields on read; identifier character sets are not. |
| ECMA-119:9.4 | `PathTableHeader` | partial | `iso::spec::hadris_iso_matches_ecma_119_oracle` | `iso_read` | Both L- and M-type path tables are written and read; the optional secondary path tables are not populated. |
| El-Torito:validation | `BootValidationEntry` | partial | `iso::boot::test_eltorito_boot_catalog_comparison` | `iso_read` | The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation. |
| El-Torito:section-header | `BootCatalogHeader` | partial | `iso::boot::test_hadris_multisection_boot_catalog` | `iso_read` | The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation. |
| El-Torito:section-entry | `BootSectionEntry` | partial | `iso::boot::test_floppy_emulation_media_type_and_default_load_size` | `iso_read` | The catalog entry is modeled and interoperability-tested, but the audit has not established clause-complete validation. |

## hadris-fat

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| FAT:BPB | `RawBpb` | full | `boot::tests::check_bpb_rejects_bad_sizes`, `fatfs_format::rejects_bad_options_and_devices` | `fat_read` | |
| FAT:FSInfo | `RawFsInfo` | full | `boot::tests::fs_info_signatures`, `fatfs_read::fsinfo_unknown_values_mount_and_count_by_scanning` | `fat_read` | FAT32 free-cluster/next-free tracking |
| FAT:LFN | `RawLfnEntry` | partial | `lfn::tests::checksum_matches_reference`, `lfn::tests::encoded_orders_entries_last_first`, `lfn::tests::assembler_rejects_broken_sequences`, `fatfs_write::long_names_up_to_255_units`, `dirent::tests::layouts_match_the_specification` | `fat_read` | Sequence, attributes, checksum, terminator and filler are read and written; names are UTF-16 only, with no legacy ANSI fallback. |
| FAT:DirEntry | `RawDirEntry` | partial | `slot::tests::decodes_short_fields`, `fatfs_write::short_names_and_case_bits`, `dirent::tests::layouts_match_the_specification` | `fat_read` | Name/attributes/timestamps/cluster/size and NT case flags (`DIR_NTRes`) are read and written; extended access-time granularity is not modeled. |
| EXFAT:3.1 | `BootSector` | partial | `exfat_read::mount_rejects_bad_boot_sectors`, `exfat_format::formats_every_sector_size`, `exfat_read::damaged_main_boot_regions_mount_from_the_backup` | `exfat_read` | Every field and the boot checksum are checked at mount except `PartitionOffset` and `DriveSelect`; a valid backup boot region replaces a damaged main one, read-only, and `check` reports any difference. |
| EXFAT:7.4 | `FileEntry` | partial | `exfat_write::times_and_attributes_round_trip`, `exfat_read::entry_sets_cross_clusters` | `exfat_read` | Attributes and the three timestamps with their 10 ms increments and UTC offsets are read and written; entry sets with benign secondary entries are read and kept. |
| EXFAT:7.6 | `StreamEntry` | full | `exfat_read::valid_data_length_reads_zeros`, `exfat_write::contiguous_files_grow_into_chains` | `exfat_read` |  |
| EXFAT:7.7 | `NameEntry` | full | `exfat_write::long_names_up_to_255_units`, `exfat::codec::tests::names_are_checked` | `exfat_read` |  |
| EXFAT:7.1 | `BitmapEntry` | partial | `exfat_read::fragmented_bitmap_and_upcase_table`, `exfat_check::bitmap_mismatches_are_found`, `exfat_write::texfat_keeps_both_fats_and_bitmaps` | `exfat_read` | Bitmaps are read and written through their FAT chains; on TexFAT volumes the bitmap `ActiveFat` selects is read and both are written. TexFAT transactions are not supported. |
| EXFAT:7.2 | `UpcaseEntry` | full | `exfat_read::fragmented_bitmap_and_upcase_table`, `exfat_check::upcase_checksum_is_checked` | `exfat_read` |  |
| EXFAT:7.3 | `LabelEntry` | full | `exfat_write::labels_are_set_and_removed` | `exfat_read` |  |

## hadris-part

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| MBR:layout | `RawMbr` | unknown | `roundtrip::mbr_layout_roundtrip` | `part_read` | Behavior is tested, but authoritative source text was unavailable for this audit. |
| MBR:partition-entry | `RawMbrEntry` | unknown | `roundtrip::mbr_layout_roundtrip` | `part_read` | Behavior is tested, including EBR chains, but authoritative source text was unavailable for this audit. |
| UEFI:GPT-Header | `RawGptHeader` | unknown | `read::backup_gpt_replaces_a_corrupt_primary`, `roundtrip::gpt_layout_roundtrip` | `part_read` | Behavior is tested, but authoritative source text was unavailable for this audit. |
| UEFI:GPT-Entry | `RawGptEntry` | unknown | `roundtrip::gpt_layout_roundtrip`, `roundtrip::utf16_names_roundtrip` | `part_read` | Behavior is tested, but authoritative source text was unavailable for this audit. |
