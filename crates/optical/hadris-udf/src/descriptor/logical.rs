//! Logical Volume Descriptor (ECMA-167 3/10.6)

use super::{
    CharSpec, DescriptorTag, EntityIdentifier, ExtentDescriptor, LongAllocationDescriptor,
    TagIdentifier,
};
use crate::error::Result;

/// Logical Volume Descriptor (ECMA-167 3/10.6)
///
/// @hadris-spec ECMA-167:3/10.6
/// @hadris-compliance full
/// @hadris-tests comprehensive_udf::test_allocation_descriptor_sizes
/// @hadris-fuzz udf_read
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
    pub fn validate(&self, location: u32) -> Result<()> {
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

    /// Yield Type 1 partition maps from the embedded map table (ECMA-167 3/10.7.2).
    ///
    /// Skips unknown / Type 2 maps and stops if the table is truncated or malformed.
    pub fn type1_partition_maps(&self) -> impl Iterator<Item = Type1PartitionMap> + '_ {
        let maps = &self.partition_maps;
        let mut offset = 0usize;
        let mut remaining = self.num_partition_maps as usize;
        core::iter::from_fn(move || {
            while remaining > 0 {
                remaining -= 1;
                if offset + 2 > maps.len() {
                    return None;
                }
                let map_type = maps[offset];
                let map_len = maps[offset + 1] as usize;
                if map_len < 2 || offset + map_len > maps.len() {
                    return None;
                }
                let entry = &maps[offset..offset + map_len];
                offset += map_len;
                if map_type == 1 && map_len >= core::mem::size_of::<Type1PartitionMap>() {
                    return Some(*bytemuck::from_bytes(
                        &entry[..core::mem::size_of::<Type1PartitionMap>()],
                    ));
                }
            }
            None
        })
    }
}

/// Type 1 Partition Map (ECMA-167 3/10.7.2)
///
/// @hadris-spec ECMA-167:3/10.7.2
/// @hadris-compliance full
/// @hadris-tests descriptor::logical::tests::type1_partition_maps_parses_embedded_table
/// @hadris-fuzz udf_read
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

    #[test]
    fn type1_partition_maps_parses_embedded_table() {
        let mut lvd = LogicalVolumeDescriptor {
            tag: bytemuck::Zeroable::zeroed(),
            vds_number: 0,
            descriptor_char_set: CharSpec::default(),
            logical_volume_identifier: [0; 128],
            logical_block_size: 2048,
            domain_identifier: EntityIdentifier::EMPTY,
            logical_volume_contents_use: [0; 16],
            map_table_length: 6,
            num_partition_maps: 1,
            implementation_identifier: EntityIdentifier::EMPTY,
            implementation_use: [0; 128],
            integrity_sequence_extent: ExtentDescriptor::default(),
            partition_maps: [0; 72],
        };
        let map = Type1PartitionMap {
            partition_map_type: 1,
            partition_map_length: 6,
            volume_sequence_number: 1,
            partition_number: 0,
        };
        lvd.partition_maps[..6].copy_from_slice(bytemuck::bytes_of(&map));

        let mut parsed = lvd.type1_partition_maps();
        let first = parsed.next().expect("type 1 map");
        assert!(parsed.next().is_none());
        assert_eq!(first.partition_map_type, 1);
        assert_eq!(first.partition_map_length, 6);
        assert_eq!(first.volume_sequence_number, 1);
        assert_eq!(first.partition_number, 0);
    }
}
