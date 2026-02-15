//! Partition Descriptor (ECMA-167 3/10.5)

use super::{DescriptorTag, EntityIdentifier, TagIdentifier};
use crate::error::UdfResult;

/// Partition Descriptor (ECMA-167 3/10.5)
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
}
