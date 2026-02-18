//! Anchor Volume Descriptor Pointer (ECMA-167 3/10.2)

use super::{DescriptorTag, ExtentDescriptor, TagIdentifier};
use crate::error::{UdfError, UdfResult};
use super::super::super::{Read, Seek, SeekFrom};

/// Anchor Volume Descriptor Pointer (AVDP)
///
/// Located at sector 256 (and optionally at N-256 and N where N is last sector)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AnchorVolumeDescriptorPointer {
    /// Descriptor tag
    pub tag: DescriptorTag,
    /// Main Volume Descriptor Sequence extent
    pub main_vds_extent: ExtentDescriptor,
    /// Reserve Volume Descriptor Sequence extent
    pub reserve_vds_extent: ExtentDescriptor,
    /// Reserved (480 bytes)
    reserved: [u8; 480],
}

unsafe impl bytemuck::Zeroable for AnchorVolumeDescriptorPointer {}
unsafe impl bytemuck::Pod for AnchorVolumeDescriptorPointer {}

io_transform! {

impl AnchorVolumeDescriptorPointer {
    /// Standard location for the first AVDP (sector 256)
    pub const LOCATION_256: u32 = 256;

    /// Read and parse an AVDP from the given location
    pub async fn read<R: Read + Seek>(reader: &mut R, location: u32) -> UdfResult<Self> {
        reader.seek(SeekFrom::Start((location as u64) * 2048)).await?;

        let mut buffer = [0u8; 512];
        reader.read_exact(&mut buffer).await?;

        let avdp: Self = *bytemuck::from_bytes(&buffer);
        avdp.validate(location)?;
        Ok(avdp)
    }

    /// Find and read the AVDP from standard locations
    ///
    /// Tries sector 256 first, then N-256, then N (last sector)
    pub async fn find<R: Read + Seek>(reader: &mut R, total_sectors: Option<u64>) -> UdfResult<Self> {
        // Try sector 256 first (always present)
        if let Ok(avdp) = Self::read(reader, Self::LOCATION_256).await {
            return Ok(avdp);
        }

        // Try N-256 and N if we know the disk size
        if let Some(n) = total_sectors {
            if n > 256 {
                if let Ok(avdp) = Self::read(reader, (n - 256) as u32).await {
                    return Ok(avdp);
                }
            }
            if let Ok(avdp) = Self::read(reader, (n - 1) as u32).await {
                return Ok(avdp);
            }
        }

        Err(UdfError::NoAnchor)
    }

    /// Validate this AVDP
    fn validate(&self, location: u32) -> UdfResult<()> {
        self.tag
            .validate(TagIdentifier::AnchorVolumeDescriptorPointer, location)?;

        // Verify CRC if present
        if self.tag.descriptor_crc_length > 0 {
            let data = bytemuck::bytes_of(self);
            if !self.tag.verify_crc(&data[16..]) {
                return Err(UdfError::CrcMismatch {
                    expected: self.tag.descriptor_crc,
                    computed: 0,
                });
            }
        }

        Ok(())
    }
}

} // io_transform!

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<AnchorVolumeDescriptorPointer>(), 512);
}
