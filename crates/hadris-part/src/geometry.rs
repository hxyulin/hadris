//! Disk geometry and alignment utilities.
//!
//! This module provides types and utilities for working with disk geometry
//! and partition alignment, including support for modern Advanced Format disks.

use crate::PartitionInfoTrait;
use crate::error::{PartitionError, Result};

/// Disk geometry information.
///
/// Contains information about the disk's logical and physical block sizes,
/// total capacity, and provides utilities for calculating alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskGeometry {
    /// Logical block (sector) size in bytes.
    pub block_size: u32,
    /// Total number of blocks on the disk.
    pub total_blocks: u64,
    /// Physical sector size (for alignment), if different from logical.
    ///
    /// Modern Advanced Format drives have 4096-byte physical sectors
    /// but may present 512-byte logical sectors for compatibility.
    pub physical_block_size: Option<u32>,
}

impl DiskGeometry {
    /// Standard logical block size (512 bytes).
    pub const STANDARD_BLOCK_SIZE: u32 = 512;

    /// Advanced Format block size (4096 bytes).
    pub const ADVANCED_FORMAT_BLOCK_SIZE: u32 = 4096;

    /// Default partition alignment (1 MiB in bytes).
    ///
    /// 1 MiB alignment is the modern standard for optimal performance
    /// on both traditional and Advanced Format drives.
    pub const DEFAULT_ALIGNMENT_BYTES: u64 = 1024 * 1024;

    /// Creates geometry for a standard 512-byte sector disk.
    ///
    /// # Arguments
    ///
    /// * `total_blocks` - Total number of 512-byte sectors on the disk
    pub const fn standard(total_blocks: u64) -> Self {
        Self {
            block_size: Self::STANDARD_BLOCK_SIZE,
            total_blocks,
            physical_block_size: None,
        }
    }

    /// Creates geometry for a 4K sector disk (Advanced Format).
    ///
    /// # Arguments
    ///
    /// * `total_blocks` - Total number of 4096-byte sectors on the disk
    pub const fn advanced_format(total_blocks: u64) -> Self {
        Self {
            block_size: Self::ADVANCED_FORMAT_BLOCK_SIZE,
            total_blocks,
            physical_block_size: None,
        }
    }

    /// Creates geometry for a 512e disk (512-byte logical, 4K physical).
    ///
    /// These disks present 512-byte logical sectors for compatibility
    /// but have 4096-byte physical sectors internally.
    ///
    /// # Arguments
    ///
    /// * `total_blocks` - Total number of 512-byte logical sectors
    pub const fn emulated_512(total_blocks: u64) -> Self {
        Self {
            block_size: Self::STANDARD_BLOCK_SIZE,
            total_blocks,
            physical_block_size: Some(Self::ADVANCED_FORMAT_BLOCK_SIZE),
        }
    }

    /// Creates a new DiskGeometry with custom parameters.
    pub const fn new(block_size: u32, total_blocks: u64, physical_block_size: Option<u32>) -> Self {
        Self {
            block_size,
            total_blocks,
            physical_block_size,
        }
    }

    /// Returns the total disk size in bytes.
    pub const fn total_bytes(&self) -> u64 {
        self.total_blocks * self.block_size as u64
    }

    /// Returns the effective alignment boundary in bytes.
    ///
    /// This is the physical block size if set, otherwise the logical block size.
    pub const fn alignment_boundary(&self) -> u32 {
        match self.physical_block_size {
            Some(size) => size,
            None => self.block_size,
        }
    }

    /// Returns the default partition alignment in sectors.
    ///
    /// This is 1 MiB expressed in sectors for the current block size.
    /// For 512-byte sectors, this is 2048 sectors.
    /// For 4096-byte sectors, this is 256 sectors.
    pub const fn default_alignment(&self) -> u64 {
        Self::DEFAULT_ALIGNMENT_BYTES / self.block_size as u64
    }

    /// Aligns an LBA up to the next alignment boundary.
    ///
    /// # Arguments
    ///
    /// * `lba` - The LBA to align
    /// * `alignment_sectors` - The alignment boundary in sectors
    ///
    /// # Returns
    ///
    /// The LBA rounded up to the next multiple of `alignment_sectors`.
    pub const fn align_up(&self, lba: u64, alignment_sectors: u64) -> u64 {
        if alignment_sectors == 0 {
            return lba;
        }
        let mask = alignment_sectors - 1;
        (lba + mask) & !mask
    }

    /// Aligns an LBA down to the previous alignment boundary.
    ///
    /// # Arguments
    ///
    /// * `lba` - The LBA to align
    /// * `alignment_sectors` - The alignment boundary in sectors
    ///
    /// # Returns
    ///
    /// The LBA rounded down to the previous multiple of `alignment_sectors`.
    pub const fn align_down(&self, lba: u64, alignment_sectors: u64) -> u64 {
        if alignment_sectors == 0 {
            return lba;
        }
        let mask = alignment_sectors - 1;
        lba & !mask
    }

    /// Checks if an LBA is properly aligned.
    ///
    /// # Arguments
    ///
    /// * `lba` - The LBA to check
    /// * `alignment_sectors` - The alignment boundary in sectors
    pub const fn is_aligned(&self, lba: u64, alignment_sectors: u64) -> bool {
        if alignment_sectors == 0 {
            return true;
        }
        lba % alignment_sectors == 0
    }

    /// Calculates the first usable LBA for GPT partitions.
    ///
    /// This accounts for:
    /// - MBR (1 sector)
    /// - GPT header (1 sector)
    /// - Partition entry array
    ///
    /// # Arguments
    ///
    /// * `num_entries` - Number of partition entries (typically 128)
    /// * `entry_size` - Size of each entry in bytes (typically 128)
    pub const fn gpt_first_usable_lba(&self, num_entries: u32, entry_size: u32) -> u64 {
        let entry_bytes = num_entries as u64 * entry_size as u64;
        let entry_sectors = (entry_bytes + self.block_size as u64 - 1) / self.block_size as u64;
        // MBR (LBA 0) + GPT header (LBA 1) + partition entries
        2 + entry_sectors
    }

    /// Calculates the first usable LBA aligned to the default alignment.
    ///
    /// # Arguments
    ///
    /// * `num_entries` - Number of partition entries (typically 128)
    /// * `entry_size` - Size of each entry in bytes (typically 128)
    pub const fn gpt_first_usable_lba_aligned(&self, num_entries: u32, entry_size: u32) -> u64 {
        let first = self.gpt_first_usable_lba(num_entries, entry_size);
        self.align_up(first, self.default_alignment())
    }

    /// Calculates the last usable LBA for GPT partitions.
    ///
    /// This accounts for:
    /// - Backup partition entry array
    /// - Backup GPT header (1 sector at the last LBA)
    ///
    /// # Arguments
    ///
    /// * `num_entries` - Number of partition entries (typically 128)
    /// * `entry_size` - Size of each entry in bytes (typically 128)
    pub const fn gpt_last_usable_lba(&self, num_entries: u32, entry_size: u32) -> u64 {
        let entry_bytes = num_entries as u64 * entry_size as u64;
        let entry_sectors = (entry_bytes + self.block_size as u64 - 1) / self.block_size as u64;
        // Last LBA is total_blocks - 1
        // Backup header at last LBA, backup entries before that
        self.total_blocks - 1 - entry_sectors - 1
    }

    /// Calculates the last usable LBA aligned down to the default alignment.
    ///
    /// # Arguments
    ///
    /// * `num_entries` - Number of partition entries (typically 128)
    /// * `entry_size` - Size of each entry in bytes (typically 128)
    pub const fn gpt_last_usable_lba_aligned(&self, num_entries: u32, entry_size: u32) -> u64 {
        let last = self.gpt_last_usable_lba(num_entries, entry_size);
        // Align down and subtract 1 to stay within bounds
        self.align_down(last + 1, self.default_alignment()) - 1
    }

    /// Calculates the usable space in sectors for GPT partitions.
    ///
    /// # Arguments
    ///
    /// * `num_entries` - Number of partition entries (typically 128)
    /// * `entry_size` - Size of each entry in bytes (typically 128)
    pub const fn gpt_usable_sectors(&self, num_entries: u32, entry_size: u32) -> u64 {
        let first = self.gpt_first_usable_lba(num_entries, entry_size);
        let last = self.gpt_last_usable_lba(num_entries, entry_size);
        if last > first { last - first + 1 } else { 0 }
    }
}

/// Validates that a partition is properly aligned.
///
/// # Arguments
///
/// * `partition` - The partition to validate
/// * `geometry` - The disk geometry
/// * `alignment` - The required alignment in sectors
///
/// # Errors
///
/// Returns `PartitionError::MisalignedPartition` if the partition is not aligned.
pub fn validate_partition_alignment<P: PartitionInfoTrait>(
    partition: &P,
    geometry: &DiskGeometry,
    alignment: u64,
) -> Result<()> {
    if !geometry.is_aligned(partition.start_lba(), alignment) {
        return Err(PartitionError::MisalignedPartition {
            lba: partition.start_lba(),
            required_alignment: alignment,
        });
    }
    Ok(())
}

/// Validates that all partitions in a slice are properly aligned.
///
/// # Arguments
///
/// * `partitions` - The partitions to validate
/// * `geometry` - The disk geometry
/// * `alignment` - The required alignment in sectors
///
/// # Errors
///
/// Returns `PartitionError::MisalignedPartition` for the first misaligned partition found.
pub fn validate_all_partitions_aligned<P: PartitionInfoTrait>(
    partitions: &[P],
    geometry: &DiskGeometry,
    alignment: u64,
) -> Result<()> {
    for partition in partitions {
        validate_partition_alignment(partition, geometry, alignment)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_geometry() {
        let geom = DiskGeometry::standard(2_097_152); // 1 GiB
        assert_eq!(geom.block_size, 512);
        assert_eq!(geom.total_bytes(), 1024 * 1024 * 1024);
        assert_eq!(geom.default_alignment(), 2048); // 1 MiB / 512 = 2048
    }

    #[test]
    fn test_advanced_format_geometry() {
        let geom = DiskGeometry::advanced_format(262_144); // 1 GiB
        assert_eq!(geom.block_size, 4096);
        assert_eq!(geom.total_bytes(), 1024 * 1024 * 1024);
        assert_eq!(geom.default_alignment(), 256); // 1 MiB / 4096 = 256
    }

    #[test]
    fn test_emulated_512_geometry() {
        let geom = DiskGeometry::emulated_512(2_097_152);
        assert_eq!(geom.block_size, 512);
        assert_eq!(geom.physical_block_size, Some(4096));
        assert_eq!(geom.alignment_boundary(), 4096);
    }

    #[test]
    fn test_align_up() {
        let geom = DiskGeometry::standard(1000000);

        assert_eq!(geom.align_up(0, 2048), 0);
        assert_eq!(geom.align_up(1, 2048), 2048);
        assert_eq!(geom.align_up(2047, 2048), 2048);
        assert_eq!(geom.align_up(2048, 2048), 2048);
        assert_eq!(geom.align_up(2049, 2048), 4096);
    }

    #[test]
    fn test_align_down() {
        let geom = DiskGeometry::standard(1000000);

        assert_eq!(geom.align_down(0, 2048), 0);
        assert_eq!(geom.align_down(1, 2048), 0);
        assert_eq!(geom.align_down(2047, 2048), 0);
        assert_eq!(geom.align_down(2048, 2048), 2048);
        assert_eq!(geom.align_down(4095, 2048), 2048);
        assert_eq!(geom.align_down(4096, 2048), 4096);
    }

    #[test]
    fn test_is_aligned() {
        let geom = DiskGeometry::standard(1000000);

        assert!(geom.is_aligned(0, 2048));
        assert!(!geom.is_aligned(1, 2048));
        assert!(geom.is_aligned(2048, 2048));
        assert!(geom.is_aligned(4096, 2048));
        assert!(!geom.is_aligned(4097, 2048));
    }

    #[test]
    fn test_gpt_usable_lba() {
        // 100 MiB disk with 512-byte sectors
        let geom = DiskGeometry::standard(204800);

        // 128 entries * 128 bytes = 16384 bytes = 32 sectors
        let first = geom.gpt_first_usable_lba(128, 128);
        let last = geom.gpt_last_usable_lba(128, 128);

        // First usable: 2 (MBR + header) + 32 (entries) = 34
        assert_eq!(first, 34);

        // Last usable: total - 1 - 32 (backup entries) - 1 (backup header)
        // = 204800 - 1 - 32 - 1 = 204766
        assert_eq!(last, 204766);

        // First aligned to 1 MiB (2048 sectors)
        let first_aligned = geom.gpt_first_usable_lba_aligned(128, 128);
        assert_eq!(first_aligned, 2048);
    }

    #[test]
    fn test_gpt_usable_sectors() {
        let geom = DiskGeometry::standard(204800);
        let usable = geom.gpt_usable_sectors(128, 128);

        // 204766 - 34 + 1 = 204733
        assert_eq!(usable, 204733);
    }
}
