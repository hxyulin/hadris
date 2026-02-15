//! UDF File operations

use crate::descriptor::{DescriptorTag, LongAllocationDescriptor};
use crate::time::UdfTimestamp;

/// File Entry (ECMA-167 4/14.9)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FileEntry {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// ICB Tag
    pub icb_tag: IcbTag,
    /// UID
    pub uid: u32,
    /// GID
    pub gid: u32,
    /// Permissions
    pub permissions: u32,
    /// File Link Count
    pub file_link_count: u16,
    /// Record Format
    pub record_format: u8,
    /// Record Display Attributes
    pub record_display_attributes: u8,
    /// Record Length
    pub record_length: u32,
    /// Information Length (file size)
    pub information_length: u64,
    /// Logical Blocks Recorded
    pub logical_blocks_recorded: u64,
    /// Access Date and Time
    pub access_time: UdfTimestamp,
    /// Modification Date and Time
    pub modification_time: UdfTimestamp,
    /// Attribute Date and Time
    pub attribute_time: UdfTimestamp,
    /// Checkpoint
    pub checkpoint: u32,
    /// Extended Attribute ICB
    pub extended_attribute_icb: LongAllocationDescriptor,
    /// Implementation Identifier
    pub implementation_identifier: [u8; 32],
    /// Unique ID
    pub unique_id: u64,
    /// Length of Extended Attributes
    pub extended_attributes_length: u32,
    /// Length of Allocation Descriptors
    pub allocation_descriptors_length: u32,
    // Followed by Extended Attributes and Allocation Descriptors
}

unsafe impl bytemuck::Zeroable for FileEntry {}
unsafe impl bytemuck::Pod for FileEntry {}

impl FileEntry {
    /// Base size without variable-length fields
    pub const BASE_SIZE: usize = 176;

    /// Get the file size
    pub fn size(&self) -> u64 {
        self.information_length
    }

    /// Get the file type
    pub fn file_type(&self) -> FileType {
        self.icb_tag.file_type()
    }

    /// Check if this is a directory
    pub fn is_directory(&self) -> bool {
        self.file_type() == FileType::Directory
    }

    /// Check if this is a regular file
    pub fn is_file(&self) -> bool {
        self.file_type() == FileType::RegularFile
    }

    /// Get the allocation descriptor type
    pub fn allocation_type(&self) -> AllocationType {
        AllocationType::from_bits((self.icb_tag.flags & 0x07) as u8)
    }
}

/// Extended File Entry (ECMA-167 4/14.17)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ExtendedFileEntry {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// ICB Tag
    pub icb_tag: IcbTag,
    /// UID
    pub uid: u32,
    /// GID
    pub gid: u32,
    /// Permissions
    pub permissions: u32,
    /// File Link Count
    pub file_link_count: u16,
    /// Record Format
    pub record_format: u8,
    /// Record Display Attributes
    pub record_display_attributes: u8,
    /// Record Length
    pub record_length: u32,
    /// Information Length (file size)
    pub information_length: u64,
    /// Object Size
    pub object_size: u64,
    /// Logical Blocks Recorded
    pub logical_blocks_recorded: u64,
    /// Access Date and Time
    pub access_time: UdfTimestamp,
    /// Modification Date and Time
    pub modification_time: UdfTimestamp,
    /// Creation Date and Time
    pub creation_time: UdfTimestamp,
    /// Attribute Date and Time
    pub attribute_time: UdfTimestamp,
    /// Checkpoint
    pub checkpoint: u32,
    /// Reserved
    reserved: u32,
    /// Extended Attribute ICB
    pub extended_attribute_icb: LongAllocationDescriptor,
    /// Stream Directory ICB
    pub stream_directory_icb: LongAllocationDescriptor,
    /// Implementation Identifier
    pub implementation_identifier: [u8; 32],
    /// Unique ID
    pub unique_id: u64,
    /// Length of Extended Attributes
    pub extended_attributes_length: u32,
    /// Length of Allocation Descriptors
    pub allocation_descriptors_length: u32,
    // Followed by Extended Attributes and Allocation Descriptors
}

unsafe impl bytemuck::Zeroable for ExtendedFileEntry {}
unsafe impl bytemuck::Pod for ExtendedFileEntry {}

impl ExtendedFileEntry {
    /// Base size without variable-length fields
    pub const BASE_SIZE: usize = 216;

    /// Get the file size
    pub fn size(&self) -> u64 {
        self.information_length
    }

    /// Get the file type
    pub fn file_type(&self) -> FileType {
        self.icb_tag.file_type()
    }

    /// Check if this is a directory
    pub fn is_directory(&self) -> bool {
        self.file_type() == FileType::Directory
    }

    /// Check if this is a regular file
    pub fn is_file(&self) -> bool {
        self.file_type() == FileType::RegularFile
    }

    /// Get the allocation descriptor type
    pub fn allocation_type(&self) -> AllocationType {
        AllocationType::from_bits((self.icb_tag.flags & 0x07) as u8)
    }
}

/// ICB Tag (ECMA-167 4/14.6)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Zeroable, bytemuck::Pod)]
pub struct IcbTag {
    /// Prior Recorded Number of Direct Entries
    pub prior_recorded_num_direct_entries: u32,
    /// Strategy Type
    pub strategy_type: u16,
    /// Strategy Parameters
    pub strategy_parameters: [u8; 2],
    /// Maximum Number of Entries
    pub max_num_entries: u16,
    /// Reserved
    reserved: u8,
    /// File Type
    pub file_type: u8,
    /// Parent ICB Location (6 bytes)
    pub parent_icb_location: [u8; 6],
    /// Flags
    pub flags: u16,
}

impl IcbTag {
    /// Get the file type
    pub fn file_type(&self) -> FileType {
        FileType::from_u8(self.file_type)
    }
}

/// File type (ECMA-167 4/14.6.6)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FileType {
    /// Unspecified
    Unspecified = 0,
    /// Unallocated Space Entry
    UnallocatedSpaceEntry = 1,
    /// Partition Integrity Entry
    PartitionIntegrityEntry = 2,
    /// Indirect Entry
    IndirectEntry = 3,
    /// Directory
    Directory = 4,
    /// Regular File
    RegularFile = 5,
    /// Block Special Device
    BlockDevice = 6,
    /// Character Special Device
    CharacterDevice = 7,
    /// Extended Attribute Record
    ExtendedAttribute = 8,
    /// FIFO
    Fifo = 9,
    /// Socket
    Socket = 10,
    /// Terminal Entry
    TerminalEntry = 11,
    /// Symbolic Link
    SymbolicLink = 12,
    /// Stream Directory
    StreamDirectory = 13,
    /// Unknown type
    Unknown = 255,
}

impl FileType {
    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Unspecified,
            1 => Self::UnallocatedSpaceEntry,
            2 => Self::PartitionIntegrityEntry,
            3 => Self::IndirectEntry,
            4 => Self::Directory,
            5 => Self::RegularFile,
            6 => Self::BlockDevice,
            7 => Self::CharacterDevice,
            8 => Self::ExtendedAttribute,
            9 => Self::Fifo,
            10 => Self::Socket,
            11 => Self::TerminalEntry,
            12 => Self::SymbolicLink,
            13 => Self::StreamDirectory,
            _ => Self::Unknown,
        }
    }
}

/// Allocation descriptor type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationType {
    /// Short allocation descriptors (8 bytes each)
    Short,
    /// Long allocation descriptors (16 bytes each)
    Long,
    /// Extended allocation descriptors (20 bytes each)
    Extended,
    /// Data is embedded in the allocation descriptors field
    Embedded,
}

impl AllocationType {
    pub(crate) fn from_bits(bits: u8) -> Self {
        match bits & 0x07 {
            0 => Self::Short,
            1 => Self::Long,
            2 => Self::Extended,
            3 => Self::Embedded,
            _ => Self::Short, // Default
        }
    }
}

/// A UDF file handle
pub struct UdfFile {
    /// File size
    size: u64,
    /// ICB location
    icb: LongAllocationDescriptor,
    /// Allocation type
    allocation_type: AllocationType,
}

impl UdfFile {
    /// Create a new file handle
    pub(crate) fn new(
        size: u64,
        icb: LongAllocationDescriptor,
        allocation_type: AllocationType,
    ) -> Self {
        Self {
            size,
            icb,
            allocation_type,
        }
    }

    /// Get the file size
    pub fn size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<FileEntry>(), 176);
    static_assertions::const_assert_eq!(size_of::<ExtendedFileEntry>(), 216);
    static_assertions::const_assert_eq!(size_of::<IcbTag>(), 20);

    #[test]
    fn test_file_type_roundtrip() {
        for ft in [
            FileType::Directory,
            FileType::RegularFile,
            FileType::SymbolicLink,
        ] {
            let value = ft as u8;
            assert_eq!(FileType::from_u8(value), ft);
        }
    }
}
