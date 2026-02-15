//! exFAT Boot Region implementation.
//!
//! The exFAT boot region consists of 12 sectors:
//! - Sector 0: Main Boot Sector
//! - Sector 1-8: Extended Boot Sectors
//! - Sector 9: OEM Parameters
//! - Sector 10: Reserved
//! - Sector 11: Boot Checksum
//!
//! There is also a backup boot region at sectors 12-23.

use core::mem::size_of;

use hadris_common::types::{
    endian::{Endian, LittleEndian},
    number::{U16, U32, U64},
};

use crate::error::{FatError, Result};
use crate::io::{Read, ReadExt, Seek, SeekFrom};

/// Size of the boot region in sectors
pub const BOOT_REGION_SECTORS: usize = 12;

/// exFAT filesystem signature "EXFAT   " (8 bytes, space-padded)
pub const EXFAT_SIGNATURE: [u8; 8] = *b"EXFAT   ";

/// Boot sector signature (0xAA55)
pub const BOOT_SIGNATURE: u16 = 0xAA55;

/// Raw exFAT Boot Sector structure (512 bytes minimum, but can be larger).
///
/// This represents the Main Boot Sector (sector 0) and Backup Boot Sector (sector 12).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawExFatBootSector {
    /// Jump instruction to boot code (0xEB7690 typical)
    pub jump_boot: [u8; 3],
    /// Filesystem name: must be "EXFAT   " (8 bytes, space-padded)
    pub fs_name: [u8; 8],
    /// Must be zero (reserved for FAT BPB compatibility)
    pub must_be_zero: [u8; 53],
    /// Partition offset in sectors from start of media
    pub partition_offset: U64<LittleEndian>,
    /// Volume length in sectors
    pub volume_length: U64<LittleEndian>,
    /// FAT offset in sectors from start of volume
    pub fat_offset: U32<LittleEndian>,
    /// FAT length in sectors
    pub fat_length: U32<LittleEndian>,
    /// Cluster heap offset in sectors from start of volume
    pub cluster_heap_offset: U32<LittleEndian>,
    /// Total number of clusters in the cluster heap
    pub cluster_count: U32<LittleEndian>,
    /// First cluster of root directory
    pub first_cluster_of_root: U32<LittleEndian>,
    /// Volume serial number
    pub volume_serial_number: U32<LittleEndian>,
    /// Filesystem revision (major.minor as high.low byte)
    pub fs_revision: U16<LittleEndian>,
    /// Volume flags
    pub volume_flags: U16<LittleEndian>,
    /// Log2 of bytes per sector (9 = 512, 10 = 1024, etc.)
    pub bytes_per_sector_shift: u8,
    /// Log2 of sectors per cluster
    pub sectors_per_cluster_shift: u8,
    /// Number of FATs (1 or 2)
    pub number_of_fats: u8,
    /// Drive select (for INT 13h)
    pub drive_select: u8,
    /// Percent of clusters in use
    pub percent_in_use: u8,
    /// Reserved
    pub reserved: [u8; 7],
    /// Boot code
    pub boot_code: [u8; 390],
    /// Boot signature (0xAA55)
    pub boot_signature: U16<LittleEndian>,
}

// Safety: RawExFatBootSector is a C-style struct with no padding requirements issues
unsafe impl bytemuck::NoUninit for RawExFatBootSector {}
unsafe impl bytemuck::Zeroable for RawExFatBootSector {}
unsafe impl bytemuck::AnyBitPattern for RawExFatBootSector {}

impl RawExFatBootSector {
    /// Validate the boot sector
    pub fn validate(&self) -> Result<()> {
        // Check filesystem name
        if self.fs_name != EXFAT_SIGNATURE {
            return Err(FatError::ExFatInvalidSignature {
                expected: EXFAT_SIGNATURE,
                found: self.fs_name,
            });
        }

        // Check boot signature
        let sig = self.boot_signature.get();
        if sig != BOOT_SIGNATURE {
            return Err(FatError::InvalidBootSignature { found: sig });
        }

        // Check must_be_zero field
        if self.must_be_zero.iter().any(|&b| b != 0) {
            return Err(FatError::ExFatInvalidBootSector {
                reason: "must_be_zero field contains non-zero bytes",
            });
        }

        // Validate bytes_per_sector_shift (9-12 for 512-4096 bytes)
        if !(9..=12).contains(&self.bytes_per_sector_shift) {
            return Err(FatError::ExFatInvalidBootSector {
                reason: "invalid bytes_per_sector_shift (must be 9-12)",
            });
        }

        // Validate sectors_per_cluster_shift (0-25, but bytes_per_sector_shift + sectors_per_cluster_shift <= 25)
        let combined_shift = self.bytes_per_sector_shift + self.sectors_per_cluster_shift;
        if combined_shift > 25 {
            return Err(FatError::ExFatInvalidBootSector {
                reason: "combined sector/cluster shift too large (max cluster size 32MB)",
            });
        }

        // Validate number of FATs (1 or 2)
        if self.number_of_fats != 1 && self.number_of_fats != 2 {
            return Err(FatError::ExFatInvalidBootSector {
                reason: "invalid number_of_fats (must be 1 or 2)",
            });
        }

        Ok(())
    }

    /// Get bytes per sector
    pub fn bytes_per_sector(&self) -> usize {
        1 << self.bytes_per_sector_shift
    }

    /// Get sectors per cluster
    pub fn sectors_per_cluster(&self) -> usize {
        1 << self.sectors_per_cluster_shift
    }

    /// Get bytes per cluster
    pub fn bytes_per_cluster(&self) -> usize {
        self.bytes_per_sector() * self.sectors_per_cluster()
    }
}

/// Computed exFAT filesystem information derived from the boot sector.
#[derive(Debug, Clone)]
pub struct ExFatInfo {
    /// Bytes per sector (typically 512)
    pub bytes_per_sector: usize,
    /// Sectors per cluster
    pub sectors_per_cluster: usize,
    /// Bytes per cluster
    pub bytes_per_cluster: usize,
    /// FAT offset in bytes from start of volume
    pub fat_offset: u64,
    /// FAT length in bytes
    pub fat_length: u64,
    /// Cluster heap offset in bytes from start of volume
    pub cluster_heap_offset: u64,
    /// Total number of clusters
    pub cluster_count: u32,
    /// First cluster of root directory
    pub root_cluster: u32,
    /// Volume serial number
    pub volume_serial: u32,
    /// Number of FATs (1 or 2)
    pub fat_count: u8,
}

impl ExFatInfo {
    /// Create ExFatInfo from a validated boot sector
    pub fn from_boot_sector(bs: &RawExFatBootSector) -> Self {
        let bytes_per_sector = bs.bytes_per_sector();
        let sectors_per_cluster = bs.sectors_per_cluster();

        Self {
            bytes_per_sector,
            sectors_per_cluster,
            bytes_per_cluster: bytes_per_sector * sectors_per_cluster,
            fat_offset: bs.fat_offset.get() as u64 * bytes_per_sector as u64,
            fat_length: bs.fat_length.get() as u64 * bytes_per_sector as u64,
            cluster_heap_offset: bs.cluster_heap_offset.get() as u64 * bytes_per_sector as u64,
            cluster_count: bs.cluster_count.get(),
            root_cluster: bs.first_cluster_of_root.get(),
            volume_serial: bs.volume_serial_number.get(),
            fat_count: bs.number_of_fats,
        }
    }

    /// Convert a cluster number to a byte offset from the start of the volume
    pub fn cluster_to_offset(&self, cluster: u32) -> u64 {
        // Clusters start at 2
        self.cluster_heap_offset + (cluster as u64 - 2) * self.bytes_per_cluster as u64
    }

    /// Check if a cluster number is valid
    pub fn is_valid_cluster(&self, cluster: u32) -> bool {
        cluster >= 2 && cluster < self.cluster_count + 2
    }
}

/// Parsed exFAT Boot Sector with computed values.
#[derive(Clone)]
pub struct ExFatBootSector {
    /// Raw boot sector data
    raw: RawExFatBootSector,
    /// Computed filesystem info
    pub info: ExFatInfo,
}

impl core::fmt::Debug for ExFatBootSector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExFatBootSector")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl ExFatBootSector {
    /// Read and validate the boot sector from a data source.
    pub fn read<DATA: Read + Seek>(data: &mut DATA) -> Result<Self> {
        data.seek(SeekFrom::Start(0))?;
        let raw: RawExFatBootSector = data.read_struct()?;
        raw.validate()?;

        let info = ExFatInfo::from_boot_sector(&raw);
        Ok(Self { raw, info })
    }

    /// Validate the boot region checksum.
    ///
    /// The checksum is computed over sectors 0-10 and stored in sector 11.
    /// This validates both the main boot region and optionally the backup.
    pub fn validate_checksum<DATA: Read + Seek>(data: &mut DATA, sector_size: usize) -> Result<()> {
        // Read sectors 0-10 and compute checksum
        let mut checksum: u32 = 0;

        for sector in 0..11 {
            data.seek(SeekFrom::Start(sector as u64 * sector_size as u64))?;

            for byte_idx in 0..sector_size {
                let mut byte = [0u8; 1];
                data.read_exact(&mut byte)?;

                // Skip VolumeFlags (bytes 106-107) and PercentInUse (byte 112)
                // in sector 0 as they may change
                if sector == 0 && (byte_idx == 106 || byte_idx == 107 || byte_idx == 112) {
                    continue;
                }

                // Rotate right and add
                checksum = checksum.rotate_right(1).wrapping_add(byte[0] as u32);
            }
        }

        // Read checksum sector (sector 11)
        data.seek(SeekFrom::Start(11 * sector_size as u64))?;

        // The checksum sector contains the checksum repeated to fill the sector
        let expected_count = sector_size / size_of::<u32>();
        for _ in 0..expected_count {
            let mut stored = [0u8; 4];
            data.read_exact(&mut stored)?;
            let stored_checksum = u32::from_le_bytes(stored);

            if stored_checksum != checksum {
                return Err(FatError::ExFatInvalidChecksum {
                    expected: checksum,
                    found: stored_checksum,
                });
            }
        }

        Ok(())
    }

    /// Get the raw boot sector
    pub fn raw(&self) -> &RawExFatBootSector {
        &self.raw
    }

    /// Get the computed filesystem info
    pub fn info(&self) -> &ExFatInfo {
        &self.info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::const_assert_eq;

    const_assert_eq!(size_of::<RawExFatBootSector>(), 512);
}
