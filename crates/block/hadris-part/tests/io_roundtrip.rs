//! I/O roundtrip tests for partition table extension traits.

use hadris_io::Cursor;
use hadris_io::ErrorKind;
use hadris_part::hybrid::HybridMbrBuilder;
use hadris_part::{
    DiskPartitionScheme, DiskPartitionSchemeReadExt, DiskPartitionSchemeWriteExt,
    GptPartitionEntry, Guid, MasterBootRecord, MasterBootRecordReadExt, MasterBootRecordWriteExt,
    MbrPartition, MbrPartitionType, PartitionError, PartitionSchemeType,
};
use std::io::Cursor as StdCursor;
use std::io::{Seek, SeekFrom};

#[test]
fn mbr_read_write_roundtrip() {
    let mut mbr = MasterBootRecord::default();
    mbr.with_partition_table(|table| {
        table[0] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
        table[1] = MbrPartition::new(MbrPartitionType::LinuxNative, 206848, 1000000);
    });
    assert!(mbr.has_valid_signature());

    let mut owned = vec![0u8; 512];
    {
        let mut cursor = StdCursor::new(&mut owned[..]);
        mbr.write_to(&mut cursor).unwrap();
    }

    let mbr2 = MasterBootRecord::read_from(&mut Cursor::new(&owned[..])).unwrap();
    assert!(mbr2.has_valid_signature());
    let t2 = mbr2.get_partition_table();
    assert_eq!(t2.count(), 2);
    assert_eq!(t2[0].start_lba.to_ne() as u64, 2048);
    assert_eq!(t2[0].sector_count.to_ne() as u64, 204800);
    assert_eq!(t2[1].start_lba.to_ne() as u64, 206848);
}

#[test]
fn mbr_read_from_short_buffer_is_io_error() {
    let mut cursor = Cursor::new(&[0u8; 64]);
    let err = MasterBootRecord::read_from(&mut cursor).unwrap_err();
    match err {
        PartitionError::Io(e) => assert_eq!(e.kind(), ErrorKind::UnexpectedEof),
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn disk_partition_scheme_reads_mbr() {
    let mut mbr = MasterBootRecord::default();
    mbr.with_partition_table(|table| {
        table[0] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
    });

    let mut disk = vec![0u8; 512];
    disk.copy_from_slice(bytemuck::bytes_of(&mbr));

    let mut cursor = StdCursor::new(&disk[..]);
    let scheme = DiskPartitionScheme::read_from(&mut cursor, 512).unwrap();
    assert_eq!(scheme.scheme_type(), PartitionSchemeType::Mbr);
    let parts = scheme.partitions();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].start_lba, 2048);
    assert_eq!(parts[0].size_sectors, 204800);
}

#[test]
fn v2_partition_table_detect_and_open_restore_clear_lifecycle() {
    let mut mbr = MasterBootRecord::default();
    mbr.with_partition_table(|table| {
        table[0] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 204800);
    });
    let disk = bytemuck::bytes_of(&mbr).to_vec();
    let mut cursor = StdCursor::new(disk);
    cursor.seek(SeekFrom::Start(19)).unwrap();

    let kind = hadris_part::sync::partition_table::detect(&mut cursor).unwrap();
    assert_eq!(kind, PartitionSchemeType::Mbr);
    assert_eq!(cursor.stream_position().unwrap(), 19);

    let table: hadris_part::PartitionTable =
        hadris_part::sync::partition_table::open(&mut cursor, 512).unwrap();
    assert_eq!(table.scheme_type(), PartitionSchemeType::Mbr);
}

fn populated_gpt() -> DiskPartitionScheme {
    let mut scheme = DiskPartitionScheme::new_gpt(4096, 512);
    let DiskPartitionScheme::Gpt { gpt, .. } = &mut scheme else {
        unreachable!();
    };
    gpt.add_partition(GptPartitionEntry::new(
        Guid::EFI_SYSTEM,
        Guid::from_bytes([0x11; 16]),
        40,
        399,
    ))
    .unwrap();
    gpt.add_partition(GptPartitionEntry::new(
        Guid::LINUX_FILESYSTEM,
        Guid::from_bytes([0x22; 16]),
        400,
        2047,
    ))
    .unwrap();
    scheme
}

#[test]
fn gpt_scheme_sync_write_open_and_detect_roundtrip() {
    let scheme = populated_gpt();
    let mut disk = StdCursor::new(vec![0_u8; 4096 * 512]);
    scheme.write_to(&mut disk).unwrap();

    disk.seek(SeekFrom::Start(73)).unwrap();
    assert_eq!(
        hadris_part::sync::partition_table::detect(&mut disk).unwrap(),
        PartitionSchemeType::Gpt
    );
    assert_eq!(disk.stream_position().unwrap(), 73);

    let opened = hadris_part::sync::partition_table::open(&mut disk, 512).unwrap();
    assert_eq!(opened.scheme_type(), PartitionSchemeType::Gpt);
    opened.validate().unwrap();
    let partitions = opened.partitions();
    assert_eq!(partitions.len(), 2);
    assert_eq!((partitions[0].start_lba, partitions[0].end_lba), (40, 399));
    assert_eq!(partitions[1].size_sectors, 1648);
}

#[test]
fn sync_open_rejects_truncated_and_corrupt_gpt() {
    let scheme = populated_gpt();
    let mut complete = StdCursor::new(vec![0_u8; 4096 * 512]);
    scheme.write_to(&mut complete).unwrap();
    let complete = complete.into_inner();

    let mut truncated = StdCursor::new(complete[..512].to_vec());
    assert!(matches!(
        hadris_part::sync::partition_table::open(&mut truncated, 512),
        Err(PartitionError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof
    ));

    let mut corrupt = complete;
    corrupt[512..520].copy_from_slice(b"NOT GPT!");
    let mut corrupt = StdCursor::new(corrupt);
    assert!(matches!(
        hadris_part::sync::partition_table::open(&mut corrupt, 512),
        Err(PartitionError::InvalidGptSignature { .. })
    ));
}

#[test]
fn hybrid_scheme_sync_write_open_roundtrip() {
    let DiskPartitionScheme::Gpt { gpt, .. } = populated_gpt() else {
        unreachable!();
    };
    let hybrid_mbr = HybridMbrBuilder::new(4096)
        .protective_slot(3)
        .mirror_partition(0, MbrPartitionType::EfiSystemPartition, true)
        .build(&gpt.entries)
        .unwrap();
    let scheme = DiskPartitionScheme::Hybrid { hybrid_mbr, gpt };

    let mut disk = StdCursor::new(vec![0_u8; 4096 * 512]);
    scheme.write_to(&mut disk).unwrap();
    let opened = hadris_part::sync::partition_table::open(&mut disk, 512).unwrap();
    assert_eq!(opened.scheme_type(), PartitionSchemeType::Hybrid);
    opened.validate().unwrap();
    assert_eq!(opened.partitions().len(), 2);
}
