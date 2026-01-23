//! Hybrid MBR support for dual BIOS/UEFI bootable disks.
//!
//! A Hybrid MBR is a special MBR that contains both a protective entry (type 0xEE)
//! and regular MBR partition entries that mirror selected GPT partitions. This allows
//! the disk to be bootable on both BIOS and UEFI systems.
//!
//! # Warning
//!
//! Hybrid MBRs are not part of the UEFI specification and can cause issues with
//! some operating systems. Use with caution.

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::error::{PartitionError, Result};
use crate::gpt::GptPartitionEntry;
use crate::mbr::{Chs, MasterBootRecord, MbrPartition, MbrPartitionTable, MbrPartitionType};

/// A partition to be mirrored from GPT to MBR in a hybrid configuration.
#[derive(Debug, Clone, Copy)]
pub struct MirroredPartition {
    /// Index of the GPT partition to mirror (0-based).
    pub gpt_partition_index: u32,
    /// MBR partition type to use.
    pub mbr_type: MbrPartitionType,
    /// Whether to mark this partition as bootable (active).
    pub bootable: bool,
}

impl MirroredPartition {
    /// Creates a new mirrored partition configuration.
    pub const fn new(gpt_index: u32, mbr_type: MbrPartitionType) -> Self {
        Self {
            gpt_partition_index: gpt_index,
            mbr_type,
            bootable: false,
        }
    }

    /// Sets the bootable flag.
    pub const fn with_bootable(mut self, bootable: bool) -> Self {
        self.bootable = bootable;
        self
    }
}

/// Configuration for a Hybrid MBR.
///
/// A Hybrid MBR contains:
/// - A protective MBR entry (type 0xEE) covering either the entire disk or the
///   area not covered by mirrored partitions
/// - Up to 3 mirrored partition entries (MBR can only have 4 entries total)
#[derive(Debug, Clone)]
pub struct HybridMbrConfig {
    /// Index of the MBR slot for the protective partition (0-3).
    /// Usually 0 or 3 (first or last).
    pub protective_slot: usize,
    /// Partitions to mirror from GPT to MBR (max 3).
    #[cfg(feature = "alloc")]
    pub mirrored: Vec<MirroredPartition>,
    /// Partitions to mirror from GPT to MBR (max 3).
    #[cfg(not(feature = "alloc"))]
    pub mirrored: [Option<MirroredPartition>; 3],
    /// Number of mirrored partitions (used in no-alloc mode).
    #[cfg(not(feature = "alloc"))]
    pub mirrored_count: usize,
}

impl Default for HybridMbrConfig {
    fn default() -> Self {
        Self {
            protective_slot: 0,
            #[cfg(feature = "alloc")]
            mirrored: Vec::new(),
            #[cfg(not(feature = "alloc"))]
            mirrored: [None; 3],
            #[cfg(not(feature = "alloc"))]
            mirrored_count: 0,
        }
    }
}

impl HybridMbrConfig {
    /// Creates a new empty Hybrid MBR configuration.
    pub const fn new() -> Self {
        Self {
            protective_slot: 0,
            #[cfg(feature = "alloc")]
            mirrored: Vec::new(),
            #[cfg(not(feature = "alloc"))]
            mirrored: [None, None, None],
            #[cfg(not(feature = "alloc"))]
            mirrored_count: 0,
        }
    }

    /// Sets the protective partition slot index.
    pub const fn with_protective_slot(mut self, slot: usize) -> Self {
        self.protective_slot = slot;
        self
    }

    /// Adds a mirrored partition.
    #[cfg(feature = "alloc")]
    pub fn add_mirrored(mut self, partition: MirroredPartition) -> Self {
        self.mirrored.push(partition);
        self
    }

    /// Adds a mirrored partition.
    #[cfg(not(feature = "alloc"))]
    pub fn add_mirrored(mut self, partition: MirroredPartition) -> Self {
        if self.mirrored_count < 3 {
            self.mirrored[self.mirrored_count] = Some(partition);
            self.mirrored_count += 1;
        }
        self
    }

    /// Returns the number of mirrored partitions.
    #[cfg(feature = "alloc")]
    pub fn mirrored_count(&self) -> usize {
        self.mirrored.len()
    }

    /// Returns the number of mirrored partitions.
    #[cfg(not(feature = "alloc"))]
    pub fn mirrored_count(&self) -> usize {
        self.mirrored_count
    }

    /// Validates the configuration.
    pub fn validate(&self) -> Result<()> {
        if self.protective_slot > 3 {
            return Err(PartitionError::InvalidHybridMbr {
                reason: "protective slot must be 0-3",
            });
        }

        let count = self.mirrored_count();
        if count > 3 {
            return Err(PartitionError::TooManyPartitions {
                max: 3,
                requested: count,
            });
        }

        // Check that mirrored slots don't conflict with protective slot
        #[cfg(feature = "alloc")]
        for (i, _) in self.mirrored.iter().enumerate() {
            let slot = self.calculate_slot(i);
            if slot == self.protective_slot {
                return Err(PartitionError::InvalidHybridMbr {
                    reason: "mirrored partition conflicts with protective slot",
                });
            }
        }

        Ok(())
    }

    /// Calculates the MBR slot for a mirrored partition index.
    fn calculate_slot(&self, mirror_index: usize) -> usize {
        let mut slot = 0;
        let mut count = 0;
        while count <= mirror_index && slot < 4 {
            if slot != self.protective_slot {
                if count == mirror_index {
                    return slot;
                }
                count += 1;
            }
            slot += 1;
        }
        slot
    }
}

/// Builder for creating Hybrid MBRs.
#[derive(Debug)]
pub struct HybridMbrBuilder {
    config: HybridMbrConfig,
    disk_sectors: u64,
}

impl HybridMbrBuilder {
    /// Creates a new builder for a disk with the given size in sectors.
    pub fn new(disk_sectors: u64) -> Self {
        Self {
            config: HybridMbrConfig::new(),
            disk_sectors,
        }
    }

    /// Sets the protective partition slot (0-3).
    pub fn protective_slot(mut self, slot: usize) -> Self {
        self.config.protective_slot = slot;
        self
    }

    /// Adds a GPT partition to mirror in the MBR.
    pub fn mirror_partition(self, gpt_index: u32, mbr_type: MbrPartitionType, bootable: bool) -> Self {
        let partition = MirroredPartition::new(gpt_index, mbr_type).with_bootable(bootable);
        Self {
            config: self.config.add_mirrored(partition),
            ..self
        }
    }

    /// Builds the Hybrid MBR given the GPT partition entries.
    pub fn build(self, gpt_entries: &[GptPartitionEntry]) -> Result<MasterBootRecord> {
        self.config.validate()?;

        let mut mbr = MasterBootRecord::default();
        let mut partition_table = MbrPartitionTable::new();

        // Collect mirrored partitions and their ranges
        #[cfg(feature = "alloc")]
        let mirrored_iter = self.config.mirrored.iter();
        #[cfg(not(feature = "alloc"))]
        let mirrored_iter = self.config.mirrored[..self.config.mirrored_count]
            .iter()
            .filter_map(|p| p.as_ref());

        let mut mirrored_ranges: [(u64, u64); 3] = [(0, 0); 3];
        let mut mirror_count = 0;

        for (i, mirrored) in mirrored_iter.enumerate() {
            let gpt_idx = mirrored.gpt_partition_index as usize;
            if gpt_idx >= gpt_entries.len() {
                return Err(PartitionError::InvalidHybridMbr {
                    reason: "GPT partition index out of bounds",
                });
            }

            let gpt_entry = &gpt_entries[gpt_idx];
            if gpt_entry.is_unused() {
                return Err(PartitionError::InvalidHybridMbr {
                    reason: "referenced GPT partition is unused",
                });
            }

            // Check if partition fits in 32-bit MBR addressing
            if gpt_entry.last_lba > u32::MAX as u64 {
                return Err(PartitionError::InvalidHybridMbr {
                    reason: "GPT partition extends beyond MBR 32-bit limit",
                });
            }

            let slot = self.config.calculate_slot(i);
            let start_lba = gpt_entry.first_lba as u32;
            let sector_count = (gpt_entry.last_lba - gpt_entry.first_lba + 1) as u32;

            partition_table[slot] = MbrPartition {
                boot_indicator: if mirrored.bootable { 0x80 } else { 0x00 },
                start_chs: Chs::new(start_lba),
                part_type: mirrored.mbr_type.to_u8(),
                end_chs: Chs::new(start_lba + sector_count - 1),
                start_lba,
                sector_count,
            };

            mirrored_ranges[mirror_count] = (gpt_entry.first_lba, gpt_entry.last_lba);
            mirror_count += 1;
        }

        // Create protective MBR entry
        // The protective entry should cover areas not covered by mirrored partitions
        // For simplicity, we'll make it cover from sector 1 to the end of the disk
        // (or the first mirrored partition, whichever comes first)
        let protective_end = if mirror_count > 0 {
            // Find the start of the first mirrored partition
            let mut first_start = u64::MAX;
            for i in 0..mirror_count {
                if mirrored_ranges[i].0 < first_start && mirrored_ranges[i].0 > 1 {
                    first_start = mirrored_ranges[i].0;
                }
            }
            if first_start == u64::MAX {
                // All mirrored partitions start at sector 1 or less
                self.disk_sectors.min(u32::MAX as u64) as u32
            } else {
                (first_start - 1).min(u32::MAX as u64) as u32
            }
        } else {
            self.disk_sectors.min(u32::MAX as u64) as u32
        };

        let protective_size = if protective_end > 1 {
            protective_end - 1
        } else {
            1
        };

        partition_table[self.config.protective_slot] = MbrPartition {
            boot_indicator: 0x00,
            start_chs: Chs::new(1),
            part_type: MbrPartitionType::ProtectiveMbr.to_u8(),
            end_chs: Chs::new(protective_end),
            start_lba: 1,
            sector_count: protective_size,
        };

        mbr.partition_table = partition_table;
        Ok(mbr)
    }
}

/// Checks if an MBR appears to be a Hybrid MBR.
///
/// Returns `true` if the MBR contains a protective partition (type 0xEE)
/// and at least one other non-empty partition.
pub fn is_hybrid_mbr(mbr: &MasterBootRecord) -> bool {
    let mut has_protective = false;
    let mut has_other = false;

    let pt = mbr.get_partition_table();
    for partition in &pt.partitions {
        if partition.is_empty() {
            continue;
        }
        if partition.partition_type().is_protective() {
            has_protective = true;
        } else {
            has_other = true;
        }
    }

    has_protective && has_other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpt::Guid;

    #[test]
    fn test_hybrid_mbr_config_validation() {
        let config = HybridMbrConfig::new()
            .with_protective_slot(0)
            .add_mirrored(MirroredPartition::new(0, MbrPartitionType::Fat32));

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_hybrid_mbr_config_too_many_partitions() {
        let mut config = HybridMbrConfig::new();
        #[cfg(feature = "alloc")]
        {
            config.mirrored = alloc::vec![
                MirroredPartition::new(0, MbrPartitionType::Fat32),
                MirroredPartition::new(1, MbrPartitionType::LinuxNative),
                MirroredPartition::new(2, MbrPartitionType::LinuxNative),
                MirroredPartition::new(3, MbrPartitionType::LinuxNative),
            ];
        }

        let result = config.validate();
        assert!(matches!(result, Err(PartitionError::TooManyPartitions { .. })));
    }

    #[test]
    fn test_hybrid_mbr_builder() {
        let gpt_entries = [
            GptPartitionEntry::new(Guid::EFI_SYSTEM, Guid::UNUSED, 2048, 206847),
            GptPartitionEntry::default(),
        ];

        let mbr = HybridMbrBuilder::new(1000000)
            .protective_slot(0)
            .mirror_partition(0, MbrPartitionType::EfiSystemPartition, false)
            .build(&gpt_entries)
            .unwrap();

        assert!(mbr.has_valid_signature());
        assert!(is_hybrid_mbr(&mbr));
    }

    #[test]
    fn test_is_hybrid_mbr() {
        // Pure protective MBR
        let protective = MasterBootRecord::protective(1000000);
        assert!(!is_hybrid_mbr(&protective));

        // Hybrid MBR
        let mut hybrid = protective;
        hybrid.with_partition_table(|pt| {
            pt[1] = MbrPartition::new(MbrPartitionType::Fat32, 2048, 100000);
        });
        assert!(is_hybrid_mbr(&hybrid));
    }
}
