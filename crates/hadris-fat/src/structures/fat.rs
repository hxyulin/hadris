/*
RootDirSectors = ((BPB_RootEntCnt * 32) + (BPB_BytsPerSec – 1)) / BPB_BytsPerSec;
TmpVal1 = DskSize – (BPB_ResvdSecCnt + RootDirSectors);
TmpVal2 = (256 * BPB_SecPerClus) + BPB_NumFATs;
If(FATType == FAT32)
TmpVal2 = TmpVal2 / 2;
FATSz = (TMPVal1 + (TmpVal2 – 1)) / TmpVal2;
If(FATType == FAT32) {
BPB_FATSz16 = 0;
BPB_FATSz32 = FATSz;
} else {
BPB_FATSz16 = LOWORD(FATSz);
/* there is no BPB_FATSz32 in a FAT16 BPB */
}
*/

use hadris_core::{ReadWriteError, Reader, Writer};

pub mod constants {
    pub const FAT16_CLUSTER_FREE: u16 = 0x0000;
    pub const FAT16_CLUSTER_BAD: u16 = 0xFFF7;
    pub const FAT16_CLUSTER_RESERVED: u16 = 0xFFF8;
    pub const FAT16_CLUSTER_LAST: u16 = 0xFFFF;

    pub const FAT32_CLUSTER_FREE: u32 = 0x00000000;
    pub const FAT32_CLUSTER_BAD: u32 = 0xFFFFFFF7;
    pub const FAT32_CLUSTER_RESERVED: u32 = 0xFFFFFFF8;
    pub const FAT32_CLUSTER_LAST: u32 = 0xFFFFFFFF;
}

pub struct Fat32 {
    /// The offset of the FAT in bytes
    offset: usize,
    /// The size of the FAT in bytes
    size: usize,
    /// Number of fats
    num: usize,
    /// The size of a sector in bytes
    bytes_per_sector: usize,
}

#[cfg(feature = "read")]
impl Fat32 {
    pub fn new(offset: usize, size: usize, num: usize, bytes_per_sector: usize) -> Self {
        Self {
            offset,
            size,
            num,
            bytes_per_sector,
        }
    }

    fn data_offset(&self) -> usize {
        self.offset + self.num * self.size
    }

    pub fn next_cluster_index(
        &self,
        reader: &mut dyn Reader,
        cluster: u32,
    ) -> Result<u32, ReadWriteError> {
        let offset = self.offset + cluster as usize * size_of::<u32>();
        let mut buf = [0u8; 4];
        reader.read_bytes(offset, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }

    /// Read data from a FAT
    ///
    /// The root_directory_offset is the offset of the root directory in bytes
    pub fn read_data(
        &self,
        reader: &mut dyn Reader,
        cluster_size: usize,
        mut cluster: u32,
        offset: usize,
        buffer: &mut [u8],
    ) -> Result<usize, ReadWriteError> {
        let mut data_offset = 0;
        let mut bytes_read = 0;

        while data_offset < buffer.len() {
            let new_offset = (cluster as usize - 2) * cluster_size + self.data_offset();
            if data_offset + cluster_size > offset {
                let cluster_offset = if offset > data_offset {
                    offset - data_offset
                } else {
                    0
                };
                let read_size = (cluster_size - cluster_offset).min(buffer.len() - bytes_read);
                reader.read_bytes(new_offset, &mut buffer[bytes_read..bytes_read + read_size])?;
                bytes_read += read_size;
            }
            data_offset += cluster_size;
            cluster = self.next_cluster_index(reader, cluster)?;
            if cluster < 2 || cluster > 0x0FFF_FFF6 {
                break;
            }
        }
        Ok(bytes_read)
    }
}

#[cfg(feature = "write")]
impl Fat32 {
    pub fn init(&self, writer: &mut dyn Writer) {
        // We need to write the first two entries
        let mut buffer = [0u8; 12];
        buffer[0..4].copy_from_slice(&0xFFFF_FFF8_u32.to_le_bytes());
        buffer[4..8].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        // Root directory
        buffer[8..12].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
        writer.write_bytes(self.offset, &buffer).unwrap();
    }

    pub fn allocate_clusters(
        &self,
        writer: &mut dyn Writer,
        count: u32,
        free_count: &mut u32,
        next_free: &mut u32,
    ) -> Result<u32, ReadWriteError> {
        if count == 0 {
            return Ok(0);
        }

        let mut start_cluster = next_free.clone();
        if self.next_cluster_index2(writer, start_cluster)? != constants::FAT32_CLUSTER_FREE {
            start_cluster = self.find_free_cluster(writer)?;
        }
        let mut current_cluster = start_cluster;
        for _ in 1..count {
            let next_free_new = self.find_free_cluster(writer)?;
            self.mark_cluster_as(writer, current_cluster, next_free_new)?;
            current_cluster = next_free_new;
        }
        self.mark_cluster_as(writer, current_cluster, constants::FAT32_CLUSTER_LAST)?;

        *next_free = self.find_free_cluster(writer)?;
        *free_count -= count;
        Ok(start_cluster)
    }

    fn mark_cluster_as(
        &self,
        writer: &mut dyn Writer,
        cluster: u32,
        value: u32,
    ) -> Result<(), ReadWriteError> {
        let entry_offset = self.offset + cluster as usize * size_of::<u32>();
        let mut buffer = [0u8; 4];
        buffer.copy_from_slice(&value.to_le_bytes());
        writer.write_bytes(entry_offset, &buffer)
    }

    pub fn find_free_cluster(&self, writer: &mut dyn Writer) -> Result<u32, ReadWriteError> {
        let mut buffer = [0u8; 512];
        let entries_per_sector = self.bytes_per_sector / size_of::<u32>();
        for current_cluster in 0..self.size / self.bytes_per_sector {
            let cluster_offset = self.offset + current_cluster as usize * self.bytes_per_sector;
            writer.read_bytes(cluster_offset, &mut buffer)?;
            for i in 0..entries_per_sector {
                let entry = u32::from_le_bytes(
                    buffer[i * size_of::<u32>()..i * size_of::<u32>() + size_of::<u32>()]
                        .try_into()
                        .unwrap(),
                );
                if entry == constants::FAT32_CLUSTER_FREE {
                    return Ok(current_cluster as u32);
                }
            }
        }
        Err(ReadWriteError::OutOfBounds)
    }

    pub fn write_data(
        &self,
        writer: &mut dyn Writer,
        cluster_size: usize,
        mut cluster: u32,
        offset: usize,
        data: &[u8],
    ) -> Result<usize, ReadWriteError> {
        let mut data_offset = 0;
        let mut bytes_written = 0;

        while data_offset < data.len() {
            let new_offset = (cluster as usize - 2) * cluster_size + self.data_offset();
            if data_offset + cluster_size > offset {
                let cluster_offset = if offset > data_offset {
                    offset - data_offset
                } else {
                    0
                };
                let write_size = (cluster_size - cluster_offset).min(data.len() - bytes_written);
                writer.write_bytes(new_offset, &data[bytes_written..bytes_written + write_size])?;
                bytes_written += write_size;
            }
            data_offset += cluster_size;
            cluster = self.next_cluster_index2(writer, cluster)?;
            if cluster < 2 || cluster > 0x0FFF_FFF6 {
                break;
            }
        }
        Ok(bytes_written)
    }

    // This is bit of a hack to prevent upcasting
    pub fn next_cluster_index2(
        &self,
        reader: &mut dyn Writer,
        cluster: u32,
    ) -> Result<u32, ReadWriteError> {
        let offset = self.offset + cluster as usize * size_of::<u32>();
        let mut buf = [0u8; 4];
        reader.read_bytes(offset, &mut buf)?;
        Ok(u32::from_le_bytes(buf))
    }
}
