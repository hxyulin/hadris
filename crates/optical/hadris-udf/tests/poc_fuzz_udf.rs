//! Regression tests for fuzzer-found read-path bugs.
#![cfg(all(feature = "std", feature = "sync", feature = "write"))]

use std::io::Cursor;

use hadris_udf::UdfVolume;
use hadris_udf::sync::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};

fn crc16_itu(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        let mut x = ((crc >> 8) ^ (byte as u16)) & 0xFF;
        x ^= x >> 4;
        crc = (crc << 8) ^ (x << 12) ^ (x << 5) ^ x;
    }
    crc
}

/// A corrupt File Entry whose `extended_attributes_length` is odd shifts the
/// allocation descriptors to an unaligned offset inside the ICB buffer.
/// Parsing them must not panic on the misaligned pointer cast.
#[test]
fn read_file_tolerates_unaligned_allocation_descriptors() {
    let mut root = SimpleDir::root();
    root.add_file(SimpleFile::new("hello.txt", b"hello world".to_vec()));
    let output = UdfWriter::create(Cursor::new(Vec::new()), &root, UdfWriteOptions::default())
        .expect("format");
    let mut image = output.target.into_inner();

    let volume = UdfVolume::open(Cursor::new(image.clone())).expect("mount");
    let partition_start = volume.info().partition_start as usize;
    let root_dir = volume.root_dir().expect("root dir");
    let entry = root_dir
        .entries()
        .find(|e| e.name == "hello.txt")
        .expect("file entry");
    let fe_sector = partition_start + entry.icb.logical_block_num as usize;
    drop(volume);

    let base = fe_sector * 2048;
    // File Entry tag identifier (261) little-endian
    assert_eq!([image[base], image[base + 1]], [0x05, 0x01]);

    // extended_attributes_length (File Entry offset 168) becomes odd, moving
    // the allocation descriptors to an unaligned offset without changing
    // allocation_descriptors_length.
    image[base + 168..base + 172].copy_from_slice(&1u32.to_le_bytes());

    let crc_length = u16::from_le_bytes([image[base + 10], image[base + 11]]) as usize;
    let crc = crc16_itu(&image[base + 16..base + 16 + crc_length]);
    image[base + 8..base + 10].copy_from_slice(&crc.to_le_bytes());

    // The tag checksum covers the CRC field, so it is updated last.
    let tag_sum = (0..16)
        .filter(|&i| i != 4)
        .fold(0u8, |sum, i| sum.wrapping_add(image[base + i]));
    image[base + 4] = tag_sum;

    let volume = UdfVolume::open(Cursor::new(image)).expect("remount");
    let root_dir = volume.root_dir().expect("root dir");
    let entry = root_dir
        .entries()
        .find(|e| e.name == "hello.txt")
        .expect("file entry");
    let _ = volume.read_file(entry);
}
