//! File Set Descriptor (ECMA-167 4/14.1)

use super::{CharSpec, DescriptorTag, EntityIdentifier, LongAllocationDescriptor, TagIdentifier};
use crate::error::UdfResult;
use crate::time::UdfTimestamp;

/// File Set Descriptor (ECMA-167 4/14.1)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FileSetDescriptor {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// Recording Date and Time
    pub recording_date_time: UdfTimestamp,
    /// Interchange Level
    pub interchange_level: u16,
    /// Maximum Interchange Level
    pub max_interchange_level: u16,
    /// Character Set List
    pub character_set_list: u32,
    /// Maximum Character Set List
    pub max_character_set_list: u32,
    /// File Set Number
    pub file_set_number: u32,
    /// File Set Descriptor Number
    pub file_set_desc_number: u32,
    /// Logical Volume Identifier Character Set
    pub logical_volume_id_char_set: CharSpec,
    /// Logical Volume Identifier (dstring, 128 bytes)
    pub logical_volume_identifier: [u8; 128],
    /// File Set Character Set
    pub file_set_char_set: CharSpec,
    /// File Set Identifier (dstring, 32 bytes)
    pub file_set_identifier: [u8; 32],
    /// Copyright File Identifier (dstring, 32 bytes)
    pub copyright_file_identifier: [u8; 32],
    /// Abstract File Identifier (dstring, 32 bytes)
    pub abstract_file_identifier: [u8; 32],
    /// Root Directory ICB
    pub root_directory_icb: LongAllocationDescriptor,
    /// Domain Identifier
    pub domain_identifier: EntityIdentifier,
    /// Next Extent
    pub next_extent: LongAllocationDescriptor,
    /// System Stream Directory ICB
    pub system_stream_directory_icb: LongAllocationDescriptor,
    /// Reserved
    reserved: [u8; 32],
}

unsafe impl bytemuck::Zeroable for FileSetDescriptor {}
unsafe impl bytemuck::Pod for FileSetDescriptor {}

impl FileSetDescriptor {
    /// Validate this descriptor
    pub fn validate(&self, location: u32) -> UdfResult<()> {
        self.tag
            .validate(TagIdentifier::FileSetDescriptor, location)
    }

    /// Get the root directory ICB location
    pub fn root_icb(&self) -> &LongAllocationDescriptor {
        &self.root_directory_icb
    }

    /// Get the file set identifier as a string
    #[cfg(feature = "alloc")]
    pub fn file_set_id(&self) -> alloc::string::String {
        super::primary::decode_dstring(&self.file_set_identifier)
    }

    /// Check if this is a UDF domain
    pub fn is_udf_domain(&self) -> bool {
        self.domain_identifier.is(b"*OSTA UDF Compliant")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<FileSetDescriptor>(), 512);
}
