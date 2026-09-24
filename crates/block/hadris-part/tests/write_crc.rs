//! A written GPT must carry valid checksums whatever other features are on.
#![cfg(all(feature = "std", feature = "sync", feature = "write"))]

use hadris_part::{GptDisk, GptDiskWriteExt, GptPartitionEntry, Guid};
use std::io::Cursor;

const SECTOR: usize = 512;

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn le_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn assert_header_crcs(image: &[u8], lba: usize) {
    let sector = &image[lba * SECTOR..(lba + 1) * SECTOR];
    assert_eq!(&sector[..8], b"EFI PART");
    let header_size = le_u32(sector, 12) as usize;
    let mut header = sector[..header_size].to_vec();
    header[16..20].fill(0);
    assert_eq!(
        le_u32(sector, 16),
        crc32(&header),
        "header CRC at LBA {lba}"
    );

    let entries_lba = le_u64(sector, 72) as usize;
    let entries_len = le_u32(sector, 80) as usize * le_u32(sector, 84) as usize;
    let entries = &image[entries_lba * SECTOR..entries_lba * SECTOR + entries_len];
    assert_eq!(
        le_u32(sector, 88),
        crc32(entries),
        "entry array CRC at LBA {lba}"
    );
}

#[test]
fn written_gpt_has_valid_crcs() {
    let sectors = 2048u64;
    let mut disk = GptDisk::new(sectors, SECTOR as u32);
    disk.add_partition(GptPartitionEntry::new(
        Guid::LINUX_FILESYSTEM,
        Guid::from_bytes([7; 16]),
        64,
        1000,
    ))
    .unwrap();

    let mut cursor = Cursor::new(vec![0u8; sectors as usize * SECTOR]);
    disk.write_to(&mut cursor).unwrap();
    let image = cursor.into_inner();

    assert_header_crcs(&image, 1);
    assert_header_crcs(&image, sectors as usize - 1);
}
