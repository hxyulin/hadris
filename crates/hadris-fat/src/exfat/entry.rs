//! exFAT Directory Entry structures.
//!
//! exFAT uses directory entry sets consisting of multiple 32-byte entries:
//! - File Directory Entry (0x85): Primary entry with attributes and timestamps
//! - Stream Extension Entry (0xC0): File location, size, and NoFatChain flag
//! - File Name Entry (0xC1): 15 UTF-16 characters per entry
//!
//! A complete file entry set has 3-19 entries total.

use alloc::string::String;
use alloc::vec::Vec;

use hadris_common::types::{
    endian::{Endian, LittleEndian},
    number::{U16, U32, U64},
};

use super::time::ExFatTimestamp;

/// Entry type codes
pub mod entry_type {
    /// End of directory marker (unused entry)
    pub const END_OF_DIRECTORY: u8 = 0x00;
    /// Allocation bitmap entry
    pub const ALLOCATION_BITMAP: u8 = 0x81;
    /// Up-case table entry
    pub const UPCASE_TABLE: u8 = 0x82;
    /// Volume label entry
    pub const VOLUME_LABEL: u8 = 0x83;
    /// File directory entry
    pub const FILE_DIRECTORY: u8 = 0x85;
    /// Volume GUID entry (optional)
    pub const VOLUME_GUID: u8 = 0xA0;
    /// TexFAT padding entry
    pub const TEXFAT_PADDING: u8 = 0xA1;
    /// Windows CE access control entry
    pub const ACCESS_CONTROL: u8 = 0xA2;
    /// Stream extension entry
    pub const STREAM_EXTENSION: u8 = 0xC0;
    /// File name entry
    pub const FILE_NAME: u8 = 0xC1;
    /// Deleted file directory entry
    pub const DELETED_FILE: u8 = 0x05;
}

/// File attribute flags (compatible with FAT but with extensions)
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileAttributes: u16 {
        /// File is read-only
        const READ_ONLY = 0x0001;
        /// File is hidden
        const HIDDEN = 0x0002;
        /// File is a system file
        const SYSTEM = 0x0004;
        /// Entry is a directory
        const DIRECTORY = 0x0010;
        /// File has been modified since last backup
        const ARCHIVE = 0x0020;
    }
}

/// Raw File Directory Entry (0x85) - 32 bytes.
///
/// This is the primary entry in a file entry set.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFileDirectoryEntry {
    /// Entry type (0x85 for file, 0x05 for deleted)
    pub entry_type: u8,
    /// Number of secondary entries (1-18)
    pub secondary_count: u8,
    /// Checksum of all entries in the set
    pub set_checksum: U16<LittleEndian>,
    /// File attributes
    pub file_attributes: U16<LittleEndian>,
    /// Reserved
    pub reserved1: U16<LittleEndian>,
    /// Creation timestamp
    pub create_timestamp: U32<LittleEndian>,
    /// Last modified timestamp
    pub last_modified_timestamp: U32<LittleEndian>,
    /// Last accessed timestamp
    pub last_accessed_timestamp: U32<LittleEndian>,
    /// Creation time 10ms increment (0-199)
    pub create_10ms_increment: u8,
    /// Last modified time 10ms increment (0-199)
    pub last_modified_10ms_increment: u8,
    /// Creation UTC offset (in 15-minute increments, 0x80 = invalid)
    pub create_utc_offset: u8,
    /// Last modified UTC offset
    pub last_modified_utc_offset: u8,
    /// Last accessed UTC offset
    pub last_accessed_utc_offset: u8,
    /// Reserved
    pub reserved2: [u8; 7],
}

unsafe impl bytemuck::NoUninit for RawFileDirectoryEntry {}
unsafe impl bytemuck::Zeroable for RawFileDirectoryEntry {}
unsafe impl bytemuck::AnyBitPattern for RawFileDirectoryEntry {}

/// Raw Stream Extension Entry (0xC0) - 32 bytes.
///
/// Contains file location and size information.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawStreamExtensionEntry {
    /// Entry type (0xC0)
    pub entry_type: u8,
    /// General secondary flags
    /// - Bit 0: AllocationPossible (always 1 for stream extension)
    /// - Bit 1: NoFatChain (1 = contiguous, don't use FAT)
    pub general_secondary_flags: u8,
    /// Reserved
    pub reserved1: u8,
    /// Name length in Unicode characters (1-255)
    pub name_length: u8,
    /// Hash of the up-cased filename
    pub name_hash: U16<LittleEndian>,
    /// Reserved
    pub reserved2: U16<LittleEndian>,
    /// Valid data length (actual file content size)
    pub valid_data_length: U64<LittleEndian>,
    /// Reserved
    pub reserved3: U32<LittleEndian>,
    /// First cluster of file data
    pub first_cluster: U32<LittleEndian>,
    /// Data length (allocated size, may be larger than valid_data_length)
    pub data_length: U64<LittleEndian>,
}

unsafe impl bytemuck::NoUninit for RawStreamExtensionEntry {}
unsafe impl bytemuck::Zeroable for RawStreamExtensionEntry {}
unsafe impl bytemuck::AnyBitPattern for RawStreamExtensionEntry {}

impl RawStreamExtensionEntry {
    /// Check if the file is stored contiguously (NoFatChain flag)
    pub fn is_contiguous(&self) -> bool {
        (self.general_secondary_flags & 0x02) != 0
    }
}

/// Raw File Name Entry (0xC1) - 32 bytes.
///
/// Contains up to 15 UTF-16 characters of the filename.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFileNameEntry {
    /// Entry type (0xC1)
    pub entry_type: u8,
    /// General secondary flags (usually 0)
    pub general_secondary_flags: u8,
    /// Filename characters (15 UTF-16LE code units)
    pub file_name: [u8; 30],
}

unsafe impl bytemuck::NoUninit for RawFileNameEntry {}
unsafe impl bytemuck::Zeroable for RawFileNameEntry {}
unsafe impl bytemuck::AnyBitPattern for RawFileNameEntry {}

/// Raw Allocation Bitmap Entry (0x81) - 32 bytes.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawAllocationBitmapEntry {
    /// Entry type (0x81)
    pub entry_type: u8,
    /// Bitmap flags
    /// - Bit 0: BitmapIdentifier (0 = first bitmap, 1 = second bitmap for TexFAT)
    pub bitmap_flags: u8,
    /// Reserved
    pub reserved: [u8; 18],
    /// First cluster of the bitmap
    pub first_cluster: U32<LittleEndian>,
    /// Data length (size of bitmap in bytes)
    pub data_length: U64<LittleEndian>,
}

unsafe impl bytemuck::NoUninit for RawAllocationBitmapEntry {}
unsafe impl bytemuck::Zeroable for RawAllocationBitmapEntry {}
unsafe impl bytemuck::AnyBitPattern for RawAllocationBitmapEntry {}

/// Raw Up-case Table Entry (0x82) - 32 bytes.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawUpcaseTableEntry {
    /// Entry type (0x82)
    pub entry_type: u8,
    /// Reserved
    pub reserved1: [u8; 3],
    /// Table checksum
    pub table_checksum: U32<LittleEndian>,
    /// Reserved
    pub reserved2: [u8; 12],
    /// First cluster of the up-case table
    pub first_cluster: U32<LittleEndian>,
    /// Data length (size of table in bytes)
    pub data_length: U64<LittleEndian>,
}

unsafe impl bytemuck::NoUninit for RawUpcaseTableEntry {}
unsafe impl bytemuck::Zeroable for RawUpcaseTableEntry {}
unsafe impl bytemuck::AnyBitPattern for RawUpcaseTableEntry {}

/// Raw Volume Label Entry (0x83) - 32 bytes.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawVolumeLabelEntry {
    /// Entry type (0x83)
    pub entry_type: u8,
    /// Character count (0-11)
    pub character_count: u8,
    /// Volume label (up to 11 UTF-16LE characters)
    pub volume_label: [u8; 22],
    /// Reserved
    pub reserved: [u8; 8],
}

unsafe impl bytemuck::NoUninit for RawVolumeLabelEntry {}
unsafe impl bytemuck::Zeroable for RawVolumeLabelEntry {}
unsafe impl bytemuck::AnyBitPattern for RawVolumeLabelEntry {}

/// Union of all directory entry types.
#[repr(C)]
#[derive(Clone, Copy)]
pub union RawDirectoryEntry {
    /// Entry type byte (first byte of any entry)
    pub entry_type: u8,
    /// File directory entry
    pub file: RawFileDirectoryEntry,
    /// Stream extension entry
    pub stream: RawStreamExtensionEntry,
    /// File name entry
    pub name: RawFileNameEntry,
    /// Allocation bitmap entry
    pub bitmap: RawAllocationBitmapEntry,
    /// Up-case table entry
    pub upcase: RawUpcaseTableEntry,
    /// Volume label entry
    pub label: RawVolumeLabelEntry,
    /// Raw bytes
    pub bytes: [u8; 32],
}

unsafe impl bytemuck::NoUninit for RawDirectoryEntry {}
unsafe impl bytemuck::Zeroable for RawDirectoryEntry {}
unsafe impl bytemuck::AnyBitPattern for RawDirectoryEntry {}

/// Parsed exFAT file entry with all data from the entry set.
#[derive(Debug, Clone)]
pub struct ExFatFileEntry {
    /// File name (Unicode, up to 255 characters)
    pub name: String,
    /// File attributes
    pub attributes: FileAttributes,
    /// First cluster of file data
    pub first_cluster: u32,
    /// Total data length (allocated size)
    pub data_length: u64,
    /// Valid data length (actual content size)
    pub valid_data_length: u64,
    /// Whether the file is stored contiguously (NoFatChain flag)
    pub no_fat_chain: bool,
    /// Name hash for quick comparison
    pub name_hash: u16,
    /// Creation timestamp
    pub created: ExFatTimestamp,
    /// Last modified timestamp
    pub modified: ExFatTimestamp,
    /// Last accessed timestamp
    pub accessed: ExFatTimestamp,
    /// Parent directory first cluster (for write operations)
    pub(crate) parent_cluster: u32,
    /// Byte offset of this entry set within the parent directory
    pub(crate) entry_offset: u64,
}

impl ExFatFileEntry {
    /// Check if this entry is a directory
    pub fn is_directory(&self) -> bool {
        self.attributes.contains(FileAttributes::DIRECTORY)
    }

    /// Check if this entry is a regular file
    pub fn is_file(&self) -> bool {
        !self.is_directory()
    }

    /// Get the file size in bytes
    pub fn size(&self) -> u64 {
        self.valid_data_length
    }
}

/// Compute the checksum for a file entry set.
///
/// The checksum is computed over all entries in the set, with the
/// set_checksum field (bytes 2-3 of the primary entry) set to zero.
pub fn compute_entry_set_checksum(entries: &[RawDirectoryEntry]) -> u16 {
    let mut checksum: u16 = 0;

    for (entry_idx, entry) in entries.iter().enumerate() {
        let bytes = unsafe { &entry.bytes };

        for (byte_idx, &byte) in bytes.iter().enumerate() {
            // Skip the set_checksum field in the primary entry (bytes 2-3)
            if entry_idx == 0 && (byte_idx == 2 || byte_idx == 3) {
                continue;
            }

            // Rotate right and add
            checksum = checksum.rotate_right(1).wrapping_add(byte as u16);
        }
    }

    checksum
}

/// Parse a complete entry set starting with a File Directory Entry.
///
/// Returns the parsed file entry and the number of directory entries consumed.
pub fn parse_entry_set(entries: &[RawDirectoryEntry]) -> Option<(ExFatFileEntry, usize)> {
    if entries.is_empty() {
        return None;
    }

    // First entry must be a File Directory Entry
    let primary = unsafe { &entries[0].file };
    if primary.entry_type != entry_type::FILE_DIRECTORY {
        return None;
    }

    let secondary_count = primary.secondary_count as usize;
    if secondary_count < 2 || secondary_count > 18 {
        return None; // Invalid secondary count
    }

    let total_entries = 1 + secondary_count;
    if entries.len() < total_entries {
        return None; // Not enough entries
    }

    // Second entry must be a Stream Extension Entry
    let stream = unsafe { &entries[1].stream };
    if stream.entry_type != entry_type::STREAM_EXTENSION {
        return None;
    }

    // Collect filename from File Name Entries
    let name_length = stream.name_length as usize;
    let mut name_chars: Vec<u16> = Vec::with_capacity(name_length);

    for i in 2..total_entries {
        let name_entry = unsafe { &entries[i].name };
        if name_entry.entry_type != entry_type::FILE_NAME {
            return None; // Expected file name entry
        }

        // Extract UTF-16 characters from the entry
        for j in 0..15 {
            if name_chars.len() >= name_length {
                break;
            }
            let char_offset = j * 2;
            let code_unit = u16::from_le_bytes([
                name_entry.file_name[char_offset],
                name_entry.file_name[char_offset + 1],
            ]);
            name_chars.push(code_unit);
        }
    }

    // Convert UTF-16 to String
    let name = String::from_utf16_lossy(&name_chars);

    // Parse timestamps
    let created = ExFatTimestamp::new(
        primary.create_timestamp.get(),
        primary.create_10ms_increment,
        primary.create_utc_offset,
    );
    let modified = ExFatTimestamp::new(
        primary.last_modified_timestamp.get(),
        primary.last_modified_10ms_increment,
        primary.last_modified_utc_offset,
    );
    let accessed = ExFatTimestamp::new(
        primary.last_accessed_timestamp.get(),
        0, // No 10ms increment for accessed time
        primary.last_accessed_utc_offset,
    );

    let entry = ExFatFileEntry {
        name,
        attributes: FileAttributes::from_bits_truncate(primary.file_attributes.get()),
        first_cluster: stream.first_cluster.get(),
        data_length: stream.data_length.get(),
        valid_data_length: stream.valid_data_length.get(),
        no_fat_chain: stream.is_contiguous(),
        name_hash: stream.name_hash.get(),
        created,
        modified,
        accessed,
        parent_cluster: 0, // Set by caller
        entry_offset: 0,   // Set by caller
    };

    Some((entry, total_entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::size_of;
    use static_assertions::const_assert_eq;

    const_assert_eq!(size_of::<RawFileDirectoryEntry>(), 32);
    const_assert_eq!(size_of::<RawStreamExtensionEntry>(), 32);
    const_assert_eq!(size_of::<RawFileNameEntry>(), 32);
    const_assert_eq!(size_of::<RawAllocationBitmapEntry>(), 32);
    const_assert_eq!(size_of::<RawUpcaseTableEntry>(), 32);
    const_assert_eq!(size_of::<RawVolumeLabelEntry>(), 32);
    const_assert_eq!(size_of::<RawDirectoryEntry>(), 32);
}
