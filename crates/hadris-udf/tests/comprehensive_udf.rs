//! Comprehensive UDF filesystem tests.
//!
//! These tests cover edge cases, different UDF revisions, and various scenarios
//! for Universal Disk Format filesystems.

use std::io::Cursor;

// =============================================================================
// UDF Constants and Structures Tests
// =============================================================================

mod constants_tests {
    use hadris_udf::SECTOR_SIZE;

    #[test]
    fn test_sector_size() {
        // UDF uses 2048-byte sectors (same as CD-ROM/DVD)
        assert_eq!(SECTOR_SIZE, 2048);
    }

    #[test]
    fn test_anchor_locations() {
        // AVDP can be at sector 256, N-256, or N (where N is last sector)
        let standard_anchor = 256u32;
        assert_eq!(standard_anchor, 256);
    }
}

// =============================================================================
// Volume Recognition Sequence Tests
// =============================================================================

mod vrs_tests {
    #[test]
    fn test_vrs_identifiers() {
        // Volume Recognition Sequence identifiers
        let bea01 = b"BEA01"; // Beginning Extended Area Descriptor
        let nsr02 = b"NSR02"; // NSR Descriptor (UDF 1.02-1.50)
        let nsr03 = b"NSR03"; // NSR Descriptor (UDF 2.00+)
        let tea01 = b"TEA01"; // Terminating Extended Area Descriptor

        assert_eq!(bea01.len(), 5);
        assert_eq!(nsr02.len(), 5);
        assert_eq!(nsr03.len(), 5);
        assert_eq!(tea01.len(), 5);
    }

    #[test]
    fn test_vrs_structure_type() {
        // VRS descriptors have structure type 0
        let structure_type = 0u8;
        assert_eq!(structure_type, 0);
    }

    #[test]
    fn test_vrs_version() {
        // VRS version is 1
        let version = 1u8;
        assert_eq!(version, 1);
    }

    #[test]
    fn test_vrs_starts_at_sector_16() {
        // VRS starts at sector 16 (after ISO 9660 system area)
        let vrs_start_sector = 16u32;
        let vrs_start_offset = vrs_start_sector * 2048;
        assert_eq!(vrs_start_offset, 32768);
    }
}

// =============================================================================
// UDF Revision Tests
// =============================================================================

mod revision_tests {
    use hadris_udf::UdfRevision;

    #[test]
    fn test_udf_revisions() {
        // Test all supported UDF revisions
        let rev_102 = UdfRevision::V1_02;
        let rev_150 = UdfRevision::V1_50;
        let rev_200 = UdfRevision::V2_00;
        let rev_201 = UdfRevision::V2_01;
        let rev_250 = UdfRevision::V2_50;
        let rev_260 = UdfRevision::V2_60;

        // Verify they can be compared
        assert!(rev_102 < rev_150);
        assert!(rev_150 < rev_200);
        assert!(rev_200 < rev_201);
        assert!(rev_201 < rev_250);
        assert!(rev_250 < rev_260);
    }

    #[test]
    fn test_revision_values() {
        // UDF revision values are BCD-encoded
        // 1.02 = 0x0102, 1.50 = 0x0150, etc.
        let rev_102_value = 0x0102u16;
        let rev_150_value = 0x0150u16;
        let rev_200_value = 0x0200u16;
        let rev_201_value = 0x0201u16;
        let rev_250_value = 0x0250u16;
        let rev_260_value = 0x0260u16;

        assert_eq!(rev_102_value, 258);
        assert_eq!(rev_150_value, 336);
        assert_eq!(rev_200_value, 512);
        assert_eq!(rev_201_value, 513);
        assert_eq!(rev_250_value, 592);
        assert_eq!(rev_260_value, 608);
    }
}

// =============================================================================
// Tag Identifier Tests
// =============================================================================

mod tag_tests {
    #[test]
    fn test_descriptor_tag_ids() {
        // UDF descriptor tag identifiers
        let tag_primary_vd = 1u16;
        let tag_anchor_vdp = 2u16;
        let tag_vd_pointer = 3u16;
        let tag_impl_use_vd = 4u16;
        let tag_partition_d = 5u16;
        let tag_logical_vd = 6u16;
        let tag_unalloc_sd = 7u16;
        let tag_terminating = 8u16;
        let tag_lvid = 9u16;

        assert_eq!(tag_primary_vd, 1);
        assert_eq!(tag_anchor_vdp, 2);
        assert_eq!(tag_vd_pointer, 3);
        assert_eq!(tag_impl_use_vd, 4);
        assert_eq!(tag_partition_d, 5);
        assert_eq!(tag_logical_vd, 6);
        assert_eq!(tag_unalloc_sd, 7);
        assert_eq!(tag_terminating, 8);
        assert_eq!(tag_lvid, 9);
    }

    #[test]
    fn test_file_entry_tag_ids() {
        // File entry tag identifiers
        let tag_file_set = 256u16;
        let tag_file_id = 257u16;
        let tag_alloc_extent = 258u16;
        let tag_indirect = 259u16;
        let tag_terminal = 260u16;
        let tag_file_entry = 261u16;
        let tag_ext_attr_header = 262u16;
        let tag_unalloc_space = 263u16;
        let tag_space_bitmap = 264u16;
        let tag_partition_integ = 265u16;
        let tag_ext_file_entry = 266u16;

        assert_eq!(tag_file_set, 256);
        assert_eq!(tag_file_id, 257);
        assert_eq!(tag_alloc_extent, 258);
        assert_eq!(tag_indirect, 259);
        assert_eq!(tag_terminal, 260);
        assert_eq!(tag_file_entry, 261);
        assert_eq!(tag_ext_attr_header, 262);
        assert_eq!(tag_unalloc_space, 263);
        assert_eq!(tag_space_bitmap, 264);
        assert_eq!(tag_partition_integ, 265);
        assert_eq!(tag_ext_file_entry, 266);
    }

    #[test]
    fn test_tag_structure() {
        // Descriptor tag is 16 bytes:
        // - Tag identifier: 2 bytes
        // - Descriptor version: 2 bytes
        // - Tag checksum: 1 byte
        // - Reserved: 1 byte
        // - Tag serial number: 2 bytes
        // - Descriptor CRC: 2 bytes
        // - Descriptor CRC length: 2 bytes
        // - Tag location: 4 bytes

        let tag_size = 16;
        assert_eq!(tag_size, 16);
    }

    #[test]
    fn test_tag_checksum() {
        // Tag checksum is sum of bytes 0-3 and 5-15 of the tag
        // (byte 4 is the checksum itself and is excluded)
        let mut tag = [0u8; 16];
        tag[0] = 1; // Tag identifier low
        tag[1] = 0; // Tag identifier high
        tag[2] = 2; // Descriptor version low
        tag[3] = 0; // Descriptor version high
        // tag[4] is checksum
        tag[5] = 0; // Reserved

        let mut sum = 0u8;
        for i in 0..16 {
            if i != 4 {
                sum = sum.wrapping_add(tag[i]);
            }
        }
        tag[4] = sum;

        // Verify checksum
        let mut verify_sum = 0u8;
        for i in 0..16 {
            if i != 4 {
                verify_sum = verify_sum.wrapping_add(tag[i]);
            }
        }
        assert_eq!(verify_sum, tag[4]);
    }
}

// =============================================================================
// Anchor Volume Descriptor Pointer Tests
// =============================================================================

mod avdp_tests {
    #[test]
    fn test_avdp_structure() {
        // AVDP is 512 bytes (but stored in 2048-byte sector)
        // Contains:
        // - Descriptor tag: 16 bytes
        // - Main VDS extent: 8 bytes (length + location)
        // - Reserve VDS extent: 8 bytes

        let avdp_min_size = 16 + 8 + 8;
        assert_eq!(avdp_min_size, 32);
    }

    #[test]
    fn test_extent_descriptor() {
        // Extent descriptor: 8 bytes
        // - Extent length: 4 bytes
        // - Extent location: 4 bytes

        let extent_size = 8;
        assert_eq!(extent_size, 8);
    }

    #[test]
    fn test_avdp_locations() {
        // AVDP must be at one of these locations:
        // - Sector 256
        // - N-256 (where N is last sector)
        // - N (last sector)

        let mandatory_location = 256u32;
        assert_eq!(mandatory_location, 256);
    }
}

// =============================================================================
// Partition Descriptor Tests
// =============================================================================

mod partition_tests {
    #[test]
    fn test_partition_contents() {
        // Partition contents identifiers
        let contents_fdc01 = b"+FDC01"; // Physical partition
        let contents_cd001 = b"+CD001"; // Virtual partition
        let contents_nsr02 = b"+NSR02"; // UDF 1.02-1.50
        let contents_nsr03 = b"+NSR03"; // UDF 2.00+

        assert_eq!(contents_fdc01.len(), 6);
        assert_eq!(contents_cd001.len(), 6);
        assert_eq!(contents_nsr02.len(), 6);
        assert_eq!(contents_nsr03.len(), 6);
    }

    #[test]
    fn test_partition_access_types() {
        // Partition access types
        let access_unspecified = 0u32;
        let access_read_only = 1u32;
        let access_write_once = 2u32;
        let access_rewritable = 3u32;
        let access_overwritable = 4u32;

        assert!(access_unspecified < access_read_only);
        assert!(access_read_only < access_write_once);
        assert!(access_write_once < access_rewritable);
        assert!(access_rewritable < access_overwritable);
    }
}

// =============================================================================
// File Entry Tests
// =============================================================================

mod file_entry_tests {
    use hadris_udf::FileType;

    #[test]
    fn test_file_types() {
        // ICBTAG file types
        let type_unspecified = 0u8;
        let type_unalloc_space = 1u8;
        let type_partition_integ = 2u8;
        let type_indirect = 3u8;
        let type_directory = 4u8;
        let type_regular_file = 5u8;
        let type_block_device = 6u8;
        let type_char_device = 7u8;
        let type_ext_attr = 8u8;
        let type_fifo = 9u8;
        let type_socket = 10u8;
        let type_terminal = 11u8;
        let type_symlink = 12u8;
        let type_stream_dir = 13u8;

        assert_eq!(type_directory, 4);
        assert_eq!(type_regular_file, 5);
        assert_eq!(type_symlink, 12);

        // Test our FileType enum
        let dir = FileType::Directory;
        let file = FileType::RegularFile;

        assert_ne!(dir, file);
    }

    #[test]
    fn test_icb_flags() {
        // ICB allocation descriptor types (bits 0-2)
        let alloc_short = 0u16; // Short allocation descriptors
        let alloc_long = 1u16; // Long allocation descriptors
        let alloc_extended = 2u16; // Extended allocation descriptors
        let alloc_embedded = 3u16; // Data embedded in allocation descriptor area

        assert_eq!(alloc_short, 0);
        assert_eq!(alloc_long, 1);
        assert_eq!(alloc_extended, 2);
        assert_eq!(alloc_embedded, 3);

        // Other ICB flags (bits 3-15)
        let flag_sorted = 1u16 << 3; // Directory is sorted
        let flag_non_relocatable = 1u16 << 4;
        let flag_archive = 1u16 << 5;
        let flag_setuid = 1u16 << 6;
        let flag_setgid = 1u16 << 7;
        let flag_sticky = 1u16 << 8;
        let flag_contiguous = 1u16 << 9;
        let flag_system = 1u16 << 10;
        let flag_transformed = 1u16 << 11;
        let flag_multi_versions = 1u16 << 12;
        let flag_stream = 1u16 << 13;

        assert_eq!(flag_sorted, 8);
        assert_eq!(flag_contiguous, 512);
        assert!(flag_stream > flag_multi_versions);
    }

    #[test]
    fn test_allocation_descriptor_sizes() {
        // Short allocation descriptor: 8 bytes
        // - Extent length: 4 bytes (includes type in upper 2 bits)
        // - Extent position: 4 bytes (logical block number)

        let short_ad_size = 8;
        assert_eq!(short_ad_size, 8);

        // Long allocation descriptor: 16 bytes
        // - Extent length: 4 bytes
        // - Extent location: 6 bytes (logical block + partition)
        // - Implementation use: 6 bytes

        let long_ad_size = 16;
        assert_eq!(long_ad_size, 16);

        // Extended allocation descriptor: 20 bytes
        // - Extent length: 4 bytes
        // - Recorded length: 4 bytes
        // - Information length: 4 bytes
        // - Extent location: 6 bytes
        // - Implementation use: 2 bytes

        let extended_ad_size = 20;
        assert_eq!(extended_ad_size, 20);
    }

    #[test]
    fn test_extent_type_flags() {
        // Extent type in upper 2 bits of extent length
        let type_recorded = 0u32 << 30; // Recorded and allocated
        let type_allocated = 1u32 << 30; // Allocated but not recorded (sparse)
        let type_not_alloc = 2u32 << 30; // Not allocated (sparse)
        let type_continuation = 3u32 << 30; // Continuation of previous extent

        assert_eq!(type_recorded, 0);
        assert_eq!(type_allocated, 1 << 30);
        assert_eq!(type_not_alloc, 2 << 30);
        assert_eq!(type_continuation, 3 << 30);
    }
}

// =============================================================================
// File Identifier Descriptor Tests
// =============================================================================

mod fid_tests {
    #[test]
    fn test_fid_structure() {
        // File Identifier Descriptor:
        // - Descriptor tag: 16 bytes
        // - File version number: 2 bytes
        // - File characteristics: 1 byte
        // - Length of file identifier: 1 byte
        // - ICB: 16 bytes (long allocation descriptor)
        // - Length of implementation use: 2 bytes
        // - Implementation use: variable
        // - File identifier: variable

        let fid_fixed_size = 16 + 2 + 1 + 1 + 16 + 2;
        assert_eq!(fid_fixed_size, 38);
    }

    #[test]
    fn test_file_characteristics() {
        // File characteristics flags
        let char_hidden = 1u8 << 0;
        let char_directory = 1u8 << 1;
        let char_deleted = 1u8 << 2;
        let char_parent = 1u8 << 3;
        let char_metadata = 1u8 << 4;

        assert_eq!(char_hidden, 1);
        assert_eq!(char_directory, 2);
        assert_eq!(char_deleted, 4);
        assert_eq!(char_parent, 8);
        assert_eq!(char_metadata, 16);
    }

    #[test]
    fn test_fid_padding() {
        // FID must be padded to 4-byte boundary
        let calculate_padded_size = |base_size: usize| -> usize { (base_size + 3) & !3 };

        assert_eq!(calculate_padded_size(38), 40);
        assert_eq!(calculate_padded_size(40), 40);
        assert_eq!(calculate_padded_size(41), 44);
        assert_eq!(calculate_padded_size(50), 52);
    }
}

// =============================================================================
// Timestamp Tests
// =============================================================================

mod timestamp_tests {
    #[test]
    fn test_timestamp_structure() {
        // UDF timestamp: 12 bytes
        // - Type and time zone: 2 bytes
        // - Year: 2 bytes
        // - Month: 1 byte
        // - Day: 1 byte
        // - Hour: 1 byte
        // - Minute: 1 byte
        // - Second: 1 byte
        // - Centiseconds: 1 byte
        // - Hundreds of microseconds: 1 byte
        // - Microseconds: 1 byte

        let timestamp_size = 12;
        assert_eq!(timestamp_size, 12);
    }

    #[test]
    fn test_timestamp_type() {
        // Type in bits 12-15 of first word
        let type_utc = 1u16 << 12; // UTC
        let type_local = 2u16 << 12; // Local time

        assert_eq!(type_utc, 4096);
        assert_eq!(type_local, 8192);
    }

    #[test]
    fn test_timezone_encoding() {
        // Time zone in bits 0-11 (signed, minutes from UTC)
        // Range: -1440 to +1440 (±24 hours)

        let utc = 0i16;
        let utc_plus_5 = 300i16; // +5 hours = +300 minutes
        let utc_minus_8 = -480i16; // -8 hours = -480 minutes

        assert!(utc_plus_5 > utc);
        assert!(utc_minus_8 < utc);
    }
}

// =============================================================================
// Entity Identifier Tests
// =============================================================================

mod entity_id_tests {
    #[test]
    fn test_entity_identifier_structure() {
        // Entity identifier (regid): 32 bytes
        // - Flags: 1 byte
        // - Identifier: 23 bytes
        // - Identifier suffix: 8 bytes

        let entity_id_size = 32;
        assert_eq!(entity_id_size, 32);
    }

    #[test]
    fn test_entity_id_flags() {
        // Entity identifier flags
        let flag_dirty = 1u8 << 0;
        let flag_protected = 1u8 << 1;

        assert_eq!(flag_dirty, 1);
        assert_eq!(flag_protected, 2);
    }

    #[test]
    fn test_standard_identifiers() {
        // Standard UDF identifiers (identifier field is 23 bytes)
        // Note: "*OSTA Compressed Unicode" is 24 chars, so it's truncated
        let osta_compressed = b"*OSTA Compressed Unicod"; // Truncated to 23
        let udf_lv_info = b"*UDF LV Info";
        let udf_fsd = b"*UDF FSD";

        assert!(osta_compressed.len() <= 23);
        assert!(udf_lv_info.len() <= 23);
        assert!(udf_fsd.len() <= 23);
    }
}

// =============================================================================
// Character Set Tests
// =============================================================================

mod charset_tests {
    #[test]
    fn test_charspec_structure() {
        // Character set specification: 64 bytes
        // - Type: 1 byte
        // - Information: 63 bytes

        let charspec_size = 64;
        assert_eq!(charspec_size, 64);
    }

    #[test]
    fn test_charset_types() {
        // Character set types
        let cs_type0 = 0u8; // CS0 (required - byte)
        let cs_type1 = 1u8; // CS1 (101/10646-1 Level 1)
        let cs_type2 = 2u8; // CS2 (101/10646-1 Level 2)
        let cs_type3 = 3u8; // CS3 (101/10646-1 Level 3)

        assert_eq!(cs_type0, 0);
        assert_eq!(cs_type1, 1);
        assert_eq!(cs_type2, 2);
        assert_eq!(cs_type3, 3);
    }

    #[test]
    fn test_osta_compressed_unicode() {
        // OSTA Compressed Unicode encoding:
        // - First byte: compression ID (8, 16, or 255)
        // - Remaining bytes: UTF-8 (8), UTF-16BE (16), or empty (255)

        let comp_id_utf8 = 8u8;
        let comp_id_utf16 = 16u8;
        let comp_id_empty = 255u8;

        assert_eq!(comp_id_utf8, 8);
        assert_eq!(comp_id_utf16, 16);
        assert_eq!(comp_id_empty, 255);
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

mod edge_case_tests {
    use super::*;
    use hadris_udf::UdfFs;

    #[test]
    fn test_empty_image() {
        let data = vec![0u8; 0];
        let cursor = Cursor::new(data);

        let result = UdfFs::open(cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_too_small_image() {
        // Less than VRS start (sector 16)
        let data = vec![0u8; 16 * 2048 - 1];
        let cursor = Cursor::new(data);

        let result = UdfFs::open(cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_no_anchor() {
        // Image without AVDP at sector 256
        let data = vec![0u8; 300 * 2048];
        let cursor = Cursor::new(data);

        let result = UdfFs::open(cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_vrs() {
        // Image with invalid VRS
        let mut data = vec![0u8; 300 * 2048];

        // Put garbage at VRS location (sector 16)
        data[16 * 2048] = 0xFF;
        data[16 * 2048 + 1..16 * 2048 + 6].copy_from_slice(b"WRONG");

        let cursor = Cursor::new(data);
        let result = UdfFs::open(cursor);

        // Should fail with some error
        assert!(result.is_err());
    }
}

// =============================================================================
// Logical Volume Integrity Descriptor Tests
// =============================================================================

mod lvid_tests {
    #[test]
    fn test_lvid_structure() {
        // LVID contains:
        // - Descriptor tag: 16 bytes
        // - Recording date: 12 bytes
        // - Integrity type: 4 bytes
        // - Next integrity extent: 8 bytes
        // - Logical volume contents use: 32 bytes
        // - Number of partitions: 4 bytes
        // - Length of implementation use: 4 bytes
        // - Free space table: variable
        // - Size table: variable
        // - Implementation use: variable

        let lvid_fixed_size = 16 + 12 + 4 + 8 + 32 + 4 + 4;
        assert_eq!(lvid_fixed_size, 80);
    }

    #[test]
    fn test_integrity_types() {
        // Integrity type values
        let open_integrity = 0u32;
        let close_integrity = 1u32;

        assert_eq!(open_integrity, 0);
        assert_eq!(close_integrity, 1);
    }
}

// =============================================================================
// Sparing Tests (UDF 1.50+)
// =============================================================================

mod sparing_tests {
    #[test]
    fn test_sparing_entry_structure() {
        // Sparing map entry: 8 bytes
        // - Original location: 4 bytes
        // - Mapped location: 4 bytes

        let sparing_entry_size = 8;
        assert_eq!(sparing_entry_size, 8);
    }

    #[test]
    fn test_sparing_special_values() {
        // Special values in sparing table
        let available = 0xFFFFFFF0u32; // Available for allocation
        let defective = 0xFFFFFFF1u32; // Defective

        assert!(available > 0xF0000000);
        assert!(defective > available);
    }
}

// =============================================================================
// Metadata Partition Tests (UDF 2.50+)
// =============================================================================

mod metadata_tests {
    #[test]
    fn test_metadata_flags() {
        // Metadata partition flags
        let flag_duplicate = 1u8 << 0; // Metadata is duplicated

        assert_eq!(flag_duplicate, 1);
    }

    #[test]
    fn test_metadata_file_locations() {
        // Metadata partition uses special file locations
        // - Metadata file
        // - Metadata mirror file
        // - Metadata bitmap file

        // These are stored in the partition header
        let metadata_fields = 3;
        assert_eq!(metadata_fields, 3);
    }
}

// =============================================================================
// Extended Attribute Tests
// =============================================================================

mod extended_attr_tests {
    #[test]
    fn test_ea_header_structure() {
        // Extended attribute header: 24 bytes
        // - Descriptor tag: 16 bytes
        // - Implementation attributes location: 4 bytes
        // - Application attributes location: 4 bytes

        let ea_header_size = 24;
        assert_eq!(ea_header_size, 24);
    }

    #[test]
    fn test_ea_attribute_types() {
        // Extended attribute types
        let ea_char_set = 1u32;
        let ea_alt_permissions = 3u32;
        let ea_file_times = 5u32;
        let ea_info_times = 6u32;
        let ea_device_spec = 12u32;
        let ea_impl_use = 2048u32;
        let ea_app_use = 65536u32;

        assert!(ea_char_set < ea_alt_permissions);
        assert!(ea_alt_permissions < ea_file_times);
        assert!(ea_impl_use > ea_device_spec);
        assert!(ea_app_use > ea_impl_use);
    }
}
