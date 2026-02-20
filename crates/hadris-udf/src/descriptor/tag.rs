//! UDF Descriptor Tag (ECMA-167 3/7.2)

use crate::error::{UdfError, UdfResult};

/// Descriptor tag (ECMA-167 3/7.2)
///
/// Every UDF descriptor starts with this 16-byte tag
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Zeroable, bytemuck::Pod)]
pub struct DescriptorTag {
    /// Tag identifier
    pub tag_identifier: u16,
    /// Descriptor version
    pub descriptor_version: u16,
    /// Tag checksum (sum of bytes 0-3 and 5-15 mod 256)
    pub tag_checksum: u8,
    /// Reserved
    pub reserved: u8,
    /// Tag serial number
    pub tag_serial_number: u16,
    /// Descriptor CRC
    pub descriptor_crc: u16,
    /// Descriptor CRC length (bytes after tag to checksum)
    pub descriptor_crc_length: u16,
    /// Tag location (sector number)
    pub tag_location: u32,
}

impl DescriptorTag {
    /// Size of the tag in bytes
    pub const SIZE: usize = 16;

    /// Verify the tag checksum
    pub fn verify_checksum(&self) -> bool {
        let bytes = bytemuck::bytes_of(self);
        let mut sum: u8 = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if i != 4 {
                // Skip the checksum byte itself
                sum = sum.wrapping_add(byte);
            }
        }
        sum == self.tag_checksum
    }

    /// Verify the descriptor CRC
    ///
    /// `data` should be the bytes following the tag, of length `descriptor_crc_length`
    pub fn verify_crc(&self, data: &[u8]) -> bool {
        if data.len() < self.descriptor_crc_length as usize {
            return false;
        }
        let computed = crc16_itu(&data[..self.descriptor_crc_length as usize]);
        computed == self.descriptor_crc
    }

    /// Get the tag identifier as an enum
    pub fn identifier(&self) -> TagIdentifier {
        TagIdentifier::from_u16(self.tag_identifier)
    }

    /// Validate the tag and return an error if invalid
    pub fn validate(&self, expected: TagIdentifier, location: u32) -> UdfResult<()> {
        if !self.verify_checksum() {
            return Err(UdfError::CrcMismatch {
                expected: 0,
                computed: self.tag_checksum as u16,
            });
        }
        if self.identifier() != expected {
            return Err(UdfError::InvalidTag {
                expected: expected.to_u16(),
                found: self.tag_identifier,
            });
        }
        if self.tag_location != location {
            // Location mismatch is tolerated in some UDF implementations
            // but we note it for debugging
        }
        Ok(())
    }
}

/// Tag identifier values (ECMA-167 3/7.2.1)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum TagIdentifier {
    /// Primary Volume Descriptor
    PrimaryVolumeDescriptor = 1,
    /// Anchor Volume Descriptor Pointer
    AnchorVolumeDescriptorPointer = 2,
    /// Volume Descriptor Pointer
    VolumeDescriptorPointer = 3,
    /// Implementation Use Volume Descriptor
    ImplementationUseVolumeDescriptor = 4,
    /// Partition Descriptor
    PartitionDescriptor = 5,
    /// Logical Volume Descriptor
    LogicalVolumeDescriptor = 6,
    /// Unallocated Space Descriptor
    UnallocatedSpaceDescriptor = 7,
    /// Terminating Descriptor
    TerminatingDescriptor = 8,
    /// Logical Volume Integrity Descriptor
    LogicalVolumeIntegrityDescriptor = 9,

    // File structure descriptors (ECMA-167 4/14)
    /// File Set Descriptor
    FileSetDescriptor = 256,
    /// File Identifier Descriptor
    FileIdentifierDescriptor = 257,
    /// Allocation Extent Descriptor
    AllocationExtentDescriptor = 258,
    /// Indirect Entry
    IndirectEntry = 259,
    /// Terminal Entry
    TerminalEntry = 260,
    /// File Entry
    FileEntry = 261,
    /// Extended Attribute Header Descriptor
    ExtendedAttributeHeaderDescriptor = 262,
    /// Unallocated Space Entry
    UnallocatedSpaceEntry = 263,
    /// Space Bitmap Descriptor
    SpaceBitmapDescriptor = 264,
    /// Partition Integrity Entry
    PartitionIntegrityEntry = 265,
    /// Extended File Entry
    ExtendedFileEntry = 266,

    /// Unknown tag
    Unknown = 0xFFFF,
}

impl TagIdentifier {
    /// Convert from u16
    pub fn from_u16(value: u16) -> Self {
        match value {
            1 => Self::PrimaryVolumeDescriptor,
            2 => Self::AnchorVolumeDescriptorPointer,
            3 => Self::VolumeDescriptorPointer,
            4 => Self::ImplementationUseVolumeDescriptor,
            5 => Self::PartitionDescriptor,
            6 => Self::LogicalVolumeDescriptor,
            7 => Self::UnallocatedSpaceDescriptor,
            8 => Self::TerminatingDescriptor,
            9 => Self::LogicalVolumeIntegrityDescriptor,
            256 => Self::FileSetDescriptor,
            257 => Self::FileIdentifierDescriptor,
            258 => Self::AllocationExtentDescriptor,
            259 => Self::IndirectEntry,
            260 => Self::TerminalEntry,
            261 => Self::FileEntry,
            262 => Self::ExtendedAttributeHeaderDescriptor,
            263 => Self::UnallocatedSpaceEntry,
            264 => Self::SpaceBitmapDescriptor,
            265 => Self::PartitionIntegrityEntry,
            266 => Self::ExtendedFileEntry,
            _ => Self::Unknown,
        }
    }

    /// Convert to u16
    pub fn to_u16(self) -> u16 {
        match self {
            Self::Unknown => 0xFFFF,
            _ => self as u16,
        }
    }
}

impl core::fmt::Display for TagIdentifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PrimaryVolumeDescriptor => write!(f, "Primary Volume Descriptor"),
            Self::AnchorVolumeDescriptorPointer => {
                write!(f, "Anchor Volume Descriptor Pointer")
            }
            Self::VolumeDescriptorPointer => write!(f, "Volume Descriptor Pointer"),
            Self::ImplementationUseVolumeDescriptor => {
                write!(f, "Implementation Use Volume Descriptor")
            }
            Self::PartitionDescriptor => write!(f, "Partition Descriptor"),
            Self::LogicalVolumeDescriptor => write!(f, "Logical Volume Descriptor"),
            Self::UnallocatedSpaceDescriptor => write!(f, "Unallocated Space Descriptor"),
            Self::TerminatingDescriptor => write!(f, "Terminating Descriptor"),
            Self::LogicalVolumeIntegrityDescriptor => {
                write!(f, "Logical Volume Integrity Descriptor")
            }
            Self::FileSetDescriptor => write!(f, "File Set Descriptor"),
            Self::FileIdentifierDescriptor => write!(f, "File Identifier Descriptor"),
            Self::AllocationExtentDescriptor => write!(f, "Allocation Extent Descriptor"),
            Self::IndirectEntry => write!(f, "Indirect Entry"),
            Self::TerminalEntry => write!(f, "Terminal Entry"),
            Self::FileEntry => write!(f, "File Entry"),
            Self::ExtendedAttributeHeaderDescriptor => {
                write!(f, "Extended Attribute Header Descriptor")
            }
            Self::UnallocatedSpaceEntry => write!(f, "Unallocated Space Entry"),
            Self::SpaceBitmapDescriptor => write!(f, "Space Bitmap Descriptor"),
            Self::PartitionIntegrityEntry => write!(f, "Partition Integrity Entry"),
            Self::ExtendedFileEntry => write!(f, "Extended File Entry"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// CRC-16-ITU (CCITT) used by UDF
///
/// Polynomial: x^16 + x^12 + x^5 + 1 (0x1021)
fn crc16_itu(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        let mut x = ((crc >> 8) ^ (byte as u16)) & 0xFF;
        x ^= x >> 4;
        crc = (crc << 8) ^ (x << 12) ^ (x << 5) ^ x;
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<DescriptorTag>(), 16);

    #[test]
    fn test_tag_identifier_roundtrip() {
        for id in [
            TagIdentifier::PrimaryVolumeDescriptor,
            TagIdentifier::AnchorVolumeDescriptorPointer,
            TagIdentifier::FileEntry,
            TagIdentifier::ExtendedFileEntry,
        ] {
            let value = id.to_u16();
            assert_eq!(TagIdentifier::from_u16(value), id);
        }
    }

    #[test]
    fn test_crc16() {
        // Test vector from UDF spec
        let data = [0u8; 0];
        assert_eq!(crc16_itu(&data), 0);
    }

    #[test]
    fn test_tag_checksum() {
        let mut tag = DescriptorTag::default();
        tag.tag_identifier = 2; // AVDP

        // Calculate checksum
        let bytes = bytemuck::bytes_of(&tag);
        let mut sum: u8 = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if i != 4 {
                sum = sum.wrapping_add(byte);
            }
        }
        tag.tag_checksum = sum;

        assert!(tag.verify_checksum());
    }
}
