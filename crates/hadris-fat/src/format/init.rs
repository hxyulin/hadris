//! FAT volume initialization routines.
//!
//! This module handles writing the boot sector, FAT tables, and root directory
//! when formatting a new FAT volume.

use alloc::vec;

use crate::error::Result;
use crate::io::{Read, Seek, SeekFrom, Write};
use crate::write::FatDateTime;
use crate::{
    DirEntryAttrFlags, FatType, RawBpb, RawBpbExt16, RawBpbExt32, RawFileEntry, RawFsInfo,
    FSINFO_LEAD_SIG, FSINFO_STRUC_SIG, FSINFO_TRAIL_SIG,
};

use hadris_common::types::endian::{Endian, LittleEndian};
use hadris_common::types::number::{U16, U32};

use super::calc::FormatParams;
use super::options::FormatOptions;

/// Write all required structures to format a FAT volume.
pub fn initialize_volume<DATA: Read + Write + Seek>(
    data: &mut DATA,
    options: &FormatOptions,
    params: &FormatParams,
) -> Result<()> {
    // Zero out reserved area
    zero_reserved_area(data, params)?;

    // Write boot sector
    write_boot_sector(data, options, params)?;

    // Write FSInfo and backup boot sector (FAT32 only)
    if params.fat_type == FatType::Fat32 {
        write_fsinfo(data, params)?;
        write_backup_boot_sector(data, options, params)?;
    }

    // Initialize FAT tables
    initialize_fat_tables(data, params)?;

    // Initialize root directory
    initialize_root_directory(data, options, params)?;

    Ok(())
}

/// Zero out the reserved area.
fn zero_reserved_area<DATA: Write + Seek>(data: &mut DATA, params: &FormatParams) -> Result<()> {
    let reserved_bytes = params.reserved_sectors as usize * params.sector_size;
    data.seek(SeekFrom::Start(0))?;

    // Write zeros in chunks
    let zeros = vec![0u8; params.sector_size.min(4096)];
    let mut remaining = reserved_bytes;
    while remaining > 0 {
        let to_write = remaining.min(zeros.len());
        data.write_all(&zeros[..to_write])?;
        remaining -= to_write;
    }

    Ok(())
}

/// Write the boot sector (sector 0).
fn write_boot_sector<DATA: Write + Seek>(
    data: &mut DATA,
    options: &FormatOptions,
    params: &FormatParams,
) -> Result<()> {
    data.seek(SeekFrom::Start(0))?;

    // Build the common BPB
    let total_sectors_16 = if params.total_sectors < 0x10000 && params.fat_type != FatType::Fat32 {
        params.total_sectors as u16
    } else {
        0
    };

    let total_sectors_32 = if total_sectors_16 == 0 {
        params.total_sectors
    } else {
        0
    };

    let bpb = RawBpb {
        jump: [0xEB, 0x58, 0x90], // Short jump + NOP
        oem_name: *options.oem_name.as_bytes(),
        bytes_per_sector: U16::<LittleEndian>::new(params.sector_size as u16),
        sectors_per_cluster: params.sectors_per_cluster,
        reserved_sector_count: U16::<LittleEndian>::new(params.reserved_sectors),
        fat_count: params.fat_count,
        root_entry_count: params.root_entry_count.to_le_bytes(),
        total_sectors_16: total_sectors_16.to_le_bytes(),
        media_type: params.media_type,
        sectors_per_fat_16: if params.fat_type == FatType::Fat32 {
            [0, 0]
        } else {
            (params.sectors_per_fat as u16).to_le_bytes()
        },
        sectors_per_track: 63u16.to_le_bytes(), // Standard value
        num_heads: 255u16.to_le_bytes(),        // Standard value
        hidden_sector_count: params.hidden_sectors.to_le_bytes(),
        total_sectors_32: total_sectors_32.to_le_bytes(),
    };

    data.write_all(bytemuck::bytes_of(&bpb))?;

    // Generate volume ID if not provided
    let volume_id = options.volume_id.unwrap_or_else(generate_volume_id);

    // Write extended boot sector based on FAT type
    match params.fat_type {
        FatType::Fat12 | FatType::Fat16 => {
            let fs_type = if params.fat_type == FatType::Fat12 {
                *b"FAT12   "
            } else {
                *b"FAT16   "
            };

            let bpb_ext16 = RawBpbExt16 {
                drive_number: 0x80,
                reserved1: 0,
                ext_boot_signature: 0x29,
                volume_id: volume_id.to_le_bytes(),
                volume_label: *options.volume_label.as_bytes(),
                fs_type,
                padding1: [0; 448],
                signature_word: 0xAA55u16.to_le_bytes(),
            };
            data.write_all(bytemuck::bytes_of(&bpb_ext16))?;
        }
        FatType::Fat32 => {
            let bpb_ext32 = RawBpbExt32 {
                sectors_per_fat_32: U32::<LittleEndian>::new(params.sectors_per_fat),
                ext_flags: [0, 0], // Mirroring enabled, FAT 0 active
                version: [0, 0],   // Version 0.0
                root_cluster: U32::<LittleEndian>::new(2), // Root dir at cluster 2
                fs_info_sector: U16::<LittleEndian>::new(1),
                boot_sector: 6u16.to_le_bytes(), // Backup at sector 6
                reserved: [0; 12],
                drive_number: 0x80,
                reserved1: 0,
                ext_boot_signature: 0x29,
                volume_id: volume_id.to_le_bytes(),
                volume_label: *options.volume_label.as_bytes(),
                fs_type: *b"FAT32   ",
                padding1: [0; 420],
                signature_word: U16::<LittleEndian>::new(0xAA55),
            };
            data.write_all(bytemuck::bytes_of(&bpb_ext32))?;
        }
    }

    Ok(())
}

/// Write the FSInfo sector (FAT32 only, sector 1).
fn write_fsinfo<DATA: Write + Seek>(data: &mut DATA, params: &FormatParams) -> Result<()> {
    data.seek(SeekFrom::Start(params.sector_size as u64))?;

    // Free clusters = total - 1 (root directory takes cluster 2)
    let free_count = params.cluster_count.saturating_sub(1);

    let fsinfo = RawFsInfo {
        signature: FSINFO_LEAD_SIG.to_le_bytes(),
        reserved1: [0; 480],
        structure_signature: FSINFO_STRUC_SIG.to_le_bytes(),
        free_count: U32::<LittleEndian>::new(free_count),
        next_free: U32::<LittleEndian>::new(3), // Next free after root dir cluster
        reserved2: [0; 12],
        trail_signature: U32::<LittleEndian>::new(FSINFO_TRAIL_SIG),
    };

    data.write_all(bytemuck::bytes_of(&fsinfo))?;

    // Pad to sector size
    let written = core::mem::size_of::<RawFsInfo>();
    let padding = params.sector_size - written;
    if padding > 0 {
        data.write_all(&vec![0u8; padding])?;
    }

    Ok(())
}

/// Write the backup boot sector (FAT32 only, sector 6).
fn write_backup_boot_sector<DATA: Read + Write + Seek>(
    data: &mut DATA,
    _options: &FormatOptions,
    params: &FormatParams,
) -> Result<()> {
    // Read the primary boot sector
    data.seek(SeekFrom::Start(0))?;
    let mut boot_sector = vec![0u8; params.sector_size];
    data.read_exact(&mut boot_sector)?;

    // Write to sector 6
    data.seek(SeekFrom::Start(6 * params.sector_size as u64))?;
    data.write_all(&boot_sector)?;

    // Also write backup FSInfo to sector 7
    data.seek(SeekFrom::Start(params.sector_size as u64))?;
    let mut fsinfo_sector = vec![0u8; params.sector_size];
    data.read_exact(&mut fsinfo_sector)?;

    data.seek(SeekFrom::Start(7 * params.sector_size as u64))?;
    data.write_all(&fsinfo_sector)?;

    Ok(())
}

/// Initialize the FAT tables with media type marker and end-of-chain markers.
fn initialize_fat_tables<DATA: Write + Seek>(
    data: &mut DATA,
    params: &FormatParams,
) -> Result<()> {
    let fat_start = params.reserved_sectors as u64 * params.sector_size as u64;
    let fat_bytes = params.sectors_per_fat as usize * params.sector_size;

    // Initialize each FAT copy
    for fat_idx in 0..params.fat_count {
        let fat_offset = fat_start + (fat_idx as u64 * fat_bytes as u64);
        data.seek(SeekFrom::Start(fat_offset))?;

        // Zero the FAT first
        let zeros = vec![0u8; fat_bytes];
        data.write_all(&zeros)?;

        // Write the reserved entries (entries 0 and 1)
        data.seek(SeekFrom::Start(fat_offset))?;

        match params.fat_type {
            FatType::Fat12 => {
                // FAT12: 3 bytes for entries 0 and 1
                // Entry 0: media type | 0xF00
                // Entry 1: 0xFFF (end of chain)
                let entry0 = params.media_type as u16 | 0xF00;
                let entry1: u16 = 0xFFF;
                // Pack two 12-bit entries into 3 bytes
                let bytes = [
                    entry0 as u8,
                    ((entry0 >> 8) as u8 & 0x0F) | ((entry1 << 4) as u8),
                    (entry1 >> 4) as u8,
                ];
                data.write_all(&bytes)?;
            }
            FatType::Fat16 => {
                // FAT16: 4 bytes for entries 0 and 1
                let entry0 = 0xFF00u16 | params.media_type as u16;
                let entry1 = 0xFFFFu16;
                data.write_all(&entry0.to_le_bytes())?;
                data.write_all(&entry1.to_le_bytes())?;
            }
            FatType::Fat32 => {
                // FAT32: 8 bytes for entries 0 and 1, plus entry 2 (root dir)
                let entry0 = 0x0FFFFF00u32 | params.media_type as u32;
                let entry1 = 0x0FFFFFFFu32;
                let entry2 = 0x0FFFFFF8u32; // End of chain for root directory
                data.write_all(&entry0.to_le_bytes())?;
                data.write_all(&entry1.to_le_bytes())?;
                data.write_all(&entry2.to_le_bytes())?;
            }
        }
    }

    Ok(())
}

/// Initialize the root directory.
fn initialize_root_directory<DATA: Write + Seek>(
    data: &mut DATA,
    options: &FormatOptions,
    params: &FormatParams,
) -> Result<()> {
    let root_dir_offset = match params.fat_type {
        FatType::Fat12 | FatType::Fat16 => {
            // Fixed root directory after FAT tables
            let fat_end = params.reserved_sectors as u64 * params.sector_size as u64
                + params.fat_count as u64 * params.sectors_per_fat as u64 * params.sector_size as u64;
            fat_end
        }
        FatType::Fat32 => {
            // Root directory in cluster 2
            params.data_start_sector as u64 * params.sector_size as u64
        }
    };

    data.seek(SeekFrom::Start(root_dir_offset))?;

    // Zero the root directory
    let root_dir_size = match params.fat_type {
        FatType::Fat12 | FatType::Fat16 => params.root_entry_count as usize * 32,
        FatType::Fat32 => params.sectors_per_cluster as usize * params.sector_size,
    };
    let zeros = vec![0u8; root_dir_size];
    data.write_all(&zeros)?;

    // Write volume label entry if provided and not "NO NAME"
    if options.volume_label.as_bytes() != b"NO NAME    " {
        data.seek(SeekFrom::Start(root_dir_offset))?;

        let now = FatDateTime::now();
        let (date, time, _) = now.to_raw();

        let label_entry = RawFileEntry {
            name: *options.volume_label.as_bytes(),
            attributes: DirEntryAttrFlags::VOLUME_ID.bits(),
            reserved: 0,
            creation_time_tenth: 0,
            creation_time: time.to_le_bytes(),
            creation_date: date.to_le_bytes(),
            last_access_date: date.to_le_bytes(),
            first_cluster_high: U16::<LittleEndian>::new(0),
            last_write_time: time.to_le_bytes(),
            last_write_date: date.to_le_bytes(),
            first_cluster_low: U16::<LittleEndian>::new(0),
            size: U32::<LittleEndian>::new(0),
        };

        data.write_all(bytemuck::bytes_of(&label_entry))?;
    }

    Ok(())
}

/// Generate a pseudo-random volume ID based on current time.
fn generate_volume_id() -> u32 {
    #[cfg(feature = "std")]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs() as u32;
        let nanos = duration.subsec_nanos();
        secs ^ nanos
    }
    #[cfg(not(feature = "std"))]
    {
        // Fallback: use a fixed value or counter
        0x12345678
    }
}
