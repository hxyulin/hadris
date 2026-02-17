//! FAT volume parameter calculation.
//!
//! This module calculates the optimal parameters for formatting a FAT volume
//! based on the volume size and user options.

use crate::FatType;
use crate::error::{FatError, Result};

use super::options::{FatTypeSelection, FormatOptions};

/// Calculated parameters for formatting a FAT volume.
#[derive(Debug, Clone)]
pub struct FormatParams {
    /// FAT type (FAT12, FAT16, or FAT32)
    pub fat_type: FatType,
    /// Sector size in bytes
    pub sector_size: usize,
    /// Sectors per cluster
    pub sectors_per_cluster: u8,
    /// Number of reserved sectors
    pub reserved_sectors: u16,
    /// Number of FAT copies
    pub fat_count: u8,
    /// Root directory entry count (FAT12/16 only)
    pub root_entry_count: u16,
    /// Total sectors
    pub total_sectors: u32,
    /// Sectors per FAT
    pub sectors_per_fat: u32,
    /// Total cluster count
    pub cluster_count: u32,
    /// Data region start sector
    pub data_start_sector: u32,
    /// Media type byte
    pub media_type: u8,
    /// Hidden sectors
    pub hidden_sectors: u32,
}

/// Minimum volume sizes for each FAT type (in bytes)
const MIN_FAT12_SIZE: u64 = 1024 * 512; // ~512 KB minimum
const MIN_FAT16_SIZE: u64 = 4 * 1024 * 1024; // 4 MB minimum for FAT16
const MIN_FAT32_SIZE: u64 = 32 * 1024 * 1024; // 32 MB minimum for FAT32

/// Maximum volume sizes for each FAT type (in bytes)
const MAX_FAT12_SIZE: u64 = 32 * 1024 * 1024; // ~32 MB max for FAT12
const MAX_FAT16_SIZE: u64 = 2u64 * 1024 * 1024 * 1024; // 2 GB max for FAT16
const MAX_FAT32_SIZE: u64 = 2u64 * 1024 * 1024 * 1024 * 1024; // 2 TB max for FAT32

/// Microsoft FAT specification cluster count thresholds
const FAT12_MAX_CLUSTERS: u32 = 4084;
const FAT16_MAX_CLUSTERS: u32 = 65524;

/// Calculate formatting parameters from options.
pub fn calculate_params(options: &FormatOptions) -> Result<FormatParams> {
    let sector_size = options.sector_size.bytes();
    let total_sectors = (options.volume_size / sector_size as u64) as u32;

    if total_sectors < 128 {
        return Err(FatError::VolumeTooSmall {
            size: options.volume_size,
            min_size: 128 * sector_size as u64,
        });
    }

    // Determine FAT type
    let fat_type = determine_fat_type(options)?;

    // Calculate sectors per cluster
    let sectors_per_cluster = options.sectors_per_cluster.unwrap_or_else(|| {
        calculate_sectors_per_cluster(options.volume_size, fat_type, sector_size)
    });

    // Validate sectors per cluster is a power of 2 and within bounds
    if !sectors_per_cluster.is_power_of_two() || sectors_per_cluster > 128 {
        return Err(FatError::InvalidFormatOption {
            option: "sectors_per_cluster",
            reason: "must be a power of 2 and <= 128",
        });
    }

    // Calculate reserved sectors
    let reserved_sectors: u16 = match fat_type {
        FatType::Fat12 | FatType::Fat16 => 1,
        FatType::Fat32 => 32, // FAT32 needs more space for FSInfo and backup boot sector
    };

    // Root directory parameters (FAT12/16 only)
    let root_entry_count = match fat_type {
        FatType::Fat12 | FatType::Fat16 => options.root_entry_count,
        FatType::Fat32 => 0, // FAT32 has root dir in cluster chain
    };

    // Root directory sectors (FAT12/16 only)
    let root_dir_sectors = (root_entry_count as u32 * 32).div_ceil(sector_size as u32);

    // Calculate FAT size
    let (sectors_per_fat, cluster_count) = calculate_fat_size(
        fat_type,
        total_sectors,
        reserved_sectors as u32,
        root_dir_sectors,
        sectors_per_cluster as u32,
        options.fat_copies as u32,
        sector_size,
    )?;

    // Validate cluster count for FAT type
    validate_cluster_count(fat_type, cluster_count)?;

    // Calculate data start sector
    let data_start_sector =
        reserved_sectors as u32 + (options.fat_copies as u32 * sectors_per_fat) + root_dir_sectors;

    Ok(FormatParams {
        fat_type,
        sector_size,
        sectors_per_cluster,
        reserved_sectors,
        fat_count: options.fat_copies,
        root_entry_count,
        total_sectors,
        sectors_per_fat,
        cluster_count,
        data_start_sector,
        media_type: options.media_type.value(),
        hidden_sectors: options.hidden_sectors,
    })
}

/// Determine the FAT type based on volume size and user preference.
fn determine_fat_type(options: &FormatOptions) -> Result<FatType> {
    let size = options.volume_size;

    // Check minimum size
    if size < MIN_FAT12_SIZE {
        return Err(FatError::VolumeTooSmall {
            size,
            min_size: MIN_FAT12_SIZE,
        });
    }

    match options.fat_type {
        FatTypeSelection::Auto => {
            // Auto-select based on volume size
            if size <= MAX_FAT12_SIZE {
                Ok(FatType::Fat12)
            } else if size <= MAX_FAT16_SIZE {
                Ok(FatType::Fat16)
            } else if size <= MAX_FAT32_SIZE {
                Ok(FatType::Fat32)
            } else {
                Err(FatError::VolumeTooLarge {
                    size,
                    max_size: MAX_FAT32_SIZE,
                })
            }
        }
        FatTypeSelection::Fat12 => {
            if size > MAX_FAT12_SIZE {
                Err(FatError::VolumeTooLarge {
                    size,
                    max_size: MAX_FAT12_SIZE,
                })
            } else {
                Ok(FatType::Fat12)
            }
        }
        FatTypeSelection::Fat16 => {
            if size < MIN_FAT16_SIZE {
                Err(FatError::VolumeTooSmall {
                    size,
                    min_size: MIN_FAT16_SIZE,
                })
            } else if size > MAX_FAT16_SIZE {
                Err(FatError::VolumeTooLarge {
                    size,
                    max_size: MAX_FAT16_SIZE,
                })
            } else {
                Ok(FatType::Fat16)
            }
        }
        FatTypeSelection::Fat32 => {
            if size < MIN_FAT32_SIZE {
                Err(FatError::VolumeTooSmall {
                    size,
                    min_size: MIN_FAT32_SIZE,
                })
            } else if size > MAX_FAT32_SIZE {
                Err(FatError::VolumeTooLarge {
                    size,
                    max_size: MAX_FAT32_SIZE,
                })
            } else {
                Ok(FatType::Fat32)
            }
        }
    }
}

/// Calculate optimal sectors per cluster based on volume size.
///
/// These values are based on Microsoft's recommendations.
fn calculate_sectors_per_cluster(volume_size: u64, fat_type: FatType, sector_size: usize) -> u8 {
    let size_mb = volume_size / (1024 * 1024);

    match fat_type {
        FatType::Fat12 => {
            // FAT12: Keep clusters small for efficiency
            if sector_size == 512 {
                if size_mb <= 2 {
                    1
                } else if size_mb <= 4 {
                    2
                } else if size_mb <= 8 {
                    4
                } else if size_mb <= 16 {
                    8
                } else {
                    16
                }
            } else {
                1
            }
        }
        FatType::Fat16 => {
            // FAT16: Microsoft recommended defaults
            if sector_size == 512 {
                if size_mb <= 8 {
                    1
                } else if size_mb <= 16 {
                    2
                } else if size_mb <= 32 {
                    4
                } else if size_mb <= 64 {
                    8
                } else if size_mb <= 128 {
                    16
                } else if size_mb <= 256 {
                    32
                } else if size_mb <= 512 {
                    64
                } else {
                    128
                }
            } else {
                // Adjust for larger sector sizes
                (32768 / sector_size).clamp(1, 128) as u8
            }
        }
        FatType::Fat32 => {
            // FAT32: Microsoft recommended defaults
            let size_gb = volume_size / (1024 * 1024 * 1024);
            if sector_size == 512 {
                if size_mb <= 64 {
                    1
                } else if size_mb <= 128 {
                    2
                } else if size_mb <= 256 {
                    4
                } else if size_gb <= 8 {
                    8
                } else if size_gb <= 16 {
                    16
                } else if size_gb <= 32 {
                    32
                } else {
                    64
                }
            } else {
                // Adjust for larger sector sizes
                (32768 / sector_size).clamp(1, 128) as u8
            }
        }
    }
}

/// Calculate the number of sectors per FAT and total cluster count.
fn calculate_fat_size(
    fat_type: FatType,
    total_sectors: u32,
    reserved_sectors: u32,
    root_dir_sectors: u32,
    sectors_per_cluster: u32,
    fat_count: u32,
    sector_size: usize,
) -> Result<(u32, u32)> {
    // Available sectors for FAT and data
    let overhead = reserved_sectors + root_dir_sectors;
    if total_sectors <= overhead {
        return Err(FatError::VolumeTooSmall {
            size: total_sectors as u64 * sector_size as u64,
            min_size: (overhead + 1) as u64 * sector_size as u64,
        });
    }

    let data_and_fat_sectors = total_sectors - overhead;

    // Calculate based on FAT entry size
    let (sectors_per_fat, cluster_count) = match fat_type {
        FatType::Fat12 => {
            // FAT12: 1.5 bytes per entry
            // Formula: clusters = (data_and_fat_sectors - fat_count * fat_sectors) / spc
            // fat_sectors = ceil((clusters + 2) * 1.5 / sector_size)
            // Solve iteratively
            let mut fat_sectors = 1u32;
            loop {
                let data_sectors = data_and_fat_sectors.saturating_sub(fat_count * fat_sectors);
                let clusters = data_sectors / sectors_per_cluster;
                let needed_fat_bytes = ((clusters + 2) * 3).div_ceil(2);
                let needed_fat_sectors = needed_fat_bytes.div_ceil(sector_size as u32);

                if needed_fat_sectors <= fat_sectors {
                    break (fat_sectors, clusters);
                }
                fat_sectors = needed_fat_sectors;
                if fat_sectors > total_sectors {
                    return Err(FatError::VolumeTooSmall {
                        size: total_sectors as u64 * sector_size as u64,
                        min_size: MIN_FAT12_SIZE,
                    });
                }
            }
        }
        FatType::Fat16 => {
            // FAT16: 2 bytes per entry
            let mut fat_sectors = 1u32;
            loop {
                let data_sectors = data_and_fat_sectors.saturating_sub(fat_count * fat_sectors);
                let clusters = data_sectors / sectors_per_cluster;
                let needed_fat_bytes = (clusters + 2) * 2;
                let needed_fat_sectors = needed_fat_bytes.div_ceil(sector_size as u32);

                if needed_fat_sectors <= fat_sectors {
                    break (fat_sectors, clusters);
                }
                fat_sectors = needed_fat_sectors;
                if fat_sectors > total_sectors {
                    return Err(FatError::VolumeTooSmall {
                        size: total_sectors as u64 * sector_size as u64,
                        min_size: MIN_FAT16_SIZE,
                    });
                }
            }
        }
        FatType::Fat32 => {
            // FAT32: 4 bytes per entry
            let mut fat_sectors = 1u32;
            loop {
                let data_sectors = data_and_fat_sectors.saturating_sub(fat_count * fat_sectors);
                let clusters = data_sectors / sectors_per_cluster;
                let needed_fat_bytes = (clusters + 2) * 4;
                let needed_fat_sectors = needed_fat_bytes.div_ceil(sector_size as u32);

                if needed_fat_sectors <= fat_sectors {
                    break (fat_sectors, clusters);
                }
                fat_sectors = needed_fat_sectors;
                if fat_sectors > total_sectors {
                    return Err(FatError::VolumeTooSmall {
                        size: total_sectors as u64 * sector_size as u64,
                        min_size: MIN_FAT32_SIZE,
                    });
                }
            }
        }
    };

    Ok((sectors_per_fat, cluster_count))
}

/// Validate that the cluster count is appropriate for the FAT type.
fn validate_cluster_count(fat_type: FatType, cluster_count: u32) -> Result<()> {
    match fat_type {
        FatType::Fat12 => {
            if cluster_count > FAT12_MAX_CLUSTERS {
                return Err(FatError::InvalidFormatOption {
                    option: "cluster_count",
                    reason: "too many clusters for FAT12 (max 4084)",
                });
            }
        }
        FatType::Fat16 => {
            if cluster_count <= FAT12_MAX_CLUSTERS {
                return Err(FatError::InvalidFormatOption {
                    option: "cluster_count",
                    reason: "too few clusters for FAT16 (use FAT12 instead)",
                });
            }
            if cluster_count > FAT16_MAX_CLUSTERS {
                return Err(FatError::InvalidFormatOption {
                    option: "cluster_count",
                    reason: "too many clusters for FAT16 (max 65524)",
                });
            }
        }
        FatType::Fat32 => {
            if cluster_count <= FAT16_MAX_CLUSTERS {
                return Err(FatError::InvalidFormatOption {
                    option: "cluster_count",
                    reason: "too few clusters for FAT32 (use FAT16 instead)",
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fat12_small_volume() {
        let options = FormatOptions::new(2 * 1024 * 1024); // 2 MB
        let params = calculate_params(&options).unwrap();
        assert_eq!(params.fat_type, FatType::Fat12);
    }

    #[test]
    fn test_fat16_medium_volume() {
        let options = FormatOptions::new(64 * 1024 * 1024); // 64 MB
        let params = calculate_params(&options).unwrap();
        assert_eq!(params.fat_type, FatType::Fat16);
    }

    #[test]
    fn test_fat32_large_volume() {
        // Need a large volume (4 GB) to trigger FAT32 with auto-selection
        // because 1-2 GB is still within FAT16 range
        let options = FormatOptions::new(4u64 * 1024 * 1024 * 1024); // 4 GB
        let params = calculate_params(&options).unwrap();
        assert_eq!(params.fat_type, FatType::Fat32);
    }

    #[test]
    fn test_forced_fat32() {
        let mut options = FormatOptions::new(64 * 1024 * 1024); // 64 MB
        options.fat_type = FatTypeSelection::Fat32;
        let params = calculate_params(&options).unwrap();
        assert_eq!(params.fat_type, FatType::Fat32);
    }
}
