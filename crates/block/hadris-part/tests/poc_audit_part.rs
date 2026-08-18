//! Regression tests for arithmetic-overflow/robustness bugs found by a manual
//! audit of hadris-part. Crafted images and extreme geometry values must
//! produce errors or saturated values, never panics.
//!
//! Tests that parse a crafted GPT image are gated off with `crc`: the image
//! carries no valid checksums, so with `crc` enabled the read fails earlier
//! with a CRC mismatch before reaching the behavior under test.

#![cfg(all(feature = "sync", feature = "alloc", feature = "read"))]

use hadris_io::Cursor;
#[cfg(not(feature = "crc"))]
use hadris_part::gpt::{GptPartitionEntry, Guid};
#[cfg(not(feature = "crc"))]
use hadris_part::hybrid::HybridMbrBuilder;
use hadris_part::mbr::{MasterBootRecord, MbrPartition};
use hadris_part::scheme::GptDisk;
use hadris_part::{
    DiskGeometry, Error, MasterBootRecordReadExt, MbrPartitionType, PartitionTable,
    PartitionTableReadExt,
};

const BLOCK: usize = 512;

/// Builds a 100-sector image whose GPT at LBA 1 (with a matching backup
/// header at LBA 99) contains one entry spanning `first_lba..=last_lba`.
#[cfg(not(feature = "crc"))]
fn gpt_image(first_lba: u64, last_lba: u64) -> Vec<u8> {
    let sectors = 100u64;
    let mut image = vec![0u8; sectors as usize * BLOCK];
    let backup_lba = sectors - 1;
    let entry_array_blocks = (128u64 * 128).div_ceil(BLOCK as u64); // 32

    image[446 + 4] = 0xEE; // protective MBR slot
    image[510] = 0x55;
    image[511] = 0xAA;

    let write_header =
        |image: &mut [u8], at_lba: u64, my_lba: u64, alt_lba: u64, entries_lba: u64| {
            let h = at_lba as usize * BLOCK;
            image[h..h + 8].copy_from_slice(b"EFI PART");
            image[h + 8..h + 12].copy_from_slice(&0x10000u32.to_le_bytes()); // revision
            image[h + 12..h + 16].copy_from_slice(&92u32.to_le_bytes()); // header size
            image[h + 24..h + 32].copy_from_slice(&my_lba.to_le_bytes());
            image[h + 32..h + 40].copy_from_slice(&alt_lba.to_le_bytes());
            image[h + 40..h + 48].copy_from_slice(&34u64.to_le_bytes()); // first usable
            image[h + 48..h + 56].copy_from_slice(&66u64.to_le_bytes()); // last usable
            image[h + 72..h + 80].copy_from_slice(&entries_lba.to_le_bytes());
            image[h + 80..h + 84].copy_from_slice(&128u32.to_le_bytes()); // num entries
            image[h + 84..h + 88].copy_from_slice(&128u32.to_le_bytes()); // entry size
        };
    write_header(&mut image, 1, 1, backup_lba, 2);
    write_header(
        &mut image,
        backup_lba,
        backup_lba,
        1,
        backup_lba - entry_array_blocks,
    );

    let e = 2 * BLOCK;
    image[e..e + 16].copy_from_slice(&[0xAF; 16]); // type GUID (non-zero)
    image[e + 32..e + 40].copy_from_slice(&first_lba.to_le_bytes());
    image[e + 40..e + 48].copy_from_slice(&last_lba.to_le_bytes());
    image
}

/// A GPT entry spanning first_lba=0..=u64::MAX saturates `size_sectors` to
/// u64::MAX; the byte-size conversions on `PartitionInfo` must saturate too
/// instead of overflowing.
#[cfg(not(feature = "crc"))]
#[test]
fn partition_info_size_bytes_saturates_on_full_range_entry() {
    let image = gpt_image(0, u64::MAX);
    let table =
        PartitionTable::read_from(&mut Cursor::new(&image), BLOCK as u32).expect("read GPT");
    let parts = table.partitions();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].size_sectors, u64::MAX);
    assert_eq!(parts[0].size_bytes(), u64::MAX);
    assert_eq!(parts[0].size_bytes_with_sector_size(4096), u64::MAX);
}

/// A zeroed CHS (sector component 0) is invalid: CHS sectors are 1-based.
/// `Chs::as_lba` must return the out-of-range sentinel instead of underflowing.
#[test]
fn chs_zero_sector_is_out_of_range() {
    let mut sector = [0u8; 512];
    sector[446 + 4] = 0x83; // non-empty type; CHS fields stay all-zero
    sector[510] = 0x55;
    sector[511] = 0xAA;
    let mbr = MasterBootRecord::read_from(&mut Cursor::new(&sector)).expect("read MBR");
    let pt = mbr.get_partition_table();
    assert_eq!(pt.partitions[0].start_chs.as_lba(), u32::MAX);
}

/// Mirroring a GPT entry with first_lba > last_lba underflowed the sector
/// count; it must be rejected.
#[cfg(not(feature = "crc"))]
#[test]
fn hybrid_build_rejects_inverted_lba_range() {
    let image = gpt_image(2048, 100);
    let table =
        PartitionTable::read_from(&mut Cursor::new(&image), BLOCK as u32).expect("read GPT");
    let PartitionTable::Gpt { gpt, .. } = &table else {
        panic!("expected GPT");
    };
    let err = HybridMbrBuilder::new(100)
        .mirror_partition(0, MbrPartitionType::LinuxNative, false)
        .build(&gpt.entries)
        .unwrap_err();
    assert!(matches!(err, Error::InvalidHybridMbr { .. }));
}

/// `GptDisk::new` must not divide by zero for block sizes smaller than one
/// partition entry, nor underflow the usable range on tiny disks.
#[test]
fn gpt_disk_new_tolerates_extreme_geometry() {
    let small_block = GptDisk::new(1_000_000, 64);
    assert_eq!(
        small_block.primary_header.last_usable_lba.to_ne(),
        1_000_000 - 2 - 128
    );

    let tiny_disk = GptDisk::new(10, BLOCK as u32);
    assert_eq!(tiny_disk.primary_header.last_usable_lba.to_ne(), 0);
    assert_eq!(tiny_disk.primary_header.alternate_lba.to_ne(), 9);
}

/// `GptDisk::read_from` must reject a block size too small to hold a GPT
/// header instead of dividing by zero.
#[test]
fn read_rejects_zero_block_size() {
    // With block_size 0 the "LBA 1" header would be read at offset 0, so the
    // MBR bootstrap doubles as the header.
    let mut image = vec![0u8; 4096];
    image[0..8].copy_from_slice(b"EFI PART");
    image[80..84].copy_from_slice(&1u32.to_le_bytes()); // num entries
    image[84..88].copy_from_slice(&128u32.to_le_bytes()); // entry size
    image[446 + 4] = 0xEE; // protective MBR slot
    image[510] = 0x55;
    image[511] = 0xAA;
    let err = PartitionTable::read_from(&mut Cursor::new(&image), 0).unwrap_err();
    assert!(matches!(err, Error::InvalidBlockSize { size: 0, .. }));
}

/// The GPT write path must reject an entry-array LBA whose byte offset is not
/// representable instead of overflowing the multiplication.
#[cfg(feature = "write")]
#[test]
fn gpt_write_rejects_unrepresentable_entry_lba() {
    use endian_num::Le;
    use hadris_part::GptDiskWriteExt;

    let mut gpt = GptDisk::new(204800, BLOCK as u32);
    gpt.primary_header.partition_entry_lba = Le::<u64>::from_ne(u64::MAX / 512 + 1);
    let mut out = std::io::Cursor::new(Vec::new());
    let err = gpt.write_to(&mut out).unwrap_err();
    assert!(matches!(err, Error::Io(_)));
}

/// `MbrPartition::new` must saturate the end LBA near the 32-bit limit
/// instead of overflowing.
#[test]
fn mbr_partition_new_saturates_end_lba() {
    let partition = MbrPartition::new(MbrPartitionType::Fat32, u32::MAX, 2);
    assert_eq!(partition.end_lba(), u32::MAX);
}

/// `DiskGeometry::align_up` must saturate for LBAs near u64::MAX, and
/// `gpt_last_usable_lba` must not underflow on tiny disks.
#[test]
fn geometry_helpers_saturate_on_extreme_values() {
    let geom = DiskGeometry::standard(1_000_000);
    assert_eq!(geom.align_up(u64::MAX, 2048), u64::MAX & !2047);

    let tiny = DiskGeometry::standard(10);
    assert_eq!(tiny.gpt_last_usable_lba(128, 128), 0);
    assert_eq!(tiny.gpt_last_usable_lba_aligned(128, 128), 0);
}

/// Control: the crafted image parses fine when entry LBAs are sane, proving
/// the harness above reaches the exercised code paths.
#[cfg(not(feature = "crc"))]
#[test]
fn control_sane_image_parses() {
    let image = gpt_image(34, 66);
    let table =
        PartitionTable::read_from(&mut Cursor::new(&image), BLOCK as u32).expect("read GPT");
    let parts = table.partitions();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].start_lba, 34);
    assert_eq!(parts[0].size_bytes(), 33 * 512);

    let entry = GptPartitionEntry::new(Guid::EFI_SYSTEM, Guid::UNUSED, 2048, 206847);
    assert_eq!(entry.size_sectors(), 204800);
}
