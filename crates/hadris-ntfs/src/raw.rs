//! On-disk structures for NTFS filesystems.
//!
//! Only the boot sector is defined as a full bytemuck-parsable struct.
//! MFT records, attributes, and index records are read into byte buffers
//! and parsed field-by-field, because fixup processing modifies the buffer
//! in place before attribute parsing begins.

use hadris_common::types::{
    endian::LittleEndian,
    number::{U16, U64},
};

/// NTFS boot sector (512 bytes).
///
/// Layout follows the standard NTFS BPB at the start of the volume.
/// The OEM ID must be `"NTFS    "` (space-padded to 8 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawNtfsBootSector {
    /// Jump instruction (3 bytes)
    pub jump: [u8; 3],
    /// OEM identifier — must be `b"NTFS    "` for a valid NTFS volume
    pub oem_id: [u8; 8],
    /// Bytes per sector (usually 512)
    pub bytes_per_sector: U16<LittleEndian>,
    /// Sectors per cluster (power of two)
    pub sectors_per_cluster: u8,
    /// Reserved sectors (always 0 on NTFS)
    pub _reserved: [u8; 2],
    /// Unused legacy BPB fields (FAT count, root entries, total_sec16)
    pub _zero1: [u8; 5],
    /// Media descriptor byte (0xF8 for fixed disk)
    pub media_descriptor: u8,
    /// Unused (sectors-per-FAT in FAT, always 0 on NTFS)
    pub _zero2: [u8; 2],
    /// Sectors per track (CHS geometry)
    pub sectors_per_track: [u8; 2],
    /// Number of heads (CHS geometry)
    pub num_heads: [u8; 2],
    /// Hidden sectors before this volume
    pub hidden_sectors: [u8; 4],
    /// Unused legacy (total_sectors_32 + extra)
    pub _zero3: [u8; 8],
    /// Total number of sectors in the volume
    pub total_sectors: U64<LittleEndian>,
    /// Logical Cluster Number (LCN) of the start of `$MFT`
    pub mft_lcn: U64<LittleEndian>,
    /// LCN of the start of `$MFTMirr`
    pub mft_mirr_lcn: U64<LittleEndian>,
    /// Encoded MFT record size.
    ///
    /// If positive (as i8): size in clusters.
    /// If negative: size = 2^|value| bytes (e.g. -10 → 1024).
    pub clusters_per_mft_record: u8,
    pub _unused1: [u8; 3],
    /// Encoded index record size (same encoding as MFT record size).
    pub clusters_per_index_record: u8,
    pub _unused2: [u8; 3],
    /// Volume serial number
    pub volume_serial: U64<LittleEndian>,
    /// Checksum (unused by Windows)
    pub checksum: [u8; 4],
    /// Bootstrap code
    pub bootstrap: [u8; 426],
    /// End-of-sector marker (0xAA55)
    pub signature: U16<LittleEndian>,
}

// Safety: all fields are byte arrays or repr(transparent) wrappers over byte arrays
unsafe impl bytemuck::NoUninit for RawNtfsBootSector {}
unsafe impl bytemuck::Zeroable for RawNtfsBootSector {}
unsafe impl bytemuck::AnyBitPattern for RawNtfsBootSector {}

// Verified: size_of::<RawNtfsBootSector>() == 512
const _: () = assert!(core::mem::size_of::<RawNtfsBootSector>() == 512);
