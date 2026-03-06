//! UDF Filesystem main module

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

use super::super::{Read, Seek, SeekFrom};

use super::descriptor::{
    self, AnchorVolumeDescriptorPointer, ExtentDescriptor, FileSetDescriptor,
    LogicalVolumeDescriptor, LongAllocationDescriptor, PartitionDescriptor,
    PrimaryVolumeDescriptor, TagIdentifier, parse_vrs,
};
use crate::dir::{
    FileCharacteristics, FileIdentifierDescriptor, UdfDir, UdfDirEntry, decode_filename,
};
use crate::error::{UdfError, UdfResult};
use crate::file::{AllocationType, ExtendedFileEntry, FileEntry};
use crate::{SECTOR_SIZE, UdfRevision};

/// UDF filesystem information
#[derive(Debug, Clone)]
pub struct UdfInfo {
    /// Logical block size (usually 2048)
    pub block_size: u32,
    /// Partition start location (in sectors)
    pub partition_start: u32,
    /// Partition length (in blocks)
    pub partition_length: u32,
    /// Volume identifier
    pub volume_id: String,
    /// UDF revision
    pub udf_revision: UdfRevision,
}

/// Main UDF filesystem handle
pub struct UdfFs<DATA: Read + Seek> {
    data: Mutex<DATA>,
    info: UdfInfo,
    root_icb: LongAllocationDescriptor,
}

io_transform! {

impl<DATA: Read + Seek> UdfFs<DATA> {
    /// Open a UDF filesystem
    pub async fn open(mut data: DATA) -> UdfResult<Self> {
        // Parse Volume Recognition Sequence
        let vrs_type = parse_vrs(&mut data).await?;

        // Find the Anchor Volume Descriptor Pointer
        let avdp = AnchorVolumeDescriptorPointer::find(&mut data, None).await?;

        // Read the Main Volume Descriptor Sequence
        let (pvd, partition, lvd, fsd) = Self::read_vds(&mut data, &avdp.main_vds_extent).await?;

        let udf_revision = match vrs_type {
            descriptor::VrsType::Nsr02 => UdfRevision::V1_02,
            descriptor::VrsType::Nsr03 => UdfRevision::V2_01,
        };

        let info = UdfInfo {
            block_size: lvd.logical_block_size,
            partition_start: partition.partition_starting_location,
            partition_length: partition.partition_length,
            volume_id: pvd.volume_id(),
            udf_revision,
        };

        let root_icb = fsd.root_directory_icb;

        Ok(Self {
            data: Mutex::new(data),
            info,
            root_icb,
        })
    }

    /// Read the Volume Descriptor Sequence
    async fn read_vds(
        data: &mut DATA,
        extent: &ExtentDescriptor,
    ) -> UdfResult<(
        PrimaryVolumeDescriptor,
        PartitionDescriptor,
        LogicalVolumeDescriptor,
        FileSetDescriptor,
    )> {
        let start_sector = extent.location as u64;
        let num_sectors = (extent.length as u64).div_ceil(SECTOR_SIZE as u64);

        let mut pvd: Option<PrimaryVolumeDescriptor> = None;
        let mut partition: Option<PartitionDescriptor> = None;
        let mut lvd: Option<LogicalVolumeDescriptor> = None;

        let mut buffer = [0u8; SECTOR_SIZE];

        for i in 0..num_sectors {
            let sector = start_sector + i;
            data.seek(SeekFrom::Start(sector * SECTOR_SIZE as u64)).await?;
            data.read_exact(&mut buffer).await?;

            let tag: &descriptor::DescriptorTag = bytemuck::from_bytes(&buffer[..16]);

            match tag.identifier() {
                TagIdentifier::PrimaryVolumeDescriptor => {
                    let desc: PrimaryVolumeDescriptor = *bytemuck::from_bytes(&buffer);
                    pvd = Some(desc);
                }
                TagIdentifier::PartitionDescriptor => {
                    let desc: PartitionDescriptor = *bytemuck::from_bytes(&buffer);
                    partition = Some(desc);
                }
                TagIdentifier::LogicalVolumeDescriptor => {
                    let desc: LogicalVolumeDescriptor = *bytemuck::from_bytes(&buffer);
                    lvd = Some(desc);
                }
                TagIdentifier::TerminatingDescriptor => break,
                _ => continue,
            }
        }

        let pvd = pvd.ok_or(UdfError::InvalidVrs)?;
        let partition = partition.ok_or(UdfError::InvalidPartition(0))?;
        let lvd = lvd.ok_or(UdfError::InvalidVrs)?;

        // Read File Set Descriptor from the location in LVD
        let fsd_location = lvd.file_set_location();
        let fsd = Self::read_file_set_descriptor(data, &partition, &fsd_location).await?;

        Ok((pvd, partition, lvd, fsd))
    }

    /// Read the File Set Descriptor
    async fn read_file_set_descriptor(
        data: &mut DATA,
        partition: &PartitionDescriptor,
        icb: &LongAllocationDescriptor,
    ) -> UdfResult<FileSetDescriptor> {
        let sector = partition.partition_starting_location as u64 + icb.logical_block_num as u64;
        data.seek(SeekFrom::Start(sector * SECTOR_SIZE as u64)).await?;

        let mut buffer = [0u8; SECTOR_SIZE];
        data.read_exact(&mut buffer).await?;

        let fsd: FileSetDescriptor = *bytemuck::from_bytes(&buffer);
        Ok(fsd)
    }

    /// Get filesystem information
    pub fn info(&self) -> &UdfInfo {
        &self.info
    }

    /// Get the root directory
    pub async fn root_dir(&self) -> UdfResult<UdfDir> {
        self.read_directory(&self.root_icb).await
    }

    /// Read a directory from its ICB
    pub async fn read_directory(&self, icb: &LongAllocationDescriptor) -> UdfResult<UdfDir> {
        let mut data = self.data.lock();

        // Calculate absolute sector
        let sector = self.info.partition_start as u64 + icb.logical_block_num as u64;
        data.seek(SeekFrom::Start(sector * SECTOR_SIZE as u64)).await?;

        // Read the file entry
        let mut buffer = [0u8; SECTOR_SIZE];
        data.read_exact(&mut buffer).await?;

        let tag: &descriptor::DescriptorTag = bytemuck::from_bytes(&buffer[..16]);

        let (_dir_size, allocation_type, alloc_offset, alloc_length) = match tag.identifier() {
            TagIdentifier::FileEntry => {
                let fe: &FileEntry = bytemuck::from_bytes(&buffer[..FileEntry::BASE_SIZE]);
                if !fe.is_directory() {
                    return Err(UdfError::NotADirectory);
                }
                (
                    fe.size(),
                    fe.allocation_type(),
                    FileEntry::BASE_SIZE + fe.extended_attributes_length as usize,
                    fe.allocation_descriptors_length as usize,
                )
            }
            TagIdentifier::ExtendedFileEntry => {
                let efe: &ExtendedFileEntry =
                    bytemuck::from_bytes(&buffer[..ExtendedFileEntry::BASE_SIZE]);
                if !efe.is_directory() {
                    return Err(UdfError::NotADirectory);
                }
                (
                    efe.size(),
                    efe.allocation_type(),
                    ExtendedFileEntry::BASE_SIZE + efe.extended_attributes_length as usize,
                    efe.allocation_descriptors_length as usize,
                )
            }
            _ => return Err(UdfError::InvalidIcb),
        };

        // Parse directory entries
        let entries = self.parse_directory_entries(
            &mut data,
            &buffer,
            allocation_type,
            alloc_offset,
            alloc_length,
        ).await?;

        Ok(UdfDir::new(entries))
    }

    /// Parse directory entries from allocation descriptors
    async fn parse_directory_entries(
        &self,
        data: &mut DATA,
        buffer: &[u8],
        allocation_type: AllocationType,
        alloc_offset: usize,
        alloc_length: usize,
    ) -> UdfResult<Vec<UdfDirEntry>> {
        let mut entries = Vec::new();

        match allocation_type {
            AllocationType::Embedded => {
                // Data is embedded in the allocation descriptors field
                let embedded_data = &buffer[alloc_offset..alloc_offset + alloc_length];
                self.parse_fids(embedded_data, &mut entries)?;
            }
            AllocationType::Short | AllocationType::Long => {
                // Read from extent
                let alloc_data = &buffer[alloc_offset..alloc_offset + alloc_length];

                if allocation_type == AllocationType::Short {
                    // Parse short allocation descriptors
                    for chunk in alloc_data.chunks(8) {
                        if chunk.len() < 8 {
                            break;
                        }
                        let sad: &descriptor::ShortAllocationDescriptor =
                            bytemuck::from_bytes(chunk);
                        if sad.length() == 0 {
                            break;
                        }

                        let sector = self.info.partition_start as u64 + sad.extent_position as u64;
                        let extent_data = self.read_extent(data, sector, sad.length() as usize).await?;
                        self.parse_fids(&extent_data, &mut entries)?;
                    }
                } else {
                    // Parse long allocation descriptors
                    for chunk in alloc_data.chunks(16) {
                        if chunk.len() < 16 {
                            break;
                        }
                        let lad: &LongAllocationDescriptor = bytemuck::from_bytes(chunk);
                        if lad.length() == 0 {
                            break;
                        }

                        let sector =
                            self.info.partition_start as u64 + lad.logical_block_num as u64;
                        let extent_data = self.read_extent(data, sector, lad.length() as usize).await?;
                        self.parse_fids(&extent_data, &mut entries)?;
                    }
                }
            }
            AllocationType::Extended => {
                // Extended allocation descriptors not commonly used
                return Err(UdfError::InvalidIcb);
            }
        }

        Ok(entries)
    }

    /// Read an extent from disk
    async fn read_extent(&self, data: &mut DATA, sector: u64, length: usize) -> UdfResult<Vec<u8>> {
        data.seek(SeekFrom::Start(sector * SECTOR_SIZE as u64)).await?;
        let mut buffer = alloc::vec![0u8; length];
        data.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    /// Parse File Identifier Descriptors from a buffer
    fn parse_fids(&self, data: &[u8], entries: &mut Vec<UdfDirEntry>) -> UdfResult<()> {
        let mut offset = 0;

        while offset < data.len() {
            // Need at least the base FID size
            if offset + FileIdentifierDescriptor::BASE_SIZE > data.len() {
                break;
            }

            let (fid, variable_data) = match FileIdentifierDescriptor::from_bytes(&data[offset..]) {
                Ok(fid) => fid,
                Err(_) => break,
            };

            let characteristics = FileCharacteristics::from_bits_truncate(fid.file_characteristics);

            // Skip deleted entries
            if characteristics.contains(FileCharacteristics::DELETED) {
                offset += fid.total_size();
                continue;
            }

            // Extract filename
            let impl_use_len = fid.implementation_use_length as usize;
            let name_start = impl_use_len;
            let name_end = name_start + fid.file_identifier_length as usize;
            let name_data = if name_end <= variable_data.len() {
                &variable_data[name_start..name_end]
            } else {
                &[]
            };

            let name = if characteristics.contains(FileCharacteristics::PARENT) {
                String::from("..")
            } else if name_data.is_empty() {
                String::from("")
            } else {
                decode_filename(name_data)
            };

            let is_directory = characteristics.contains(FileCharacteristics::DIRECTORY);

            // Get file size (would need to read the ICB for this)
            let size = 0; // Placeholder

            entries.push(UdfDirEntry {
                name,
                is_directory,
                size,
                icb: fid.icb,
                characteristics,
            });

            offset += fid.total_size();
        }

        Ok(())
    }
}

} // io_transform!

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_udf_info() {
        let info = UdfInfo {
            block_size: 2048,
            partition_start: 257,
            partition_length: 100000,
            volume_id: String::from("TEST_VOLUME"),
            udf_revision: UdfRevision::V2_01,
        };
        assert_eq!(info.block_size, 2048);
        assert_eq!(info.volume_id, "TEST_VOLUME");
    }
}
