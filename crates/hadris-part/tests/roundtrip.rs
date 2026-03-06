//! Integration tests for hadris-part: roundtrip, geometry, edge cases.

use hadris_part::error::PartitionError;
use hadris_part::geometry::{DiskGeometry, validate_partition_alignment};
use hadris_part::gpt::{GptPartitionEntry, Guid};
use hadris_part::hybrid::{HybridMbrBuilder, is_hybrid_mbr};
use hadris_part::mbr::{MasterBootRecord, MbrPartition, MbrPartitionTable, MbrPartitionType};
use hadris_part::scheme::{PartitionSchemeType, detect_scheme_from_mbr};
use hadris_part::{PartitionInfoTrait, PartitionTableRead};

// ---------------------------------------------------------------------------
// MBR roundtrip via bytemuck
// ---------------------------------------------------------------------------

#[test]
fn mbr_write_read_roundtrip() {
    let mut mbr = MasterBootRecord::default();
    mbr.with_partition_table(|table| {
        table[0] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
        table[1] = MbrPartition::new(MbrPartitionType::LinuxNative, 206848, 1000000);
    });
    assert!(mbr.has_valid_signature());

    let bytes: &[u8] = bytemuck::bytes_of(&mbr);
    assert_eq!(bytes.len(), 512);
    let mbr2: &MasterBootRecord = bytemuck::from_bytes(bytes);

    assert!(mbr2.has_valid_signature());
    let t2 = mbr2.get_partition_table();
    assert_eq!(t2.count(), 2);
    assert_eq!(t2[0].start_lba as u64, 2048);
    assert_eq!(t2[0].sector_count as u64, 204800);
    assert_eq!(t2[1].start_lba as u64, 206848);
}

#[test]
fn mbr_protective_roundtrip() {
    let disk_sectors = 1_953_525_168u64;
    let mbr = MasterBootRecord::protective(disk_sectors);

    let bytes = bytemuck::bytes_of(&mbr);
    let mbr2: &MasterBootRecord = bytemuck::from_bytes(bytes);

    assert!(mbr2.has_valid_signature());
    assert!(mbr2.get_partition_table().is_protective());
}

// ---------------------------------------------------------------------------
// GPT roundtrip via bytemuck
// ---------------------------------------------------------------------------

#[test]
fn gpt_partition_entry_roundtrip() {
    let entry = GptPartitionEntry::new(
        Guid::EFI_SYSTEM,
        Guid::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
        2048,
        206847,
    );

    let bytes = bytemuck::bytes_of(&entry);
    assert_eq!(bytes.len(), 128);

    let entry2: &GptPartitionEntry = bytemuck::from_bytes(bytes);
    assert_eq!(entry2.type_guid, Guid::EFI_SYSTEM);
    assert_eq!(entry2.first_lba, 2048);
    assert_eq!(entry2.last_lba, 206847);
    assert_eq!(entry2.size_sectors(), 204800);
}

// ---------------------------------------------------------------------------
// DiskPartitionScheme detection
// ---------------------------------------------------------------------------

#[test]
fn detect_mbr_scheme() {
    let mut mbr = MasterBootRecord::default();
    mbr.with_partition_table(|table| {
        table[0] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
    });
    let scheme = detect_scheme_from_mbr(&mbr);
    assert_eq!(scheme, PartitionSchemeType::Mbr);
}

#[test]
fn detect_gpt_scheme() {
    let mbr = MasterBootRecord::protective(1_000_000);
    let scheme = detect_scheme_from_mbr(&mbr);
    assert_eq!(scheme, PartitionSchemeType::Gpt);
}

// ---------------------------------------------------------------------------
// DiskGeometry
// ---------------------------------------------------------------------------

#[test]
fn geometry_standard() {
    let geo = DiskGeometry::standard(2_000_000);
    assert_eq!(geo.block_size, 512);
    assert_eq!(geo.physical_block_size, None);
    assert_eq!(geo.total_blocks, 2_000_000);
    assert_eq!(geo.total_bytes(), 2_000_000 * 512);
}

#[test]
fn geometry_advanced_format() {
    let geo = DiskGeometry::advanced_format(250_000);
    assert_eq!(geo.block_size, 4096);
    assert_eq!(geo.physical_block_size, None);
    assert_eq!(geo.total_bytes(), 250_000 * 4096);
}

#[test]
fn geometry_512e() {
    let geo = DiskGeometry::emulated_512(2_000_000);
    assert_eq!(geo.block_size, 512);
    assert_eq!(geo.physical_block_size, Some(4096));
}

#[test]
fn geometry_alignment() {
    let geo = DiskGeometry::standard(2_000_000);
    let alignment = geo.default_alignment();
    assert!(alignment > 0);

    let aligned_lba = geo.align_up(1, alignment);
    assert!(geo.is_aligned(aligned_lba, alignment));
    assert!(aligned_lba >= 1);

    let already_aligned = geo.align_up(2048, alignment);
    assert_eq!(already_aligned, 2048);
}

#[test]
fn geometry_gpt_usable_lba() {
    let geo = DiskGeometry::standard(1_000_000);
    let first = geo.gpt_first_usable_lba(128, 128);
    let last = geo.gpt_last_usable_lba(128, 128);

    assert!(first > 1);
    assert!(last < 1_000_000);
    assert!(last > first);
}

// ---------------------------------------------------------------------------
// validate_partition_alignment
// ---------------------------------------------------------------------------

#[test]
fn validate_alignment_aligned() {
    let geo = DiskGeometry::standard(1_000_000);
    let alignment = geo.default_alignment();
    let partition = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
    let result = validate_partition_alignment(&partition, &geo, alignment);
    assert!(result.is_ok());
}

#[test]
fn validate_alignment_misaligned() {
    let geo = DiskGeometry::standard(1_000_000);
    let alignment = geo.default_alignment();
    let partition = MbrPartition::new(MbrPartitionType::Fat32, 1, 100);
    let result = validate_partition_alignment(&partition, &geo, alignment);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        PartitionError::MisalignedPartition { .. }
    ));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_partition_table() {
    let table = MbrPartitionTable::new();
    assert_eq!(table.count(), 0);
    assert!(table.partition(0).is_none());
    assert!(table.partition(3).is_none());
}

#[test]
fn max_mbr_partitions() {
    let mut table = MbrPartitionTable::new();
    for i in 0..4 {
        table[i] = MbrPartition::new(
            MbrPartitionType::LinuxNative,
            (i as u32 + 1) * 1_000_000,
            999_999,
        );
    }
    assert_eq!(table.count(), 4);
}

#[test]
fn gpt_unused_entry() {
    let entry = GptPartitionEntry::default();
    assert!(entry.is_unused());
    assert_eq!(entry.size_sectors(), 0);
}

#[test]
fn guid_display_format() {
    let guid = Guid::EFI_SYSTEM;
    let s = format!("{}", guid);
    assert_eq!(s.len(), 36);
    assert_eq!(&s[8..9], "-");
    assert_eq!(&s[13..14], "-");
    assert_eq!(&s[18..19], "-");
    assert_eq!(&s[23..24], "-");
}

#[test]
fn guid_roundtrip_string() {
    let original = Guid::EFI_SYSTEM;
    let s = format!("{}", original);
    let parsed = Guid::from_str(&s).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn mbr_partition_type_roundtrip() {
    let types = [
        MbrPartitionType::Empty,
        MbrPartitionType::Fat12,
        MbrPartitionType::Fat32,
        MbrPartitionType::LinuxNative,
        MbrPartitionType::ProtectiveMbr,
        MbrPartitionType::EfiSystemPartition,
    ];
    for t in &types {
        let byte = t.to_u8();
        let roundtripped = MbrPartitionType::from_u8(byte);
        assert_eq!(*t, roundtripped);
    }
}

#[test]
fn partition_info_trait_empty_partition() {
    let partition = MbrPartition::new(MbrPartitionType::Empty, 0, 0);
    assert_eq!(partition.start_lba(), 0);
    assert_eq!(partition.size_sectors(), 0);
    assert_eq!(partition.size_bytes(), 0);
}

#[test]
fn partition_info_trait_custom_sector_size() {
    let partition = MbrPartition::new(MbrPartitionType::Fat32, 2048, 100);
    assert_eq!(partition.size_bytes(), 100 * 512);
    assert_eq!(partition.size_bytes_with_sector_size(4096), 100 * 4096);
}

// ---------------------------------------------------------------------------
// Hybrid MBR
// ---------------------------------------------------------------------------

#[test]
fn hybrid_mbr_builder() {
    let gpt_entries = [
        GptPartitionEntry::new(Guid::BASIC_DATA, Guid::from_bytes([1; 16]), 2048, 206847),
        GptPartitionEntry::new(
            Guid::LINUX_FILESYSTEM,
            Guid::from_bytes([2; 16]),
            206848,
            1206847,
        ),
    ];

    let mbr = HybridMbrBuilder::new(2_000_000)
        .protective_slot(3)
        .mirror_partition(0, MbrPartitionType::Fat32, false)
        .mirror_partition(1, MbrPartitionType::LinuxNative, false)
        .build(&gpt_entries)
        .unwrap();

    assert!(mbr.has_valid_signature());
    assert!(is_hybrid_mbr(&mbr));
}

// ---------------------------------------------------------------------------
// Display traits
// ---------------------------------------------------------------------------

#[test]
fn partition_error_display() {
    let err = PartitionError::InvalidMbrSignature {
        found: [0x00, 0x00],
    };
    let msg = format!("{}", err);
    assert!(msg.contains("invalid MBR signature"));

    let err = PartitionError::DiskTooSmall {
        required: 1000,
        available: 500,
    };
    let msg = format!("{}", err);
    assert!(msg.contains("disk too small"));
}

#[test]
fn partition_scheme_type_debug() {
    let s = format!("{:?}", PartitionSchemeType::Gpt);
    assert_eq!(s, "Gpt");
}
