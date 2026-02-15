//! Logical Volume Descriptor (ECMA-167 3/10.6)

use super::{
    CharSpec, DescriptorTag, EntityIdentifier, ExtentDescriptor, LongAllocationDescriptor,
    TagIdentifier,
};
use crate::error::UdfResult;

/// Logical Volume Descriptor (ECMA-167 3/10.6)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LogicalVolumeDescriptor {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// Volume Descriptor Sequence Number
    pub vds_number: u32,
    /// Descriptor Character Set
    pub descriptor_char_set: CharSpec,
    /// Logical Volume Identifier (dstring, 128 bytes)
    pub logical_volume_identifier: [u8; 128],
    /// Logical Block Size
    pub logical_block_size: u32,
    /// Domain Identifier
    pub domain_identifier: EntityIdentifier,
    /// Logical Volume Contents Use (16 bytes)
    /// Contains LongAllocationDescriptor pointing to File Set Descriptor
    pub logical_volume_contents_use: [u8; 16],
    /// Map Table Length
    pub map_table_length: u32,
    /// Number of Partition Maps
    pub num_partition_maps: u32,
    /// Implementation Identifier
    pub implementation_identifier: EntityIdentifier,
    /// Implementation Use (128 bytes)
    pub implementation_use: [u8; 128],
    /// Integrity Sequence Extent
    pub integrity_sequence_extent: ExtentDescriptor,
    /// Partition Maps (variable length, up to remaining space)
    pub partition_maps: [u8; 72],
}

unsafe impl bytemuck::Zeroable for LogicalVolumeDescriptor {}
unsafe impl bytemuck::Pod for LogicalVolumeDescriptor {}

impl LogicalVolumeDescriptor {
    /// Validate this descriptor
    pub fn validate(&self, location: u32) -> UdfResult<()> {
        self.tag
            .validate(TagIdentifier::LogicalVolumeDescriptor, location)
    }

    /// Get the File Set Descriptor location
    pub fn file_set_location(&self) -> LongAllocationDescriptor {
        *bytemuck::from_bytes(&self.logical_volume_contents_use)
    }

    /// Get the logical volume identifier as a string
    #[cfg(feature = "alloc")]
    pub fn volume_id(&self) -> alloc::string::String {
        super::primary::decode_dstring(&self.logical_volume_identifier)
    }

    /// Check if this is a UDF domain
    pub fn is_udf_domain(&self) -> bool {
        self.domain_identifier.is(b"*OSTA UDF Compliant")
    }
}

/// Type 1 Partition Map (ECMA-167 3/10.7.2)
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct Type1PartitionMap {
    /// Partition Map Type (1)
    pub partition_map_type: u8,
    /// Partition Map Length (6)
    pub partition_map_length: u8,
    /// Volume Sequence Number
    pub volume_sequence_number: u16,
    /// Partition Number
    pub partition_number: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<LogicalVolumeDescriptor>(), 512);
    static_assertions::const_assert_eq!(size_of::<Type1PartitionMap>(), 6);
}
