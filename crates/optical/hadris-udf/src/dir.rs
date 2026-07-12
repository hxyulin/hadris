//! UDF Directory operations

use alloc::string::String;
use alloc::vec::Vec;

use crate::descriptor::{DescriptorTag, LongAllocationDescriptor, TagIdentifier};
use crate::error::{UdfError, UdfResult};

/// A UDF directory entry
#[derive(Debug, Clone)]
pub struct UdfDirEntry {
    /// Entry name
    pub name: String,
    /// Whether this is a directory
    pub is_directory: bool,
    /// File size in bytes
    pub size: u64,
    /// ICB location for this entry
    pub icb: LongAllocationDescriptor,
    /// File characteristics
    pub characteristics: FileCharacteristics,
}

impl UdfDirEntry {
    /// Get the entry name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if this is a directory
    pub fn is_dir(&self) -> bool {
        self.is_directory
    }

    /// Check if this is a regular file
    pub fn is_file(&self) -> bool {
        !self.is_directory
    }

    /// Check if this is hidden
    pub fn is_hidden(&self) -> bool {
        self.characteristics.contains(FileCharacteristics::HIDDEN)
    }

    /// Check if this is a parent directory reference (..)
    pub fn is_parent(&self) -> bool {
        self.characteristics.contains(FileCharacteristics::PARENT)
    }
}

/// File Identifier Descriptor (ECMA-167 4/14.4)
///
/// Note: Due to Rust alignment rules, this struct is 40 bytes in memory,
/// but the on-disk format is 38 bytes. Use `from_bytes` for parsing.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FileIdentifierDescriptor {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// File Version Number
    pub file_version_number: u16,
    /// File Characteristics
    pub file_characteristics: u8,
    /// Length of File Identifier
    pub file_identifier_length: u8,
    /// ICB (Information Control Block)
    pub icb: LongAllocationDescriptor,
    /// Length of Implementation Use
    pub implementation_use_length: u16,
    // Followed by:
    // - Implementation Use (implementation_use_length bytes)
    // - File Identifier (file_identifier_length bytes)
    // - Padding to 4-byte boundary
}

unsafe impl bytemuck::Zeroable for FileIdentifierDescriptor {}
unsafe impl bytemuck::Pod for FileIdentifierDescriptor {}

impl FileIdentifierDescriptor {
    /// Base size without variable-length fields (on-disk format)
    /// Note: The Rust struct is 40 bytes due to alignment padding,
    /// so we parse fields manually in from_bytes()
    pub const BASE_SIZE: usize = 38;

    /// Calculate total size of this FID
    pub fn total_size(&self) -> usize {
        let base = Self::BASE_SIZE;
        let variable =
            self.implementation_use_length as usize + self.file_identifier_length as usize;
        // Pad to 4-byte boundary
        (base + variable + 3) & !3
    }

    /// Parse from a byte buffer
    pub fn from_bytes(data: &[u8]) -> UdfResult<(Self, &[u8])> {
        if data.len() < Self::BASE_SIZE {
            return Err(UdfError::Io(hadris_io::Error::new(
                hadris_io::ErrorKind::UnexpectedEof,
                "buffer too small for FID",
            )));
        }

        // Parse fields manually due to alignment differences between
        // on-disk format (38 bytes packed) and Rust struct (40 bytes aligned)
        let tag: DescriptorTag = *bytemuck::from_bytes(&data[0..16]);
        let file_version_number = u16::from_le_bytes([data[16], data[17]]);
        let file_characteristics = data[18];
        let file_identifier_length = data[19];
        let icb: LongAllocationDescriptor = *bytemuck::from_bytes(&data[20..36]);
        let implementation_use_length = u16::from_le_bytes([data[36], data[37]]);

        let fid = Self {
            tag,
            file_version_number,
            file_characteristics,
            file_identifier_length,
            icb,
            implementation_use_length,
        };

        if fid.tag.identifier() != TagIdentifier::FileIdentifierDescriptor {
            return Err(UdfError::InvalidTag {
                expected: TagIdentifier::FileIdentifierDescriptor.to_u16(),
                found: fid.tag.tag_identifier,
            });
        }

        let total_size = fid.total_size();
        if data.len() < total_size {
            return Err(UdfError::Io(hadris_io::Error::new(
                hadris_io::ErrorKind::UnexpectedEof,
                "buffer too small for FID data",
            )));
        }

        Ok((fid, &data[Self::BASE_SIZE..total_size]))
    }
}

bitflags::bitflags! {
    /// File characteristics (ECMA-167 4/14.4.3)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileCharacteristics: u8 {
        /// Existence flag (file exists if set)
        const EXISTENCE = 0x01;
        /// Directory flag
        const DIRECTORY = 0x02;
        /// Deleted flag
        const DELETED = 0x04;
        /// Parent directory entry (..)
        const PARENT = 0x08;
        /// Metadata flag
        const METADATA = 0x10;
        /// Hidden flag (UDF extension)
        const HIDDEN = 0x20;
    }
}

/// UDF Directory handle
pub struct UdfDir {
    /// Directory entries
    entries: Vec<UdfDirEntry>,
}

impl UdfDir {
    /// Create a new directory from parsed entries
    pub(crate) fn new(entries: Vec<UdfDirEntry>) -> Self {
        Self { entries }
    }

    /// Get directory entries
    pub fn entries(&self) -> impl Iterator<Item = &UdfDirEntry> {
        self.entries.iter().filter(|e| !e.is_parent())
    }

    /// Get all entries including parent
    pub fn all_entries(&self) -> impl Iterator<Item = &UdfDirEntry> {
        self.entries.iter()
    }

    /// Find an entry by name
    pub fn find(&self, name: &str) -> Option<&UdfDirEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Get the number of entries (excluding parent)
    pub fn len(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_parent()).count()
    }

    /// Check if directory is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Decode a UDF filename from CS0 (OSTA Compressed Unicode)
pub fn decode_filename(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }

    let compression_id = data[0];
    let content = &data[1..];

    match compression_id {
        8 => {
            // 8-bit characters (Latin-1/UTF-8)
            String::from_utf8_lossy(content).into_owned()
        }
        16 => {
            // 16-bit characters (UTF-16 BE)
            let mut result = String::new();
            for chunk in content.chunks(2) {
                if chunk.len() == 2 {
                    let code_unit = u16::from_be_bytes([chunk[0], chunk[1]]);
                    if let Some(c) = char::from_u32(code_unit as u32) {
                        result.push(c);
                    }
                }
            }
            result
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Rust struct is 40 bytes (with alignment padding), but on-disk is 38 bytes
    static_assertions::const_assert_eq!(size_of::<FileIdentifierDescriptor>(), 40);

    #[test]
    fn test_file_characteristics() {
        let chars = FileCharacteristics::DIRECTORY | FileCharacteristics::EXISTENCE;
        assert!(chars.contains(FileCharacteristics::DIRECTORY));
        assert!(!chars.contains(FileCharacteristics::HIDDEN));
    }

    #[test]
    fn test_decode_filename_8bit() {
        let data = [8, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(decode_filename(&data), "hello");
    }

    #[test]
    fn test_decode_filename_16bit() {
        // UTF-16 BE: "hi"
        let data = [16, 0x00, b'h', 0x00, b'i'];
        assert_eq!(decode_filename(&data), "hi");
    }
}
