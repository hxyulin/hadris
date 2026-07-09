//! Partition Descriptor (ECMA-167 3/10.5)

use super::{DescriptorTag, EntityIdentifier, TagIdentifier};
use crate::error::UdfResult;

/// Partition Descriptor (ECMA-167 3/10.5)
///
/// @hadris-spec ECMA-167:3/10.5
/// @hadris-compliance full
/// @hadris-tests descriptor::partition::tests::partition_descriptor_layout_and_validate
/// @hadris-fuzz udf_read
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PartitionDescriptor {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// Volume Descriptor Sequence Number
    pub vds_number: u32,
    /// Partition Flags
    pub partition_flags: u16,
    /// Partition Number
    pub partition_number: u16,
    /// Partition Contents
    pub partition_contents: EntityIdentifier,
    /// Partition Contents Use (128 bytes)
    pub partition_contents_use: [u8; 128],
    /// Access Type
    pub access_type: u32,
    /// Partition Starting Location
    pub partition_starting_location: u32,
    /// Partition Length (in sectors)
    pub partition_length: u32,
    /// Implementation Identifier
    pub implementation_identifier: EntityIdentifier,
    /// Implementation Use (128 bytes)
    pub implementation_use: [u8; 128],
    /// Reserved
    reserved: [u8; 156],
}

unsafe impl bytemuck::Zeroable for PartitionDescriptor {}
unsafe impl bytemuck::Pod for PartitionDescriptor {}

impl PartitionDescriptor {
    /// Validate this descriptor
    pub fn validate(&self, location: u32) -> UdfResult<()> {
        self.tag
            .validate(TagIdentifier::PartitionDescriptor, location)
    }

    /// Get the partition contents type
    pub fn contents_type(&self) -> PartitionContents {
        if self.partition_contents.is(b"+NSR02") {
            PartitionContents::Nsr02
        } else if self.partition_contents.is(b"+NSR03") {
            PartitionContents::Nsr03
        } else if self.partition_contents.is(b"+FDC01") {
            PartitionContents::Fdc01
        } else if self.partition_contents.is(b"+CD001") {
            PartitionContents::Cd001
        } else {
            PartitionContents::Unknown
        }
    }

    /// Check if this partition is allocated
    pub fn is_allocated(&self) -> bool {
        self.partition_flags & 0x0001 != 0
    }
}

/// Partition contents identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionContents {
    /// UDF 1.02-1.50 (NSR02)
    Nsr02,
    /// UDF 2.00+ (NSR03)
    Nsr03,
    /// Fixed/Floppy disk (FDC01)
    Fdc01,
    /// ISO 9660 (CD001)
    Cd001,
    /// Unknown partition contents
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<PartitionDescriptor>(), 512);

    fn tag_with_checksum(identifier: TagIdentifier, location: u32) -> DescriptorTag {
        let mut tag = DescriptorTag {
            tag_identifier: identifier.to_u16(),
            descriptor_version: 2,
            tag_checksum: 0,
            reserved: 0,
            tag_serial_number: 0,
            descriptor_crc: 0,
            descriptor_crc_length: 0,
            tag_location: location,
        };
        let bytes = bytemuck::bytes_of(&tag);
        let mut sum: u8 = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if i != 4 {
                sum = sum.wrapping_add(byte);
            }
        }
        tag.tag_checksum = sum;
        tag
    }

    fn entity_id(id: &[u8]) -> EntityIdentifier {
        let mut entity = EntityIdentifier::EMPTY;
        entity.identifier[..id.len()].copy_from_slice(id);
        entity
    }

    /// Vertical slice for ECMA-167:3/10.5 — layout, validate, contents helpers.
    #[test]
    fn partition_descriptor_layout_and_validate() {
        let location = 20u32;
        let mut desc = PartitionDescriptor {
            tag: tag_with_checksum(TagIdentifier::PartitionDescriptor, location),
            vds_number: 1,
            partition_flags: 0x0001,
            partition_number: 0,
            partition_contents: entity_id(b"+NSR03"),
            partition_contents_use: [0; 128],
            access_type: 1,
            partition_starting_location: 0,
            partition_length: 1000,
            implementation_identifier: EntityIdentifier::EMPTY,
            implementation_use: [0; 128],
            reserved: [0; 156],
        };

        assert_eq!(core::mem::size_of_val(&desc), 512);
        assert!(desc.validate(location).is_ok());
        assert!(desc.is_allocated());
        assert_eq!(desc.contents_type(), PartitionContents::Nsr03);

        // Wrong tag id fails closed
        desc.tag.tag_identifier = TagIdentifier::PrimaryVolumeDescriptor.to_u16();
        let bytes = bytemuck::bytes_of(&desc.tag);
        let mut sum: u8 = 0;
        for (i, &byte) in bytes.iter().enumerate() {
            if i != 4 {
                sum = sum.wrapping_add(byte);
            }
        }
        desc.tag.tag_checksum = sum;
        assert!(desc.validate(location).is_err());
    }
}
