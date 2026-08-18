#![no_main]
//! Fuzz the partition table reader: detect and parse MBR/GPT from arbitrary
//! bytes at common logical block sizes. Arbitrary bytes must never
//! panic/abort/OOM.

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

use hadris_part::{PartitionTable, PartitionTableReadExt};

fn drive(data: &[u8]) {
    // `PartitionTable::read_from` detects the scheme: it reads the MBR at
    // LBA 0, then follows the protective/hybrid path into GPT parsing.
    for logical_block_size in [512u32, 4096] {
        let mut cursor = Cursor::new(data);
        let Ok(table) = PartitionTable::read_from(&mut cursor, logical_block_size) else {
            continue;
        };
        for partition in table.partitions() {
            // Oracle: exercise the saturating byte-size conversions on
            // fuzz-controlled sector counts (no assertion beyond not panicking).
            let _ = (
                partition.index,
                partition.start_lba,
                partition.end_lba,
                partition.size_sectors,
                partition.bootable,
                partition.size_bytes(),
                partition.size_bytes_with_sector_size(logical_block_size),
            );
        }
    }
}

fuzz_target!(|data: &[u8]| {
    drive(data);
});
