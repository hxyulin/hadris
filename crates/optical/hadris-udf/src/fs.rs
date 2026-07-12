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
use crate::descriptor::DescriptorTag;
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

            let tag: &DescriptorTag = bytemuck::try_from_bytes(&buffer[..size_of::<DescriptorTag>()]).map_err(|err| UdfError::PodCastError(err))?;

            match tag.identifier() {
                TagIdentifier::PrimaryVolumeDescriptor => {
                    let desc: PrimaryVolumeDescriptor = *bytemuck::try_from_bytes(&buffer[..size_of::<PrimaryVolumeDescriptor>()]).map_err(|err| UdfError::PodCastError(err))?;
                    pvd = Some(desc);
                }
                TagIdentifier::PartitionDescriptor => {
                    let desc: PartitionDescriptor = *bytemuck::try_from_bytes(&buffer[..size_of::<PartitionDescriptor>()]).map_err(|err| UdfError::PodCastError(err))?;
                    partition = Some(desc);
                }
                TagIdentifier::LogicalVolumeDescriptor => {
                    let desc: LogicalVolumeDescriptor = *bytemuck::try_from_bytes(&buffer[..size_of::<LogicalVolumeDescriptor>()]).map_err(|err| UdfError::PodCastError(err))?;
                    lvd = Some(desc);
                }
                TagIdentifier::TerminatingDescriptor => break,
                _ => continue,
            }
        }

        let pvd = pvd.ok_or_else(|| UdfError::InvalidVds("PVD"))?;
        let partition = partition.ok_or_else(|| UdfError::InvalidPartition(0))?;
        let lvd = lvd.ok_or_else(|| UdfError::InvalidVds("LVD"))?;

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

        let fsd: FileSetDescriptor = *bytemuck::try_from_bytes(&buffer[..size_of::<FileSetDescriptor>()]).map_err(|_| UdfError::InvalidFsd)?;
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

    /// Read the full contents of a regular file.
    ///
    /// Follows the entry's ICB allocation descriptors (embedded, short, or long)
    /// and truncates the result to the File Entry `information_length`.
    pub async fn read_file(&self, entry: &UdfDirEntry) -> UdfResult<Vec<u8>> {
        if entry.is_dir() {
            return Err(UdfError::NotAFile);
        }

        let mut data = self.data.lock();
        let meta = self.read_icb(&mut data, &entry.icb).await?;
        if meta.is_directory {
            return Err(UdfError::NotAFile);
        }

        let mut bytes = self
            .read_allocation_bytes(
                &mut data,
                &meta.buffer,
                meta.allocation_type,
                meta.alloc_offset,
                meta.alloc_length,
            )
            .await?;

        let size = usize::try_from(meta.size).map_err(|_| UdfError::InvalidIcb)?;
        if bytes.len() < size {
            return Err(UdfError::InvalidIcb);
        }
        bytes.truncate(size);
        Ok(bytes)
    }

    /// Read a directory from its ICB
    pub async fn read_directory(&self, icb: &LongAllocationDescriptor) -> UdfResult<UdfDir> {
        let mut data = self.data.lock();
        let meta = self.read_icb(&mut data, icb).await?;
        if !meta.is_directory {
            return Err(UdfError::NotADirectory);
        }

        let entries = self
            .parse_directory_entries(
                &mut data,
                &meta.buffer,
                meta.allocation_type,
                meta.alloc_offset,
                meta.alloc_length,
            )
            .await?;

        Ok(UdfDir::new(entries))
    }

    /// Read and parse a File Entry / Extended File Entry ICB.
    async fn read_icb(
        &self,
        data: &mut DATA,
        icb: &LongAllocationDescriptor,
    ) -> UdfResult<IcbMetadata> {
        let sector = self.info.partition_start as u64 + icb.logical_block_num as u64;
        data.seek(SeekFrom::Start(sector * SECTOR_SIZE as u64))
            .await?;

        let mut buffer = [0u8; SECTOR_SIZE];
        data.read_exact(&mut buffer).await?;

        let tag: &descriptor::DescriptorTag = bytemuck::from_bytes(&buffer[..16]);

        let (size, is_directory, allocation_type, alloc_offset, alloc_length) =
            match tag.identifier() {
                TagIdentifier::FileEntry => {
                    let fe: &FileEntry = bytemuck::from_bytes(&buffer[..FileEntry::BASE_SIZE]);
                    (
                        fe.size(),
                        fe.is_directory(),
                        fe.allocation_type(),
                        FileEntry::BASE_SIZE + fe.extended_attributes_length as usize,
                        fe.allocation_descriptors_length as usize,
                    )
                }
                TagIdentifier::ExtendedFileEntry => {
                    let efe: &ExtendedFileEntry =
                        bytemuck::from_bytes(&buffer[..ExtendedFileEntry::BASE_SIZE]);
                    (
                        efe.size(),
                        efe.is_directory(),
                        efe.allocation_type(),
                        ExtendedFileEntry::BASE_SIZE + efe.extended_attributes_length as usize,
                        efe.allocation_descriptors_length as usize,
                    )
                }
                _ => return Err(UdfError::InvalidIcb),
            };

        Ok(IcbMetadata {
            size,
            is_directory,
            allocation_type,
            alloc_offset,
            alloc_length,
            buffer,
        })
    }

    /// Concatenate bytes described by allocation descriptors in an ICB buffer.
    async fn read_allocation_bytes(
        &self,
        data: &mut DATA,
        buffer: &[u8],
        allocation_type: AllocationType,
        alloc_offset: usize,
        alloc_length: usize,
    ) -> UdfResult<Vec<u8>> {
        let alloc_range = validated_alloc_range(buffer.len(), alloc_offset, alloc_length)?;
        let mut out = Vec::new();

        match allocation_type {
            AllocationType::Embedded => {
                out.extend_from_slice(&buffer[alloc_range]);
            }
            AllocationType::Short => {
                let alloc_data = &buffer[alloc_range];
                for chunk in alloc_data.chunks(8) {
                    if chunk.len() < 8 {
                        break;
                    }
                    let sad: &descriptor::ShortAllocationDescriptor = bytemuck::from_bytes(chunk);
                    if sad.length() == 0 {
                        break;
                    }
                    let sector = self.info.partition_start as u64 + sad.extent_position as u64;
                    let extent = self
                        .read_extent(data, sector, sad.length() as usize)
                        .await?;
                    out.extend_from_slice(&extent);
                }
            }
            AllocationType::Long => {
                let alloc_data = &buffer[alloc_range];
                for chunk in alloc_data.chunks(16) {
                    if chunk.len() < 16 {
                        break;
                    }
                    let lad: &LongAllocationDescriptor = bytemuck::from_bytes(chunk);
                    if lad.length() == 0 {
                        break;
                    }
                    let sector = self.info.partition_start as u64 + lad.logical_block_num as u64;
                    let extent = self
                        .read_extent(data, sector, lad.length() as usize)
                        .await?;
                    out.extend_from_slice(&extent);
                }
            }
            AllocationType::Extended => return Err(UdfError::InvalidIcb),
        }

        Ok(out)
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
        let dir_bytes = self
            .read_allocation_bytes(data, buffer, allocation_type, alloc_offset, alloc_length)
            .await?;
        self.parse_fids(data, &dir_bytes, &mut entries).await?;
        Ok(entries)
    }

    /// Read an extent from disk
    async fn read_extent(&self, data: &mut DATA, sector: u64, length: usize) -> UdfResult<Vec<u8>> {
        let start = sector * SECTOR_SIZE as u64;
        // `length` is the untrusted extent length from an on-disk allocation
        // descriptor (30-bit, up to ~1 GiB per descriptor, and the caller chains
        // many). Bound it against the actual image size before allocating,
        // otherwise a tiny image with one bogus descriptor would force that
        // allocation up front — a DoS that aborts the process on no-overcommit
        // / embedded targets.
        let image_len = data.seek(SeekFrom::End(0)).await?;
        if start.saturating_add(length as u64) > image_len {
            return Err(UdfError::InvalidIcb);
        }
        data.seek(SeekFrom::Start(start)).await?;
        let mut buffer = alloc::vec![0u8; length];
        data.read_exact(&mut buffer).await?;
        Ok(buffer)
    }

    /// Parse File Identifier Descriptors from a buffer
    async fn parse_fids(
        &self,
        data: &mut DATA,
        fid_data: &[u8],
        entries: &mut Vec<UdfDirEntry>,
    ) -> UdfResult<()> {
        let mut offset = 0;

        while offset < fid_data.len() {
            // Need at least the base FID size
            if offset + FileIdentifierDescriptor::BASE_SIZE > fid_data.len() {
                break;
            }

            let (fid, variable_data) =
                match FileIdentifierDescriptor::from_bytes(&fid_data[offset..]) {
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

            // File size lives in the child ICB, not the FID.
            let size = if is_directory || characteristics.contains(FileCharacteristics::PARENT) {
                0
            } else {
                self.read_icb(data, &fid.icb).await?.size
            };

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

/// Parsed File Entry / Extended File Entry metadata.
struct IcbMetadata {
    size: u64,
    is_directory: bool,
    allocation_type: AllocationType,
    alloc_offset: usize,
    alloc_length: usize,
    buffer: [u8; SECTOR_SIZE],
}

} // io_transform!

/// Validate that the `[offset, offset + length)` allocation-descriptor window
/// lies within the sector-sized File Entry `buffer`.
///
/// `offset` and `length` derive from the untrusted on-disk
/// `extended_attributes_length` / `allocation_descriptors_length` (u32 each), so
/// a corrupt File Entry can point them far past the 2 KiB buffer. Without this
/// check the slice in `parse_directory_entries` panics (and on 32-bit the
/// `offset + length` addition can overflow) — a crash triggerable by a crafted
/// image, which aborts the process under `panic = "abort"` (embedded targets).
fn validated_alloc_range(
    buffer_len: usize,
    offset: usize,
    length: usize,
) -> UdfResult<core::ops::Range<usize>> {
    let end = offset.checked_add(length).ok_or(UdfError::InvalidIcb)?;
    if end > buffer_len {
        return Err(UdfError::InvalidIcb);
    }
    Ok(offset..end)
}

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

    #[test]
    fn alloc_range_rejects_out_of_bounds_and_overflow() {
        // Valid window inside a 2 KiB File Entry buffer.
        assert_eq!(validated_alloc_range(2048, 176, 100).unwrap(), 176..276);
        assert_eq!(validated_alloc_range(2048, 0, 2048).unwrap(), 0..2048);

        // A corrupt `extended_attributes_length` puts the offset past the buffer.
        assert!(validated_alloc_range(2048, 5000, 0).is_err());
        // A corrupt `allocation_descriptors_length` runs the window past the end.
        assert!(validated_alloc_range(2048, 176, 4000).is_err());
        // `offset + length` must not wrap on 32-bit usize.
        assert!(validated_alloc_range(2048, usize::MAX, 10).is_err());
    }

    fn calculate_tag_checksum(tag: &[u8; 16]) -> u8 {
        let mut sum: u8 = 0;
        for (i, &byte) in tag.iter().enumerate() {
            if i != 4 {
                sum = sum.wrapping_add(byte);
            }
        }
        sum
    }

    fn write_vrs(data: &mut [u8]) {
        let offset = 16 * 2048;
        data[offset..offset + 7].copy_from_slice(b"\0BEA01\x01");

        let offset = 17 * 2048;
        data[offset..offset + 7].copy_from_slice(b"\0NSR02\x01");

        let offset = 18 * 2048;
        data[offset..offset + 7].copy_from_slice(b"\0TEA01\x01");
    }

    fn write_avdp(data: &mut [u8]) {
        let offset = 256 * 2048;

        let mut tag = [0u8; 16];
        tag[0] = 0x02;
        tag[1] = 0x00;
        tag[2] = 0x02;
        tag[3] = 0x00;
        tag[4] = 0;
        tag[5] = 0;
        tag[6] = 0x01;
        tag[7] = 0x00;
        tag[8] = 0;
        tag[9] = 0;
        tag[10] = 0;
        tag[11] = 0;
        tag[12] = 0x00;
        tag[13] = 0x01;
        tag[14] = 0x00;
        tag[15] = 0x00;

        let checksum = calculate_tag_checksum(&tag);
        tag[4] = checksum;
        data[offset..offset + 16].copy_from_slice(&tag);

        // main_vds_extent: 4 sectors (257-260) = 8192 bytes
        data[offset + 16] = 0x00;
        data[offset + 17] = 0x20; // 8192 = 0x2000
        data[offset + 18] = 0x00;
        data[offset + 19] = 0x00;
        data[offset + 20] = 0x01;
        data[offset + 21] = 0x01; // sector 257
        data[offset + 22] = 0x00;
        data[offset + 23] = 0x00;

        // reserve_vds_extent: backup at sector 275
        data[offset + 24] = 0x00;
        data[offset + 25] = 0x20; // 8192 bytes
        data[offset + 26] = 0x00;
        data[offset + 27] = 0x00;
        data[offset + 28] = 0x13;
        data[offset + 29] = 0x01; // sector 275 = 0x0113
        data[offset + 30] = 0x00;
        data[offset + 31] = 0x00;

        data[offset + 32..offset + 56].fill(0);
    }

    fn write_primary_volume_descriptor(data: &mut [u8], sector: u32) {
        let offset = (sector as usize) * 2048;

        let mut tag = [0u8; 16];
        tag[0] = 0x01;
        tag[1] = 0x00;
        tag[2] = 0x02;
        tag[3] = 0x00;
        tag[4] = 0;
        tag[5] = 0;
        tag[6] = 0x01;
        tag[7] = 0x00;
        tag[8] = 0;
        tag[9] = 0;
        tag[10] = 0;
        tag[11] = 0;
        tag[12] = (sector & 0xFF) as u8;
        tag[13] = ((sector >> 8) & 0xFF) as u8;
        tag[14] = ((sector >> 16) & 0xFF) as u8;
        tag[15] = ((sector >> 24) & 0xFF) as u8;

        let checksum = calculate_tag_checksum(&tag);
        tag[4] = checksum;
        data[offset..offset + 16].copy_from_slice(&tag);

        let vol_id = b"MOCK_VOLUME        ";
        data[offset + 24..offset + 24 + vol_id.len()].copy_from_slice(vol_id);

        data[offset + 80] = 0x00;
        data[offset + 81] = 0x00;
        data[offset + 82] = 0x02;
        data[offset + 83] = 0x00;
    }

    fn write_partition_descriptor(data: &mut [u8], sector: u32, partition_start: u32) {
        let offset = (sector as usize) * 2048;

        let mut tag = [0u8; 16];
        tag[0] = 0x05;
        tag[1] = 0x00;
        tag[2] = 0x02;
        tag[3] = 0x00;
        tag[4] = 0;
        tag[5] = 0;
        tag[6] = 0x02;
        tag[7] = 0x00;
        tag[8] = 0;
        tag[9] = 0;
        tag[10] = 0;
        tag[11] = 0;
        tag[12] = (sector & 0xFF) as u8;
        tag[13] = ((sector >> 8) & 0xFF) as u8;
        tag[14] = ((sector >> 16) & 0xFF) as u8;
        tag[15] = ((sector >> 24) & 0xFF) as u8;

        let checksum = calculate_tag_checksum(&tag);
        tag[4] = checksum;
        data[offset..offset + 16].copy_from_slice(&tag);

        data[offset + 40] = (partition_start & 0xFF) as u8;
        data[offset + 41] = ((partition_start >> 8) & 0xFF) as u8;
        data[offset + 42] = ((partition_start >> 16) & 0xFF) as u8;
        data[offset + 43] = ((partition_start >> 24) & 0xFF) as u8;
    }

    fn write_logical_volume_descriptor(data: &mut [u8], sector: u32, fsd_sector: u32) {
        let offset = (sector as usize) * 2048;

        let mut tag = [0u8; 16];
        tag[0] = 0x06;
        tag[1] = 0x00;
        tag[2] = 0x02;
        tag[3] = 0x00;
        tag[4] = 0;
        tag[5] = 0;
        tag[6] = 0x03;
        tag[7] = 0x00;
        tag[8] = 0;
        tag[9] = 0;
        tag[10] = 0;
        tag[11] = 0;
        tag[12] = (sector & 0xFF) as u8;
        tag[13] = ((sector >> 8) & 0xFF) as u8;
        tag[14] = ((sector >> 16) & 0xFF) as u8;
        tag[15] = ((sector >> 24) & 0xFF) as u8;

        let checksum = calculate_tag_checksum(&tag);
        tag[4] = checksum;
        data[offset..offset + 16].copy_from_slice(&tag);

        // logical_block_size = 2048 at offset 16
        data[offset + 16] = 0x00;
        data[offset + 17] = 0x08;
        data[offset + 18] = 0x00;
        data[offset + 19] = 0x00;

        // file_set_location at offset 124 (LongAllocationDescriptor: 16 bytes)
        data[offset + 124] = (fsd_sector & 0xFF) as u8;
        data[offset + 125] = ((fsd_sector >> 8) & 0xFF) as u8;
        data[offset + 126] = ((fsd_sector >> 16) & 0xFF) as u8;
        data[offset + 127] = ((fsd_sector >> 24) & 0xFF) as u8;
    }

    fn write_terminating_descriptor(data: &mut [u8], sector: u32) {
        let offset = (sector as usize) * 2048;

        let mut tag = [0u8; 16];
        tag[0] = 0x08;
        tag[1] = 0x00;
        tag[2] = 0x02;
        tag[3] = 0x00;
        tag[4] = 0;
        tag[5] = 0;
        tag[6] = 0x04;
        tag[7] = 0x00;
        tag[8] = 0;
        tag[9] = 0;
        tag[10] = 0;
        tag[11] = 0;
        tag[12] = (sector & 0xFF) as u8;
        tag[13] = ((sector >> 8) & 0xFF) as u8;
        tag[14] = ((sector >> 16) & 0xFF) as u8;
        tag[15] = ((sector >> 24) & 0xFF) as u8;

        let checksum = calculate_tag_checksum(&tag);
        tag[4] = checksum;
        data[offset..offset + 16].copy_from_slice(&tag);
    }

    fn write_file_set_descriptor(data: &mut [u8], sector: u32, root_icb_sector: u32) {
        let offset = (sector as usize) * 2048;

        let mut tag = [0u8; 16];
        tag[0] = 0x00;
        tag[1] = 0x01; // Tag identifier = 256 (0x0100) for FileSetDescriptor
        tag[2] = 0x02;
        tag[3] = 0x00;
        tag[4] = 0;
        tag[5] = 0;
        tag[6] = 0x05;
        tag[7] = 0x00;
        tag[8] = 0;
        tag[9] = 0;
        tag[10] = 0;
        tag[11] = 0;
        tag[12] = (sector & 0xFF) as u8;
        tag[13] = ((sector >> 8) & 0xFF) as u8;
        tag[14] = ((sector >> 16) & 0xFF) as u8;
        tag[15] = ((sector >> 24) & 0xFF) as u8;

        let checksum = calculate_tag_checksum(&tag);
        tag[4] = checksum;
        data[offset..offset + 16].copy_from_slice(&tag);

        // root_directory_icb at offset 160 (LongAllocationDescriptor)
        data[offset + 160] = (root_icb_sector & 0xFF) as u8;
        data[offset + 161] = ((root_icb_sector >> 8) & 0xFF) as u8;
        data[offset + 162] = ((root_icb_sector >> 16) & 0xFF) as u8;
        data[offset + 163] = ((root_icb_sector >> 24) & 0xFF) as u8;
    }

    fn create_mock_udf_data() -> std::boxed::Box<[u8]> {
        let mut data = std::vec![0u8; 2048 * 300].into_boxed_slice();

        write_vrs(&mut data);
        write_avdp(&mut data);

        write_primary_volume_descriptor(&mut data, 257);
        write_partition_descriptor(&mut data, 258, 260);
        write_logical_volume_descriptor(&mut data, 259, 261);
        write_terminating_descriptor(&mut data, 260);

        write_file_set_descriptor(&mut data, 261, 262);

        data
    }

    #[test]
    fn test_udf_open() {
        let data = create_mock_udf_data();
        let cursor = std::io::Cursor::new(data);
        let result = UdfFs::open(cursor);

        assert!(result.is_ok())
    }
}
