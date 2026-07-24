//! Anchor Volume Descriptor Pointer (ECMA-167 3/10.2)

use super::super::super::{Read, Seek, SeekFrom};
use super::{DescriptorTag, ExtentDescriptor, TagIdentifier};
use crate::error::{Error, Result};

/// Anchor Volume Descriptor Pointer (AVDP)
///
/// Located at sector 256 (and optionally at N-256 and N where N is last sector)
///
/// @hadris-spec ECMA-167:3/10.2
/// @hadris-compliance full
/// @hadris-tests integration_external::write_tests::test_hadris_udf_has_valid_avdp
/// @hadris-fuzz udf_read
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
    pub async fn read<R: Read + Seek>(reader: &mut R, location: u32) -> Result<Self> {
        reader.seek(SeekFrom::Start((location as u64) * 2048)).await?;

        let mut buffer = [0u8; 512];
        reader.read_exact(&mut buffer).await?;

        DescriptorTag::validate_bytes(
            &buffer,
            TagIdentifier::AnchorVolumeDescriptorPointer,
            location,
        )?;
        let avdp = (*bytemuck::from_bytes::<Self>(&buffer)).from_disk();
        avdp.validate(location)?;
        Ok(avdp)
    }

    /// Find and read the AVDP from standard locations
    ///
    /// Tries sector 256 first, then N-256, then N (last sector)
    pub async fn find<R: Read + Seek>(reader: &mut R, total_sectors: Option<u64>) -> Result<Self> {
        // Try sector 256 first (always present)
        if let Ok(avdp) = Self::read(reader, Self::LOCATION_256).await {
            return Ok(avdp);
        }

        // Try N-256 and N if we know the disk size
        if let Some(n) = total_sectors {
            if n > 256
                && let Ok(avdp) = Self::read(reader, (n - 256) as u32).await {
                    return Ok(avdp);
                }
            if let Ok(avdp) = Self::read(reader, (n - 1) as u32).await {
                return Ok(avdp);
            }
        }

        Err(Error::NoAnchor)
    }

    /// Validate this AVDP
    fn validate(&self, location: u32) -> Result<()> {
        self.tag
            .validate(TagIdentifier::AnchorVolumeDescriptorPointer, location)?;
        Ok(())
    }

    pub(crate) fn from_disk(mut self) -> Self {
        self.tag = DescriptorTag::from_disk_bytes(bytemuck::bytes_of(&self.tag))
            .expect("DescriptorTag has its fixed on-disk size");
        self.main_vds_extent = self.main_vds_extent.from_disk();
        self.reserve_vds_extent = self.reserve_vds_extent.from_disk();
        self
    }
}

} // io_transform!

#[cfg(test)]
mod tests {
    use super::*;

    static_assertions::const_assert_eq!(size_of::<AnchorVolumeDescriptorPointer>(), 512);
}
