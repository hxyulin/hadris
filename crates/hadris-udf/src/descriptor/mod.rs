//! UDF Volume Descriptors
//!
//! This module contains structures for parsing UDF volume descriptors
//! according to ECMA-167.

mod anchor;
mod fileset;
mod logical;
mod partition;
mod primary;
mod tag;

pub use anchor::AnchorVolumeDescriptorPointer;
pub use fileset::FileSetDescriptor;
pub use logical::LogicalVolumeDescriptor;
pub use partition::{PartitionContents, PartitionDescriptor};
pub use primary::PrimaryVolumeDescriptor;
pub use tag::{DescriptorTag, TagIdentifier};

use crate::error::{UdfError, UdfResult};
use hadris_io::{Read, Seek, SeekFrom};

/// Extent descriptor (ECMA-167 3/7.1)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Zeroable, bytemuck::Pod)]
pub struct ExtentDescriptor {
    /// Length in bytes
    pub length: u32,
    /// Location (logical sector number)
    pub location: u32,
}

impl ExtentDescriptor {
    /// Check if this extent is empty
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
}

/// Long allocation descriptor (ECMA-167 4/14.14.2)
///
/// Used to reference data that may span multiple partitions
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LongAllocationDescriptor {
    /// Extent length (high 2 bits indicate type)
    pub extent_length: u32,
    /// Logical block number
    pub logical_block_num: u32,
    /// Partition reference number
    pub partition_ref_num: u16,
    /// Implementation use (6 bytes)
    pub impl_use: [u8; 6],
}

impl Default for LongAllocationDescriptor {
    fn default() -> Self {
        Self {
            extent_length: 0,
            logical_block_num: 0,
            partition_ref_num: 0,
            impl_use: [0; 6],
        }
    }
}

unsafe impl bytemuck::Zeroable for LongAllocationDescriptor {}
unsafe impl bytemuck::Pod for LongAllocationDescriptor {}

impl LongAllocationDescriptor {
    /// Get the extent length in bytes (excluding type bits)
    pub fn length(&self) -> u32 {
        self.extent_length & 0x3FFFFFFF
    }

    /// Get the extent type
    pub fn extent_type(&self) -> ExtentType {
        ExtentType::from_bits((self.extent_length >> 30) as u8)
    }

    /// Check if this is a recorded and allocated extent
    pub fn is_recorded(&self) -> bool {
        matches!(self.extent_type(), ExtentType::RecordedAllocated)
    }

    /// Get the extent location (legacy accessor)
    pub fn extent_location(&self) -> LbAddr {
        LbAddr {
            logical_block_num: self.logical_block_num,
            partition_ref_num: self.partition_ref_num,
        }
    }
}

/// Short allocation descriptor (ECMA-167 4/14.14.1)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Zeroable, bytemuck::Pod)]
pub struct ShortAllocationDescriptor {
    /// Extent length (high 2 bits indicate type)
    pub extent_length: u32,
    /// Extent position (logical block number within partition)
    pub extent_position: u32,
}

impl ShortAllocationDescriptor {
    /// Get the extent length in bytes
    pub fn length(&self) -> u32 {
        self.extent_length & 0x3FFFFFFF
    }

    /// Get the extent type
    pub fn extent_type(&self) -> ExtentType {
        ExtentType::from_bits((self.extent_length >> 30) as u8)
    }
}

/// Extent type (allocation descriptor type field)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtentType {
    /// Recorded and allocated
    RecordedAllocated,
    /// Allocated but not recorded
    AllocatedNotRecorded,
    /// Not allocated and not recorded
    NotAllocatedNotRecorded,
    /// Next extent of descriptors
    NextExtent,
}

impl ExtentType {
    fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::RecordedAllocated,
            1 => Self::AllocatedNotRecorded,
            2 => Self::NotAllocatedNotRecorded,
            3 => Self::NextExtent,
            _ => unreachable!(),
        }
    }
}

/// Logical block address (ECMA-167 4/7.1)
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LbAddr {
    /// Logical block number
    pub logical_block_num: u32,
    /// Partition reference number
    pub partition_ref_num: u16,
}

/// Entity identifier (ECMA-167 1/7.4)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EntityIdentifier {
    /// Flags
    pub flags: u8,
    /// Identifier (23 bytes, padded with zeros)
    pub identifier: [u8; 23],
    /// Identifier suffix (8 bytes)
    pub suffix: [u8; 8],
}

impl EntityIdentifier {
    /// Empty identifier
    pub const EMPTY: Self = Self {
        flags: 0,
        identifier: [0; 23],
        suffix: [0; 8],
    };

    /// Get the identifier as a string (trimmed)
    #[cfg(feature = "alloc")]
    pub fn as_str(&self) -> alloc::string::String {
        let end = self.identifier.iter().position(|&b| b == 0).unwrap_or(23);
        alloc::string::String::from_utf8_lossy(&self.identifier[..end]).into_owned()
    }

    /// Check if this is a specific identifier
    pub fn is(&self, id: &[u8]) -> bool {
        let end = self.identifier.iter().position(|&b| b == 0).unwrap_or(23);
        &self.identifier[..end] == id
    }
}

impl Default for EntityIdentifier {
    fn default() -> Self {
        Self::EMPTY
    }
}

unsafe impl bytemuck::Zeroable for EntityIdentifier {}
unsafe impl bytemuck::Pod for EntityIdentifier {}

/// Character set specification (ECMA-167 1/7.2.1)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CharSpec {
    /// Character set type (0 = CS0, OSTA Compressed Unicode)
    pub char_set_type: u8,
    /// Character set information (63 bytes)
    pub char_set_info: [u8; 63],
}

impl CharSpec {
    /// OSTA Compressed Unicode (CS0)
    pub const OSTA_COMPRESSED_UNICODE: Self = Self {
        char_set_type: 0,
        char_set_info: [0; 63],
    };
}

impl Default for CharSpec {
    fn default() -> Self {
        Self::OSTA_COMPRESSED_UNICODE
    }
}

unsafe impl bytemuck::Zeroable for CharSpec {}
unsafe impl bytemuck::Pod for CharSpec {}

/// Volume Recognition Sequence magic numbers
pub mod vrs {
    /// Beginning of Extended Area (BEA01)
    pub const BEA01: &[u8; 5] = b"BEA01";
    /// NSR02 - UDF 1.02-1.50
    pub const NSR02: &[u8; 5] = b"NSR02";
    /// NSR03 - UDF 2.00+
    pub const NSR03: &[u8; 5] = b"NSR03";
    /// Terminal Entry Area (TEA01)
    pub const TEA01: &[u8; 5] = b"TEA01";
    /// ISO 9660 CD-ROM
    pub const CD001: &[u8; 5] = b"CD001";
}

/// Parse the Volume Recognition Sequence to detect UDF
///
/// Returns the UDF NSR version found, or an error if not UDF
pub fn parse_vrs<R: Read + Seek>(reader: &mut R) -> UdfResult<VrsType> {
    // VRS starts at sector 16
    reader.seek(SeekFrom::Start(16 * 2048))?;

    let mut buffer = [0u8; 2048];
    let mut found_bea = false;
    let mut found_nsr = None;

    // Scan up to 16 sectors for VRS
    for _ in 0..16 {
        reader.read_exact(&mut buffer)?;

        // Check structure type (byte 0) and version (byte 6)
        if buffer[0] != 0 || buffer[6] != 1 {
            continue;
        }

        let id = &buffer[1..6];
        match id {
            b"BEA01" => found_bea = true,
            b"NSR02" if found_bea => found_nsr = Some(VrsType::Nsr02),
            b"NSR03" if found_bea => found_nsr = Some(VrsType::Nsr03),
            b"TEA01" if found_nsr.is_some() => return Ok(found_nsr.unwrap()),
            b"CD001" => continue, // ISO 9660 descriptor, skip
            _ => continue,
        }
    }

    match found_nsr {
        Some(nsr) => Ok(nsr),
        None => Err(UdfError::InvalidVrs),
    }
}

/// VRS type detected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrsType {
    /// NSR02 - UDF 1.02 to 1.50
    Nsr02,
    /// NSR03 - UDF 2.00 and later
    Nsr03,
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<ExtentDescriptor>(), 8);
    static_assertions::const_assert_eq!(size_of::<LongAllocationDescriptor>(), 16);
    static_assertions::const_assert_eq!(size_of::<ShortAllocationDescriptor>(), 8);
    static_assertions::const_assert_eq!(size_of::<EntityIdentifier>(), 32);
    static_assertions::const_assert_eq!(size_of::<CharSpec>(), 64);
}
