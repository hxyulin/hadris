//! exFAT Volume Formatting implementation.
//!
//! This module provides functionality to format storage devices with the exFAT filesystem.
//!
//! The exFAT boot region consists of 24 sectors total:
//! - Sectors 0-11: Main Boot Region
//!   - Sector 0: Main Boot Sector
//!   - Sectors 1-8: Extended Boot Sectors
//!   - Sector 9: OEM Parameters
//!   - Sector 10: Reserved
//!   - Sector 11: Boot Checksum
//! - Sectors 12-23: Backup Boot Region (identical to main)

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use hadris_common::types::{
    endian::{Endian, LittleEndian},
    number::{U16, U32, U64},
};

use crate::error::{FatError, Result};
use crate::io::{Read, Seek, SeekFrom, Write};

use super::boot::{RawExFatBootSector, BOOT_REGION_SECTORS, BOOT_SIGNATURE, EXFAT_SIGNATURE};
use super::entry::{
    entry_type, RawAllocationBitmapEntry, RawDirectoryEntry, RawUpcaseTableEntry,
    RawVolumeLabelEntry,
};
use super::fs::ExFatFs;
use super::upcase::generate_compressed_upcase_table;

/// Minimum volume size for exFAT (1 MB)
const MIN_VOLUME_SIZE: u64 = 1024 * 1024;

/// Maximum volume size for exFAT (256 TB with 512-byte sectors)
const MAX_VOLUME_SIZE: u64 = 256 * 1024 * 1024 * 1024 * 1024;

/// Default boundary alignment (1 MB)
const DEFAULT_BOUNDARY_ALIGNMENT: u64 = 1024 * 1024;

/// Configuration options for formatting an exFAT volume.
#[derive(Debug, Clone)]
pub struct ExFatFormatOptions {
    /// Volume label (up to 11 UTF-16 characters)
    pub label: Option<String>,
    /// Bytes per sector (512, 1024, 2048, or 4096)
    pub bytes_per_sector: usize,
    /// Sectors per cluster (cluster size must not exceed 32 MB)
    pub sectors_per_cluster: usize,
    /// Number of FATs (1 or 2)
    pub fat_count: u8,
    /// Boundary alignment for cluster heap (typically 1 MB for flash drives)
    pub boundary_alignment: Option<u64>,
    /// Volume serial number (random if None)
    pub volume_serial: Option<u32>,
    /// Partition offset (for media with partition table)
    pub partition_offset: u64,
}

impl Default for ExFatFormatOptions {
    fn default() -> Self {
        Self {
            label: None,
            bytes_per_sector: 512,
            sectors_per_cluster: 0, // Auto-calculate
            fat_count: 1,
            boundary_alignment: Some(DEFAULT_BOUNDARY_ALIGNMENT),
            volume_serial: None,
            partition_offset: 0,
        }
    }
}

impl ExFatFormatOptions {
    /// Create options with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the volume label.
    pub fn with_label(mut self, label: &str) -> Self {
        self.label = Some(label.to_string());
        self
    }

    /// Set the bytes per sector.
    pub fn with_sector_size(mut self, size: usize) -> Self {
        self.bytes_per_sector = size;
        self
    }

    /// Set the sectors per cluster.
    pub fn with_sectors_per_cluster(mut self, spc: usize) -> Self {
        self.sectors_per_cluster = spc;
        self
    }

    /// Set the number of FATs.
    pub fn with_fat_count(mut self, count: u8) -> Self {
        self.fat_count = count;
        self
    }

    fn validate(&self) -> Result<()> {
        // Validate sector size (must be power of 2, 512-4096)
        if !matches!(self.bytes_per_sector, 512 | 1024 | 2048 | 4096) {
            return Err(FatError::InvalidFormatOption {
                option: "bytes_per_sector",
                reason: "must be 512, 1024, 2048, or 4096",
            });
        }

        // Validate FAT count
        if self.fat_count != 1 && self.fat_count != 2 {
            return Err(FatError::InvalidFormatOption {
                option: "fat_count",
                reason: "must be 1 or 2",
            });
        }

        // Validate label length
        if let Some(ref label) = self.label {
            let utf16_len = label.encode_utf16().count();
            if utf16_len > 11 {
                return Err(FatError::InvalidFormatOption {
                    option: "label",
                    reason: "must be 11 UTF-16 characters or fewer",
                });
            }
        }

        Ok(())
    }
}

/// Calculated layout parameters for exFAT formatting.
#[derive(Debug, Clone)]
pub struct ExFatLayoutParams {
    /// Bytes per sector
    pub bytes_per_sector: usize,
    /// Log2 of bytes per sector
    pub bytes_per_sector_shift: u8,
    /// Sectors per cluster
    pub sectors_per_cluster: usize,
    /// Log2 of sectors per cluster
    pub sectors_per_cluster_shift: u8,
    /// Total volume size in sectors
    pub volume_length: u64,
    /// FAT offset in sectors
    pub fat_offset: u32,
    /// FAT length in sectors
    pub fat_length: u32,
    /// Cluster heap offset in sectors
    pub cluster_heap_offset: u32,
    /// Total number of clusters
    pub cluster_count: u32,
    /// Volume serial number
    pub volume_serial: u32,
    /// Number of FATs
    pub fat_count: u8,
}

/// Calculate the layout parameters for an exFAT volume.
pub fn calculate_layout(volume_size: u64, options: &ExFatFormatOptions) -> Result<ExFatLayoutParams> {
    // Validate volume size
    if volume_size < MIN_VOLUME_SIZE {
        return Err(FatError::VolumeTooSmall {
            size: volume_size,
            min_size: MIN_VOLUME_SIZE,
        });
    }
    if volume_size > MAX_VOLUME_SIZE {
        return Err(FatError::VolumeTooLarge {
            size: volume_size,
            max_size: MAX_VOLUME_SIZE,
        });
    }

    options.validate()?;

    let bytes_per_sector = options.bytes_per_sector;
    let bytes_per_sector_shift = (bytes_per_sector as u32).trailing_zeros() as u8;

    // Calculate total sectors
    let volume_length = volume_size / bytes_per_sector as u64;

    // Calculate sectors per cluster (auto if not specified)
    let sectors_per_cluster = if options.sectors_per_cluster == 0 {
        calculate_sectors_per_cluster(volume_size, bytes_per_sector)
    } else {
        options.sectors_per_cluster
    };

    // Validate cluster size (max 32 MB)
    let bytes_per_cluster = bytes_per_sector * sectors_per_cluster;
    if bytes_per_cluster > 32 * 1024 * 1024 {
        return Err(FatError::InvalidFormatOption {
            option: "sectors_per_cluster",
            reason: "cluster size exceeds 32 MB maximum",
        });
    }

    let sectors_per_cluster_shift = (sectors_per_cluster as u32).trailing_zeros() as u8;

    // FAT offset: after the main and backup boot regions (24 sectors)
    // Align to boundary if specified
    let min_fat_offset = 24u32; // 12 main + 12 backup
    let fat_offset = if let Some(alignment) = options.boundary_alignment {
        let align_sectors = (alignment / bytes_per_sector as u64) as u32;
        if align_sectors > min_fat_offset {
            align_sectors
        } else {
            min_fat_offset
        }
    } else {
        min_fat_offset
    };

    // Calculate cluster heap offset and cluster count
    // We need to iterate because FAT size depends on cluster count
    let (cluster_heap_offset, fat_length, cluster_count) =
        calculate_fat_and_heap(volume_length, fat_offset, bytes_per_sector, sectors_per_cluster, options.fat_count)?;

    // Generate volume serial number
    let volume_serial = options.volume_serial.unwrap_or_else(|| generate_serial());

    Ok(ExFatLayoutParams {
        bytes_per_sector,
        bytes_per_sector_shift,
        sectors_per_cluster,
        sectors_per_cluster_shift,
        volume_length,
        fat_offset,
        fat_length,
        cluster_heap_offset,
        cluster_count,
        volume_serial,
        fat_count: options.fat_count,
    })
}

/// Calculate sectors per cluster based on volume size.
fn calculate_sectors_per_cluster(volume_size: u64, bytes_per_sector: usize) -> usize {
    // exFAT recommendations for cluster size
    // These are similar to Windows defaults
    let mb = 1024 * 1024u64;
    let gb = 1024 * mb;

    let bytes_per_cluster = if volume_size < 256 * mb {
        4 * 1024 // 4 KB
    } else if volume_size < 32 * gb {
        32 * 1024 // 32 KB
    } else if volume_size < 256 * gb {
        128 * 1024 // 128 KB
    } else {
        256 * 1024 // 256 KB (could go up to 32 MB)
    };

    (bytes_per_cluster / bytes_per_sector).max(1)
}

/// Calculate FAT and cluster heap parameters.
fn calculate_fat_and_heap(
    volume_length: u64,
    fat_offset: u32,
    bytes_per_sector: usize,
    sectors_per_cluster: usize,
    fat_count: u8,
) -> Result<(u32, u32, u32)> {
    // Each FAT entry is 4 bytes
    // FAT length = ceil(cluster_count / entries_per_sector) where entries_per_sector = bytes_per_sector / 4
    let entries_per_sector = bytes_per_sector / 4;

    // Initial estimate: all remaining space for data
    let total_fat_sectors_estimate = 1u32;
    let cluster_heap_offset_estimate = fat_offset + total_fat_sectors_estimate * fat_count as u32;

    // Available sectors for cluster heap
    let available_sectors = volume_length as u32 - cluster_heap_offset_estimate;
    let cluster_count_estimate = available_sectors / sectors_per_cluster as u32;

    // Calculate FAT size needed for this many clusters (+2 for reserved entries)
    let fat_entries_needed = cluster_count_estimate + 2;
    let fat_length = ((fat_entries_needed as usize + entries_per_sector - 1) / entries_per_sector) as u32;

    // Recalculate with actual FAT size
    let cluster_heap_offset = fat_offset + fat_length * fat_count as u32;
    let available_sectors = if volume_length as u32 > cluster_heap_offset {
        volume_length as u32 - cluster_heap_offset
    } else {
        return Err(FatError::VolumeTooSmall {
            size: volume_length * bytes_per_sector as u64,
            min_size: MIN_VOLUME_SIZE,
        });
    };
    let cluster_count = available_sectors / sectors_per_cluster as u32;

    if cluster_count < 1 {
        return Err(FatError::VolumeTooSmall {
            size: volume_length * bytes_per_sector as u64,
            min_size: MIN_VOLUME_SIZE,
        });
    }

    Ok((cluster_heap_offset, fat_length, cluster_count))
}

/// Generate a pseudo-random volume serial number.
fn generate_serial() -> u32 {
    #[cfg(feature = "std")]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        (duration.as_secs() as u32) ^ (duration.subsec_nanos())
    }
    #[cfg(not(feature = "std"))]
    {
        0x12345678 // Fixed value for no-std
    }
}

/// Format a volume with the exFAT filesystem.
///
/// # Arguments
/// * `data` - The underlying storage device
/// * `volume_size` - Total size of the volume in bytes
/// * `options` - Formatting options
///
/// # Returns
/// An opened `ExFatFs` handle to the newly formatted filesystem.
pub fn format_exfat<DATA>(
    mut data: DATA,
    volume_size: u64,
    options: &ExFatFormatOptions,
) -> Result<ExFatFs<DATA>>
where
    DATA: Read + Write + Seek,
{
    let params = calculate_layout(volume_size, options)?;

    // Write main and backup boot regions
    write_boot_region(&mut data, &params, 0)?;
    write_boot_region(&mut data, &params, BOOT_REGION_SECTORS as u64)?;

    // Initialize FAT
    initialize_fat(&mut data, &params)?;

    // Initialize root directory with system entries
    let (bitmap_cluster, _bitmap_size, upcase_cluster, upcase_size) =
        initialize_root_directory(&mut data, &params, &options.label)?;

    // Initialize allocation bitmap (mark used clusters)
    initialize_allocation_bitmap(&mut data, &params, bitmap_cluster, upcase_cluster, upcase_size)?;

    // Write boot region checksums
    write_boot_checksum(&mut data, &params, 0)?;
    write_boot_checksum(&mut data, &params, BOOT_REGION_SECTORS as u64)?;

    // Seek back to start and open the filesystem
    data.seek(SeekFrom::Start(0))?;
    ExFatFs::open(data)
}

/// Write the boot region (main or backup).
fn write_boot_region<DATA: Write + Seek>(
    data: &mut DATA,
    params: &ExFatLayoutParams,
    sector_offset: u64,
) -> Result<()> {
    let sector_size = params.bytes_per_sector;

    // Create boot sector
    let boot_sector = create_boot_sector(params);
    let boot_bytes = bytemuck::bytes_of(&boot_sector);

    // Write boot sector (sector 0 or 12)
    data.seek(SeekFrom::Start(sector_offset * sector_size as u64))?;
    data.write_all(boot_bytes)?;
    // Pad to sector size if needed
    if boot_bytes.len() < sector_size {
        data.write_all(&vec![0u8; sector_size - boot_bytes.len()])?;
    }

    // Write extended boot sectors (sectors 1-8 or 13-20) - all zeros with 0xAA55 signature
    for i in 1..=8 {
        data.seek(SeekFrom::Start((sector_offset + i) * sector_size as u64))?;
        let mut extended = vec![0u8; sector_size];
        // Extended boot signature at end
        extended[sector_size - 2] = 0x55;
        extended[sector_size - 1] = 0xAA;
        data.write_all(&extended)?;
    }

    // Write OEM parameters sector (sector 9 or 21) - zeros
    data.seek(SeekFrom::Start((sector_offset + 9) * sector_size as u64))?;
    data.write_all(&vec![0u8; sector_size])?;

    // Write reserved sector (sector 10 or 22) - zeros
    data.seek(SeekFrom::Start((sector_offset + 10) * sector_size as u64))?;
    data.write_all(&vec![0u8; sector_size])?;

    // Boot checksum sector (sector 11 or 23) will be written later

    Ok(())
}

/// Create the boot sector structure.
fn create_boot_sector(params: &ExFatLayoutParams) -> RawExFatBootSector {
    RawExFatBootSector {
        jump_boot: [0xEB, 0x76, 0x90], // Standard jump instruction
        fs_name: EXFAT_SIGNATURE,
        must_be_zero: [0; 53],
        partition_offset: U64::<LittleEndian>::new(0),
        volume_length: U64::<LittleEndian>::new(params.volume_length),
        fat_offset: U32::<LittleEndian>::new(params.fat_offset),
        fat_length: U32::<LittleEndian>::new(params.fat_length),
        cluster_heap_offset: U32::<LittleEndian>::new(params.cluster_heap_offset),
        cluster_count: U32::<LittleEndian>::new(params.cluster_count),
        first_cluster_of_root: U32::<LittleEndian>::new(4), // Root at cluster 4 (after bitmap and upcase)
        volume_serial_number: U32::<LittleEndian>::new(params.volume_serial),
        fs_revision: U16::<LittleEndian>::new(0x0100), // Version 1.0
        volume_flags: U16::<LittleEndian>::new(0),
        bytes_per_sector_shift: params.bytes_per_sector_shift,
        sectors_per_cluster_shift: params.sectors_per_cluster_shift,
        number_of_fats: params.fat_count,
        drive_select: 0x80, // Hard disk
        percent_in_use: 0xFF, // Unknown
        reserved: [0; 7],
        boot_code: [0; 390],
        boot_signature: U16::<LittleEndian>::new(BOOT_SIGNATURE),
    }
}

/// Write the boot region checksum.
fn write_boot_checksum<DATA: Read + Write + Seek>(
    data: &mut DATA,
    params: &ExFatLayoutParams,
    sector_offset: u64,
) -> Result<()> {
    let sector_size = params.bytes_per_sector;

    // Compute checksum over sectors 0-10
    let mut checksum: u32 = 0;

    for sector in 0..11 {
        data.seek(SeekFrom::Start((sector_offset + sector) * sector_size as u64))?;

        for byte_idx in 0..sector_size {
            let mut byte = [0u8; 1];
            data.read_exact(&mut byte)?;

            // Skip VolumeFlags (bytes 106-107) and PercentInUse (byte 112) in sector 0
            if sector == 0 && (byte_idx == 106 || byte_idx == 107 || byte_idx == 112) {
                continue;
            }

            checksum = checksum.rotate_right(1).wrapping_add(byte[0] as u32);
        }
    }

    // Write checksum sector (sector 11) - checksum repeated to fill sector
    data.seek(SeekFrom::Start((sector_offset + 11) * sector_size as u64))?;
    let checksum_bytes = checksum.to_le_bytes();
    let repeat_count = sector_size / 4;
    for _ in 0..repeat_count {
        data.write_all(&checksum_bytes)?;
    }

    Ok(())
}

/// Initialize the FAT with reserved entries.
fn initialize_fat<DATA: Write + Seek>(data: &mut DATA, params: &ExFatLayoutParams) -> Result<()> {
    let sector_size = params.bytes_per_sector;
    let fat_offset_bytes = params.fat_offset as u64 * sector_size as u64;

    // FAT entry values:
    // 0: Media type (0xFFFFFFF8)
    // 1: Reserved (0xFFFFFFFF)
    // 2+: Cluster entries

    // Initialize FAT with zeros first
    data.seek(SeekFrom::Start(fat_offset_bytes))?;
    let fat_size = params.fat_length as usize * sector_size;
    data.write_all(&vec![0u8; fat_size])?;

    // Write reserved entries
    data.seek(SeekFrom::Start(fat_offset_bytes))?;
    data.write_all(&0xFFFFFFF8u32.to_le_bytes())?; // Entry 0: Media type
    data.write_all(&0xFFFFFFFFu32.to_le_bytes())?; // Entry 1: Reserved

    // If there's a second FAT, copy the entries
    if params.fat_count == 2 {
        let fat2_offset = fat_offset_bytes + params.fat_length as u64 * sector_size as u64;
        data.seek(SeekFrom::Start(fat2_offset))?;
        data.write_all(&vec![0u8; fat_size])?;
        data.seek(SeekFrom::Start(fat2_offset))?;
        data.write_all(&0xFFFFFFF8u32.to_le_bytes())?;
        data.write_all(&0xFFFFFFFFu32.to_le_bytes())?;
    }

    Ok(())
}

/// Initialize the root directory with system entries.
///
/// Returns (bitmap_cluster, bitmap_size, upcase_cluster, upcase_size)
fn initialize_root_directory<DATA: Write + Seek>(
    data: &mut DATA,
    params: &ExFatLayoutParams,
    label: &Option<String>,
) -> Result<(u32, u64, u32, u64)> {
    let sector_size = params.bytes_per_sector;
    let cluster_size = params.bytes_per_sector * params.sectors_per_cluster;
    let cluster_heap_offset = params.cluster_heap_offset as u64 * sector_size as u64;

    // Generate upcase table
    let (upcase_data, upcase_checksum) = generate_compressed_upcase_table();
    let upcase_size = upcase_data.len() as u64;
    let upcase_clusters = ((upcase_size as usize + cluster_size - 1) / cluster_size) as u32;

    // Allocation bitmap size: 1 bit per cluster
    let bitmap_size = ((params.cluster_count as usize + 7) / 8) as u64;
    let bitmap_clusters = ((bitmap_size as usize + cluster_size - 1) / cluster_size) as u32;

    // Layout:
    // Cluster 2: Allocation Bitmap
    // Cluster 2 + bitmap_clusters: Upcase Table
    // Cluster 2 + bitmap_clusters + upcase_clusters: Root Directory
    let bitmap_cluster = 2u32;
    let upcase_cluster = bitmap_cluster + bitmap_clusters;
    let root_cluster = upcase_cluster + upcase_clusters;

    // Update boot sector with correct root cluster
    // (We need to re-write it since we now know the root cluster)
    let mut boot_sector = create_boot_sector(params);
    boot_sector.first_cluster_of_root = U32::<LittleEndian>::new(root_cluster);
    data.seek(SeekFrom::Start(0))?;
    data.write_all(bytemuck::bytes_of(&boot_sector))?;
    // Also update backup
    data.seek(SeekFrom::Start(BOOT_REGION_SECTORS as u64 * sector_size as u64))?;
    data.write_all(bytemuck::bytes_of(&boot_sector))?;

    // Write upcase table to cluster heap
    let upcase_offset = cluster_heap_offset + (upcase_cluster as u64 - 2) * cluster_size as u64;
    data.seek(SeekFrom::Start(upcase_offset))?;
    data.write_all(&upcase_data)?;
    // Pad to cluster boundary
    let upcase_padding = (upcase_clusters as usize * cluster_size) - upcase_data.len();
    if upcase_padding > 0 {
        data.write_all(&vec![0u8; upcase_padding])?;
    }

    // Update FAT entries for upcase table
    let fat_offset_bytes = params.fat_offset as u64 * sector_size as u64;
    if upcase_clusters > 1 {
        for i in 0..upcase_clusters - 1 {
            let entry_offset = fat_offset_bytes + (upcase_cluster + i) as u64 * 4;
            data.seek(SeekFrom::Start(entry_offset))?;
            data.write_all(&(upcase_cluster + i + 1).to_le_bytes())?;
        }
    }
    // End of chain for last upcase cluster
    let entry_offset = fat_offset_bytes + (upcase_cluster + upcase_clusters - 1) as u64 * 4;
    data.seek(SeekFrom::Start(entry_offset))?;
    data.write_all(&0xFFFFFFFFu32.to_le_bytes())?;

    // Initialize root directory
    let root_offset = cluster_heap_offset + (root_cluster as u64 - 2) * cluster_size as u64;
    data.seek(SeekFrom::Start(root_offset))?;

    // Create directory entries
    let mut entries: Vec<RawDirectoryEntry> = Vec::new();

    // 1. Volume Label Entry (if specified)
    if let Some(label_str) = label {
        let label_entry = create_volume_label_entry(label_str);
        entries.push(label_entry);
    }

    // 2. Allocation Bitmap Entry
    let bitmap_entry = create_bitmap_entry(bitmap_cluster, bitmap_size);
    entries.push(bitmap_entry);

    // 3. Upcase Table Entry
    let upcase_entry = create_upcase_entry(upcase_cluster, upcase_size, upcase_checksum);
    entries.push(upcase_entry);

    // Write entries
    for entry in &entries {
        data.write_all(unsafe { &entry.bytes })?;
    }

    // Fill rest of cluster with zeros (end of directory markers)
    let entries_written = entries.len() * 32;
    let remaining = cluster_size - entries_written;
    data.write_all(&vec![0u8; remaining])?;

    // Update FAT entry for root directory (end of chain)
    let root_fat_offset = fat_offset_bytes + root_cluster as u64 * 4;
    data.seek(SeekFrom::Start(root_fat_offset))?;
    data.write_all(&0xFFFFFFFFu32.to_le_bytes())?;

    Ok((bitmap_cluster, bitmap_size, upcase_cluster, upcase_size))
}

/// Create a volume label directory entry.
fn create_volume_label_entry(label: &str) -> RawDirectoryEntry {
    let mut entry = RawVolumeLabelEntry {
        entry_type: entry_type::VOLUME_LABEL,
        character_count: 0,
        volume_label: [0; 22],
        reserved: [0; 8],
    };

    // Convert label to UTF-16 and copy
    let utf16: Vec<u16> = label.encode_utf16().take(11).collect();
    entry.character_count = utf16.len() as u8;

    for (i, &code_unit) in utf16.iter().enumerate() {
        let bytes = code_unit.to_le_bytes();
        entry.volume_label[i * 2] = bytes[0];
        entry.volume_label[i * 2 + 1] = bytes[1];
    }

    let mut raw = RawDirectoryEntry { bytes: [0; 32] };
    let entry_bytes = bytemuck::bytes_of(&entry);
    unsafe {
        raw.bytes[..entry_bytes.len()].copy_from_slice(entry_bytes);
    }
    raw
}

/// Create an allocation bitmap directory entry.
fn create_bitmap_entry(first_cluster: u32, size: u64) -> RawDirectoryEntry {
    let entry = RawAllocationBitmapEntry {
        entry_type: entry_type::ALLOCATION_BITMAP,
        bitmap_flags: 0, // First (and only) bitmap
        reserved: [0; 18],
        first_cluster: U32::<LittleEndian>::new(first_cluster),
        data_length: U64::<LittleEndian>::new(size),
    };

    let mut raw = RawDirectoryEntry { bytes: [0; 32] };
    let entry_bytes = bytemuck::bytes_of(&entry);
    unsafe {
        raw.bytes[..entry_bytes.len()].copy_from_slice(entry_bytes);
    }
    raw
}

/// Create an upcase table directory entry.
fn create_upcase_entry(first_cluster: u32, size: u64, checksum: u32) -> RawDirectoryEntry {
    let entry = RawUpcaseTableEntry {
        entry_type: entry_type::UPCASE_TABLE,
        reserved1: [0; 3],
        table_checksum: U32::<LittleEndian>::new(checksum),
        reserved2: [0; 12],
        first_cluster: U32::<LittleEndian>::new(first_cluster),
        data_length: U64::<LittleEndian>::new(size),
    };

    let mut raw = RawDirectoryEntry { bytes: [0; 32] };
    let entry_bytes = bytemuck::bytes_of(&entry);
    unsafe {
        raw.bytes[..entry_bytes.len()].copy_from_slice(entry_bytes);
    }
    raw
}

/// Initialize the allocation bitmap.
fn initialize_allocation_bitmap<DATA: Write + Seek>(
    data: &mut DATA,
    params: &ExFatLayoutParams,
    bitmap_cluster: u32,
    upcase_cluster: u32,
    upcase_size: u64,
) -> Result<()> {
    let sector_size = params.bytes_per_sector;
    let cluster_size = params.bytes_per_sector * params.sectors_per_cluster;
    let cluster_heap_offset = params.cluster_heap_offset as u64 * sector_size as u64;

    // Calculate bitmap size
    let bitmap_size = (params.cluster_count as usize + 7) / 8;
    let bitmap_clusters = (bitmap_size + cluster_size - 1) / cluster_size;

    // Calculate upcase clusters
    let upcase_clusters = ((upcase_size as usize + cluster_size - 1) / cluster_size) as u32;
    let root_cluster = upcase_cluster + upcase_clusters;

    // Create bitmap with system clusters marked as used
    let mut bitmap = vec![0u8; bitmap_size];

    // Mark clusters as used:
    // - Cluster 2 to 2 + bitmap_clusters - 1: Bitmap itself
    // - Cluster upcase_cluster to upcase_cluster + upcase_clusters - 1: Upcase table
    // - Cluster root_cluster: Root directory

    // Mark bitmap clusters
    for i in 0..bitmap_clusters as u32 {
        let cluster = bitmap_cluster + i;
        set_bitmap_bit(&mut bitmap, cluster - 2);
    }

    // Mark upcase clusters
    for i in 0..upcase_clusters {
        let cluster = upcase_cluster + i;
        set_bitmap_bit(&mut bitmap, cluster - 2);
    }

    // Mark root directory cluster
    set_bitmap_bit(&mut bitmap, root_cluster - 2);

    // Update FAT entries for bitmap
    let fat_offset_bytes = params.fat_offset as u64 * sector_size as u64;
    if bitmap_clusters > 1 {
        for i in 0..bitmap_clusters - 1 {
            let entry_offset = fat_offset_bytes + (bitmap_cluster + i as u32) as u64 * 4;
            data.seek(SeekFrom::Start(entry_offset))?;
            data.write_all(&(bitmap_cluster + i as u32 + 1).to_le_bytes())?;
        }
    }
    // End of chain for last bitmap cluster
    let entry_offset = fat_offset_bytes + (bitmap_cluster + bitmap_clusters as u32 - 1) as u64 * 4;
    data.seek(SeekFrom::Start(entry_offset))?;
    data.write_all(&0xFFFFFFFFu32.to_le_bytes())?;

    // Write bitmap to cluster heap
    let bitmap_offset = cluster_heap_offset + (bitmap_cluster as u64 - 2) * cluster_size as u64;
    data.seek(SeekFrom::Start(bitmap_offset))?;
    data.write_all(&bitmap)?;

    // Pad to cluster boundary
    let bitmap_padding = (bitmap_clusters * cluster_size) - bitmap_size;
    if bitmap_padding > 0 {
        data.write_all(&vec![0u8; bitmap_padding])?;
    }

    Ok(())
}

/// Set a bit in the allocation bitmap.
fn set_bitmap_bit(bitmap: &mut [u8], cluster_index: u32) {
    let byte_index = cluster_index as usize / 8;
    let bit_index = cluster_index as usize % 8;
    if byte_index < bitmap.len() {
        bitmap[byte_index] |= 1 << bit_index;
    }
}

use alloc::string::ToString;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_calculate_layout_small_volume() {
        let options = ExFatFormatOptions::new();
        let params = calculate_layout(64 * 1024 * 1024, &options).unwrap();

        assert_eq!(params.bytes_per_sector, 512);
        assert!(params.cluster_count > 0);
        assert!(params.fat_offset >= 24);
        assert!(params.cluster_heap_offset > params.fat_offset);
    }

    #[test]
    fn test_calculate_layout_large_volume() {
        let options = ExFatFormatOptions::new();
        let params = calculate_layout(1024 * 1024 * 1024, &options).unwrap();

        assert_eq!(params.bytes_per_sector, 512);
        assert!(params.sectors_per_cluster >= 1);
        assert!(params.cluster_count > 0);
    }

    #[test]
    fn test_format_exfat_writes_boot_sector() {
        // Test that formatting writes a valid boot sector
        let size = 2 * 1024 * 1024u64; // 2 MB
        let mut buffer = vec![0u8; size as usize];

        {
            let cursor = Cursor::new(&mut buffer[..]);
            let options = ExFatFormatOptions::new();
            let params = calculate_layout(size, &options).unwrap();

            let mut cursor = cursor;
            // Just write boot region, don't open
            write_boot_region(&mut cursor, &params, 0).unwrap();
            write_boot_region(&mut cursor, &params, 12).unwrap();
        }

        // Verify boot sector - jump instruction at offset 0, signature at offset 3
        assert_eq!(&buffer[0..3], &[0xEB, 0x76, 0x90]); // Jump instruction
        assert_eq!(&buffer[3..11], b"EXFAT   "); // Filesystem signature
        assert_eq!(buffer[510], 0x55); // Boot signature low
        assert_eq!(buffer[511], 0xAA); // Boot signature high
    }

    #[test]
    fn test_upcase_table_generation() {
        let (data, checksum) = generate_compressed_upcase_table();
        assert!(!data.is_empty());
        assert!(checksum != 0);
        // Basic sanity check - table should be reasonably small when compressed
        assert!(data.len() < 1000);
    }

    #[test]
    fn test_format_options_validation() {
        // Invalid sector size
        let options = ExFatFormatOptions::new().with_sector_size(256);
        assert!(options.validate().is_err());

        // Invalid FAT count
        let mut options = ExFatFormatOptions::new();
        options.fat_count = 3;
        assert!(options.validate().is_err());

        // Valid options
        let options = ExFatFormatOptions::new();
        assert!(options.validate().is_ok());
    }

    #[test]
    fn test_volume_too_small() {
        let size = 512 * 1024u64; // 512 KB - too small
        let options = ExFatFormatOptions::new();
        assert!(calculate_layout(size, &options).is_err());
    }

    #[test]
    fn test_format_exfat_full() {
        // Full integration test: format and open filesystem
        let size = 2 * 1024 * 1024u64; // 2 MB
        let mut buffer = vec![0u8; size as usize];
        let cursor = Cursor::new(&mut buffer[..]);

        let options = ExFatFormatOptions::new().with_label("TEST");
        let fs = format_exfat(cursor, size, &options).unwrap();

        let info = fs.info();
        assert_eq!(info.bytes_per_sector, 512);
        assert!(info.cluster_count > 0);
        assert!(info.root_cluster >= 2);
    }
}
