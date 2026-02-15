//! Primary Volume Descriptor (ECMA-167 3/10.1)

use super::{CharSpec, DescriptorTag, EntityIdentifier, ExtentDescriptor, TagIdentifier};
use crate::error::UdfResult;
use crate::time::UdfTimestamp;

/// Primary Volume Descriptor (ECMA-167 3/10.1)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PrimaryVolumeDescriptor {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// Volume Descriptor Sequence Number
    pub vds_number: u32,
    /// Primary Volume Descriptor Number
    pub pvd_number: u32,
    /// Volume Identifier (dstring, 32 bytes)
    pub volume_identifier: [u8; 32],
    /// Volume Sequence Number
    pub volume_sequence_number: u16,
    /// Maximum Volume Sequence Number
    pub max_volume_sequence_number: u16,
    /// Interchange Level
    pub interchange_level: u16,
    /// Maximum Interchange Level
    pub max_interchange_level: u16,
    /// Character Set List
    pub character_set_list: u32,
    /// Maximum Character Set List
    pub max_character_set_list: u32,
    /// Volume Set Identifier (dstring, 128 bytes)
    pub volume_set_identifier: [u8; 128],
    /// Descriptor Character Set
    pub descriptor_char_set: CharSpec,
    /// Explanatory Character Set
    pub explanatory_char_set: CharSpec,
    /// Volume Abstract
    pub volume_abstract: ExtentDescriptor,
    /// Volume Copyright Notice
    pub volume_copyright: ExtentDescriptor,
    /// Application Identifier
    pub application_identifier: EntityIdentifier,
    /// Recording Date and Time
    pub recording_date_time: UdfTimestamp,
    /// Implementation Identifier
    pub implementation_identifier: EntityIdentifier,
    /// Implementation Use (64 bytes)
    pub implementation_use: [u8; 64],
    /// Predecessor Volume Descriptor Sequence Location
    pub predecessor_vds_location: u32,
    /// Flags
    pub flags: u16,
    /// Reserved
    reserved: [u8; 22],
}

unsafe impl bytemuck::Zeroable for PrimaryVolumeDescriptor {}
unsafe impl bytemuck::Pod for PrimaryVolumeDescriptor {}

impl PrimaryVolumeDescriptor {
    /// Validate this descriptor
    pub fn validate(&self, location: u32) -> UdfResult<()> {
        self.tag
            .validate(TagIdentifier::PrimaryVolumeDescriptor, location)
    }

    /// Get the volume identifier as a string
    #[cfg(feature = "alloc")]
    pub fn volume_id(&self) -> alloc::string::String {
        decode_dstring(&self.volume_identifier)
    }
}

/// Decode a UDF dstring (compressed unicode)
#[cfg(feature = "alloc")]
pub fn decode_dstring(data: &[u8]) -> alloc::string::String {
    if data.is_empty() {
        return alloc::string::String::new();
    }

    // First byte is compression ID
    let compression_id = data[0];
    // Last byte is length
    let len = data[data.len() - 1] as usize;

    if len == 0 || len > data.len() - 1 {
        return alloc::string::String::new();
    }

    let content = &data[1..=len.min(data.len() - 2)];

    match compression_id {
        8 => {
            // UTF-8 (or Latin-1 in older implementations)
            alloc::string::String::from_utf8_lossy(content).into_owned()
        }
        16 => {
            // UTF-16 BE
            let mut result = alloc::string::String::new();
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
        _ => alloc::string::String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<PrimaryVolumeDescriptor>(), 512);
}
