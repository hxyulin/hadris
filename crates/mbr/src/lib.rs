#![no_std]

use core::fmt::Debug;

#[derive(Debug, Clone, Copy, Default)]
pub struct PartitionTable {
    pub partitions: [Partition; 4],
}

impl PartitionTable {
    pub fn to_le_bytes(self) -> [u8; 64] {
        let mut bytes = [0u8; 64];
        bytes[0..16].copy_from_slice(&self.partitions[0].to_le_bytes());
        bytes[16..32].copy_from_slice(&self.partitions[1].to_le_bytes());
        bytes[32..48].copy_from_slice(&self.partitions[2].to_le_bytes());
        bytes[48..64].copy_from_slice(&self.partitions[3].to_le_bytes());
        bytes
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum PartitionType {
    Fat32 = 0x0B,
    Fat32Lba = 0x0C,
    Fat16 = 0x0E,
}

#[derive(Debug, Clone, Copy)]
pub struct Partition {
    bootable: bool,
    start_chs: [u8; 3],
    kind: PartitionType,
    end_chs: [u8; 3],
    start_lba: u32,
    sector_count: u32,
}

impl Default for Partition {
    fn default() -> Self {
        Self {
            bootable: false,
            start_chs: [0; 3],
            kind: PartitionType::Fat32,
            end_chs: [0; 3],
            start_lba: 0,
            sector_count: 0,
        }
    }
}

impl Partition {
    pub fn new_lba(start_lba: u32, sector_count: u32, bootable: bool) -> Self {
        Self {
            bootable,
            start_chs: [0; 3],
            kind: PartitionType::Fat32Lba,
            end_chs: [0; 3],
            start_lba,
            sector_count,
        }
    }

    fn to_le_bytes(self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[0] = self.bootable as u8;
        bytes[1..4].copy_from_slice(&self.start_chs);
        bytes[4] = self.kind as u8;
        bytes[5..8].copy_from_slice(&self.end_chs);
        bytes[8..12].copy_from_slice(&self.start_lba.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.sector_count.to_le_bytes());
        bytes
    }
}
