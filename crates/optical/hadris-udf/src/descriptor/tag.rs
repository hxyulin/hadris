//! UDF Descriptor Tag (ECMA-167 3/7.2)

use crate::error::{Error, Result};

/// Descriptor tag (ECMA-167 3/7.2)
///
/// Every UDF descriptor starts with this 16-byte tag
///
/// @hadris-spec ECMA-167:3/7.2
/// @hadris-compliance partial
/// @hadris-note Core tag invariants are checked, but validation across every descriptor context is not yet established.
/// @hadris-tests descriptor::tag::tests::validate_bytes_enforces_version_reserved_location_and_crc
/// @hadris-fuzz udf_read
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
        self.computed_checksum() == self.tag_checksum
    }

    fn computed_checksum(&self) -> u8 {
        let bytes = self.to_disk_bytes();
        let mut sum: u8 = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if i != 4 {
                // Skip the checksum byte itself
                sum = sum.wrapping_add(byte);
            }
        }
        sum
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
    pub fn validate(&self, expected: TagIdentifier, location: u32) -> Result<()> {
        let computed_checksum = self.computed_checksum();
        if computed_checksum != self.tag_checksum {
            return Err(Error::CrcMismatch {
                expected: self.tag_checksum as u16,
                computed: computed_checksum as u16,
            });
        }
        if self.identifier() != expected {
            return Err(Error::InvalidTag {
                expected: expected.to_u16(),
                found: self.tag_identifier,
            });
        }
        if !matches!(self.descriptor_version, 2 | 3) {
            return Err(Error::InvalidVds("descriptor tag version must be 2 or 3"));
        }
        if self.reserved != 0 {
            return Err(Error::InvalidVds(
                "descriptor tag reserved byte must be zero",
            ));
        }
        if self.tag_location != location {
            return Err(Error::InvalidVds(
                "descriptor tag location does not match its recorded block",
            ));
        }
        Ok(())
    }

    pub(crate) fn from_disk_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < Self::SIZE {
            return Err(Error::InvalidVds("descriptor tag is truncated"));
        }
        Ok(Self {
            tag_identifier: u16::from_le_bytes([data[0], data[1]]),
            descriptor_version: u16::from_le_bytes([data[2], data[3]]),
            tag_checksum: data[4],
            reserved: data[5],
            tag_serial_number: u16::from_le_bytes([data[6], data[7]]),
            descriptor_crc: u16::from_le_bytes([data[8], data[9]]),
            descriptor_crc_length: u16::from_le_bytes([data[10], data[11]]),
            tag_location: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        })
    }

    pub(crate) fn validate_bytes(
        descriptor: &[u8],
        expected: TagIdentifier,
        location: u32,
    ) -> Result<Self> {
        let tag = Self::from_disk_bytes(descriptor)?;
        tag.validate(expected, location)?;
        let payload = descriptor
            .get(Self::SIZE..)
            .ok_or(Error::InvalidVds("descriptor payload is truncated"))?;
        if tag.descriptor_crc_length as usize > payload.len() {
            return Err(Error::InvalidVds(
                "descriptor CRC length exceeds descriptor payload",
            ));
        }
        if tag.descriptor_crc_length > 0 && !tag.verify_crc(payload) {
            return Err(Error::CrcMismatch {
                expected: tag.descriptor_crc,
                computed: crc16_itu(&payload[..tag.descriptor_crc_length as usize]),
            });
        }
        Ok(tag)
    }

    fn to_disk_bytes(self) -> [u8; Self::SIZE] {
        let mut bytes = [0; Self::SIZE];
        bytes[0..2].copy_from_slice(&self.tag_identifier.to_le_bytes());
        bytes[2..4].copy_from_slice(&self.descriptor_version.to_le_bytes());
        bytes[4] = self.tag_checksum;
        bytes[5] = self.reserved;
        bytes[6..8].copy_from_slice(&self.tag_serial_number.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.descriptor_crc.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.descriptor_crc_length.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.tag_location.to_le_bytes());
        bytes
    }
}

/// Tag identifier values (ECMA-167 3/7.2.1)
///
/// @hadris-spec ECMA-167:3/7.2.1
/// @hadris-compliance partial
/// @hadris-note Known identifiers are modeled and tested, but context-specific identifier constraints are not all validated.
/// @hadris-tests comprehensive_udf::test_descriptor_tag_ids
/// @hadris-fuzz udf_read
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
        let mut tag = DescriptorTag {
            tag_identifier: 2, // AVDP
            ..DescriptorTag::default()
        };

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

    fn descriptor_bytes(version: u16, reserved: u8, location: u32) -> [u8; 20] {
        let mut bytes = [0u8; 20];
        bytes[0..2].copy_from_slice(
            &TagIdentifier::PrimaryVolumeDescriptor
                .to_u16()
                .to_le_bytes(),
        );
        bytes[2..4].copy_from_slice(&version.to_le_bytes());
        bytes[5] = reserved;
        let crc = crc16_itu(&bytes[16..]);
        bytes[8..10].copy_from_slice(&crc.to_le_bytes());
        bytes[10..12].copy_from_slice(&4u16.to_le_bytes());
        bytes[12..16].copy_from_slice(&location.to_le_bytes());
        bytes[4] = bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| *index < 16 && *index != 4)
            .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
        bytes
    }

    #[test]
    fn validate_bytes_enforces_version_reserved_location_and_crc() {
        let valid = descriptor_bytes(2, 0, 17);
        assert!(
            DescriptorTag::validate_bytes(&valid, TagIdentifier::PrimaryVolumeDescriptor, 17)
                .is_ok()
        );

        assert!(
            DescriptorTag::validate_bytes(
                &descriptor_bytes(1, 0, 17),
                TagIdentifier::PrimaryVolumeDescriptor,
                17
            )
            .is_err()
        );
        assert!(
            DescriptorTag::validate_bytes(
                &descriptor_bytes(2, 1, 17),
                TagIdentifier::PrimaryVolumeDescriptor,
                17
            )
            .is_err()
        );
        assert!(
            DescriptorTag::validate_bytes(&valid, TagIdentifier::PrimaryVolumeDescriptor, 18)
                .is_err()
        );

        let mut bad_crc = valid;
        bad_crc[16] ^= 1;
        assert!(
            DescriptorTag::validate_bytes(&bad_crc, TagIdentifier::PrimaryVolumeDescriptor, 17)
                .is_err()
        );
    }

    #[test]
    fn validate_reports_stored_and_computed_tag_checksums() {
        let mut bytes = descriptor_bytes(2, 0, 17);
        bytes[4] = bytes[4].wrapping_add(1);
        let tag = DescriptorTag::from_disk_bytes(&bytes).unwrap();

        match tag
            .validate(TagIdentifier::PrimaryVolumeDescriptor, 17)
            .unwrap_err()
        {
            Error::CrcMismatch { expected, computed } => {
                assert_eq!(expected, tag.tag_checksum as u16);
                assert_eq!(computed, tag.computed_checksum() as u16);
            }
            error => panic!("unexpected error: {error}"),
        }
    }
}
