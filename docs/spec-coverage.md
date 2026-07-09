# Spec coverage

Maintainer audit index for standards-facing types in Hadris.
Not a public marketing matrix (v0).

**Design:** [`docs/superpowers/specs/2026-07-09-spec-compliance-program-design.md`](superpowers/specs/2026-07-09-spec-compliance-program-design.md)

**How to update**

1. `rg '@hadris-spec' crates/`
2. Sync rows below (one primary row per annotated item).
3. Prefer `partial` + Notes over claiming `full`.

Fuzz columns name targets under `fuzz/` (local only — not PR CI).

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
| ECMA-167:3/10.5 | `PartitionDescriptor` | full | `comprehensive_udf::test_partition_contents` | `udf_read` | |
| ECMA-167:3/10.6 | `LogicalVolumeDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |
| ECMA-167:3/10.7.2 | `Type1PartitionMap` | full | `descriptor::logical::tests::type1_partition_maps_parses_embedded_table` | `udf_read` | |
| ECMA-167:4/14.1 | `FileSetDescriptor` | full | `comprehensive_udf::test_allocation_descriptor_sizes` | `udf_read` | |

## hadris-iso

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| ECMA-119:8.4 | `PrimaryVolumeDescriptor` | full | `comprehensive_iso::test_pvd_standard_identifier` | `iso_read` | |
| ECMA-119:9.1 | `DirectoryRecordHeader` | full | `comprehensive_iso::test_directory_record_structure` | `iso_read` | Fixed fields of the directory record |
| ECMA-119:9.1 | `DirectoryRecord` | partial | `comprehensive_iso::test_directory_record_structure` | `iso_read` | Joliet+RRIP coexistence on read may hide one namespace |

## hadris-fat

| Spec | Item | Compliance | Tests | Fuzz | Notes |
|------|------|------------|-------|------|-------|
| *(pilot rows added in Task 5)* | | | | | |
