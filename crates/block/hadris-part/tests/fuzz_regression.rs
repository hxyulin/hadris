//! Regression tests for inputs found by the `part_read` fuzz target.

#[cfg(not(feature = "crc"))]
use hadris_part::Error;
use hadris_part::gpt::{GptAttributes, GptPartitionEntry, GptPartitionName, Guid};
use hadris_part::{PartitionTable, PartitionTableReadExt};
use std::io::Cursor;

/// An MBR entry whose start_lba + sector_count exceeds u32::MAX panicked
/// `MbrPartition::end_lba` (attempt to add with overflow).
#[test]
fn mbr_end_lba_saturates_on_corrupt_entry() {
    let mut sector = [0u8; 512];
    // Partition entry 0 at offset 446: type 0x83, start 0xFFFFFF00,
    // count 0xFFFFFFFF.
    sector[446 + 4] = 0x83;
    sector[446 + 8..446 + 12].copy_from_slice(&0xFFFFFF00u32.to_le_bytes());
    sector[446 + 12..446 + 16].copy_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    sector[510] = 0x55;
    sector[511] = 0xAA;

    let table = PartitionTable::read_from(&mut Cursor::new(sector), 512).expect("read MBR");
    let parts = table.partitions();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].end_lba, u32::MAX as u64);
}

/// A GPT entry spanning first_lba=0..=u64::MAX is representable on disk, but
/// its size (last - first + 1) exceeds u64 — must saturate, not overflow.
#[test]
fn gpt_size_sectors_saturates_on_full_range_entry() {
    let mut entry = GptPartitionEntry {
        type_guid: Guid::from_bytes([0xAF; 16]),
        unique_guid: Guid::from_bytes([0xDC; 16]),
        first_lba: 0u64.into(),
        last_lba: u64::MAX.into(),
        attributes: GptAttributes::new(0),
        name: GptPartitionName::default(),
    };
    assert!(!entry.is_unused());
    assert_eq!(entry.size_sectors(), u64::MAX);

    entry.last_lba = (u64::MAX - 1).into();
    assert_eq!(entry.size_sectors(), u64::MAX);
}

/// A 2 KiB image holding a protective MBR and a GPT header at LBA 1 with the
/// given (untrusted) entry-array fields.
#[cfg(not(feature = "crc"))]
fn protective_gpt_image(partition_entry_lba: u64, num_entries: u32, alternate_lba: u64) -> Vec<u8> {
    let mut image = vec![0u8; 2048];
    // Protective MBR at LBA 0: slot 0 type 0xEE, signature 0x55AA.
    image[446 + 4] = 0xEE;
    image[510] = 0x55;
    image[511] = 0xAA;
    // GPT header at LBA 1.
    let h = 512;
    image[h..h + 8].copy_from_slice(b"EFI PART");
    image[h + 24..h + 32].copy_from_slice(&1u64.to_le_bytes()); // my_lba
    image[h + 32..h + 40].copy_from_slice(&alternate_lba.to_le_bytes());
    image[h + 72..h + 80].copy_from_slice(&partition_entry_lba.to_le_bytes());
    image[h + 80..h + 84].copy_from_slice(&num_entries.to_le_bytes());
    image[h + 84..h + 88].copy_from_slice(&128u32.to_le_bytes()); // entry size
    image
}

/// `num_partition_entries` is an untrusted on-disk count: a header declaring
/// billions of entries forced a ~512 GiB up-front allocation (OOM). The entry
/// array is now bounded against the image size and rejected instead.
///
/// Gated off with `crc`: the crafted image carries no valid CRCs, so the read
/// then fails earlier with a header CRC mismatch instead of reaching the
/// bounds check.
#[cfg(not(feature = "crc"))]
#[test]
fn gpt_oversized_entry_array_is_rejected() {
    let image = protective_gpt_image(2, u32::MAX, 34);
    let err = PartitionTable::read_from(&mut Cursor::new(&image), 512).unwrap_err();
    assert!(matches!(err, Error::DiskTooSmall { .. }));
}

/// `partition_entry_lba * block_size` overflowing u64 panicked with
/// "attempt to multiply with overflow"; it must be rejected, not panic.
///
/// See `gpt_oversized_entry_array_is_rejected` for the `crc` gate rationale.
#[cfg(not(feature = "crc"))]
#[test]
fn gpt_entry_lba_times_block_size_overflow_is_rejected() {
    let image = protective_gpt_image(u64::MAX, 1, 34);
    let err = PartitionTable::read_from(&mut Cursor::new(&image), 512).unwrap_err();
    assert!(matches!(err, Error::DiskTooSmall { .. }));
}

/// `alternate_lba * block_size` overflowing u64 panicked while locating the
/// backup GPT header; the read must fail cleanly instead.
///
/// See `gpt_oversized_entry_array_is_rejected` for the `crc` gate rationale.
#[cfg(not(feature = "crc"))]
#[test]
fn gpt_backup_header_lba_overflow_is_rejected() {
    let image = protective_gpt_image(2, 4, u64::MAX);
    let err = PartitionTable::read_from(&mut Cursor::new(&image), 512).unwrap_err();
    assert!(matches!(err, Error::BackupHeaderIo { .. }));
}
