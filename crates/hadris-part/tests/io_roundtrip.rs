//! I/O roundtrip tests for partition table extension traits.

use hadris_io::Cursor;
use hadris_io::ErrorKind;
use hadris_part::{
    DiskPartitionScheme, DiskPartitionSchemeReadExt, MasterBootRecord, MasterBootRecordReadExt,
    MasterBootRecordWriteExt, MbrPartition, MbrPartitionType, PartitionError, PartitionSchemeType,
};
use std::io::Cursor as StdCursor;

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
