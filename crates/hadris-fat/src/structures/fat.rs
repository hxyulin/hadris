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

#[repr(transparent)]
pub struct Fat32 {
    pub entries: [u32],
}

impl Fat32 {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        assert!(bytes.len() % 4 == 0);
        let entries = bytemuck::cast_slice::<u8, u32>(bytes);
        // SAFETY: 'Directory' is repr(transparent) over '[DirectoryEntry]'
        // so the fat pointer is safe to cast to a thin pointer
        unsafe { &*(entries as *const [u32] as *const Fat32) }
    }

    pub fn from_bytes_mut(bytes: &mut [u8]) -> &mut Self {
        assert!(bytes.len() % 4 == 0);
        let entries = bytemuck::cast_slice_mut::<u8, u32>(bytes);
        // SAFETY: 'Directory' is repr(transparent) over '[DirectoryEntry]'
        // so the fat pointer is safe to cast to a thin pointer
        unsafe { &mut *(entries as *mut [u32] as *mut Fat32) }
    }

    pub fn init(&mut self) {
        assert!(self.entries.len() >= 2);
        self.entries[0] = 0xFFFF_FFF8;
        self.entries[1] = 0xFFFF_FFFF;
    }

    /// Returns the amount of entries per sector, given the sector size in bytes
    pub fn entries_per_sector(sector_size: usize) -> usize {
        // sector_size / 4
        sector_size >> 2
    }

    /// Returns the size of the FAT in sectors, rounded up
    pub fn fat_size(&self, sector_size: usize) -> usize {
        // Each entry is 4 bytes
        (self.entries.len() * 4 + (sector_size - 1)) / sector_size
    }

    /// Attempts to allocate at most 'max_clusters' contiguous clusters, returning the first
    /// cluster index and allocated cluster count
    pub fn allocate_contiguous_clusters(
        &mut self,
        free_count: &mut u32,
        next_free: &mut u32,
        max_clusters: u32,
    ) -> (u32, u32) {
        let mut counter = 1;
        let first_cluster = *next_free as usize;
        let mut cluster = first_cluster;
        debug_assert!(cluster != 0xFFFF_FFFF);
        debug_assert!(self.entries[cluster] == constants::FAT32_CLUSTER_FREE);
        while counter < max_clusters {
            // We preemptively link the next cluster to the current one
            self.mark_cluster_as(cluster, cluster as u32 + 1);

            if cluster + 1 >= self.entries.len()
                || self.entries[cluster + 1] != constants::FAT32_CLUSTER_FREE
            {
                break;
            }

            cluster += 1;
            counter += 1;
        }
        // We remark the cluster as allocated
        self.mark_cluster_as(cluster, constants::FAT32_CLUSTER_LAST);
        *free_count -= counter;
        *next_free = self.find_free_cluster().unwrap() as u32;
        (first_cluster as u32, counter)
    }

    /// Allocates the specified number of clusters
    pub fn allocate_clusters_fragmented(
        &mut self,
        free_count: &mut u32,
        next_free: &mut u32,
        count: u32,
    ) -> u32 {
        assert!(*free_count > 0 && *free_count != 0xFFFF_FFFF);
        assert!(*next_free != 0xFFFF_FFFF);
        let mut free_cluster = next_free.clone();
        let mut counter = count;
        // We need to mark it as not free, otherwise we will find the same cluster again
        self.mark_cluster_as(free_cluster as usize, constants::FAT32_CLUSTER_LAST);
        counter -= 1;
        while counter > 0 {
            let next_free_new = self.find_free_cluster().unwrap();
            self.link_cluster(free_cluster as usize, next_free_new as usize);
            free_cluster = next_free_new as u32;
            self.mark_cluster_as(free_cluster as usize, constants::FAT32_CLUSTER_LAST);
            counter -= 1;
        }
        *free_count -= count;
        *next_free
    }

    pub fn allocate_clusters(
        &mut self,
        free_count: &mut u32,
        next_free: &mut u32,
        count: u32,
    ) -> u32 {
        // We choose between fragmented and contiguous allocation based on the count
        const FRAGMENTED_THRESHOLD: u32 = 4;
        if count <= FRAGMENTED_THRESHOLD {
            self.allocate_clusters_fragmented(free_count, next_free, count)
        } else {
            let mut allocated = 0;
            let mut first_cluster = 0;
            let mut last_cluster = 0;
            while allocated < count {
                // TODO: Add testing for this case, because this is not tested when the loop is ran
                // multiple times
                let (cluster, allocated_this) =
                    self.allocate_contiguous_clusters(free_count, next_free, count - allocated);
                if first_cluster == 0 {
                    first_cluster = cluster;
                } else {
                    self.link_cluster(last_cluster, cluster as usize);
                }
                last_cluster = cluster as usize;
                allocated += allocated_this;
            }
            first_cluster
        }
    }

    #[inline]
    pub fn find_free_cluster(&self) -> Option<usize> {
        for (i, entry) in self.entries.iter().enumerate() {
            if *entry == constants::FAT32_CLUSTER_FREE {
                return Some(i);
            }
        }

        None
    }

    pub fn link_cluster(&mut self, base: usize, next: usize) {
        assert!(self.entries[base] == constants::FAT32_CLUSTER_LAST);
        self.mark_cluster_as(base, next as u32);
    }

    pub fn mark_cluster_as(&mut self, cluster: usize, value: u32) {
        self.entries[cluster] = value;
    }

    /// Retains the cluster chain starting at the given cluster
    ///
    /// The cluster chain is retained until the length is reached, or allocates more space
    pub fn retain_cluster_chain(
        &mut self,
        mut cluster: usize,
        length: u32,
        free_count: &mut u32,
        next_free: &mut u32,
    ) {
        let mut counter = 0;
        while counter < length {
            let value = self.entries[cluster];
            if value == constants::FAT32_CLUSTER_LAST {
                let to_allocate = length - counter;
                self.allocate_clusters(free_count, next_free, to_allocate);
                cluster = self.find_free_cluster().unwrap();
                break;
            }
            cluster = value as usize;
            counter += 1;
        }
        *free_count -= length;
        *next_free = cluster as u32;
    }

    pub fn write_data(
        &self,
        cluster_data: &mut [u8],
        cluster_size: usize,
        mut cluster: u32,
        offset: usize,
        data: &[u8],
    ) -> usize {
        let mut data_offset = 0;
        let mut bytes_written = 0;

        while bytes_written < data.len() {
            let new_offset = (cluster as usize - 2) * cluster_size;
            let remaining_data = data.len() - bytes_written;

            // We only start reading if the current cluster contains the offset
            if data_offset + cluster_size >= offset {
                // This is the offset within the cluster
                let cluster_offset = if data_offset >= offset {
                    0
                } else {
                    offset - data_offset
                };
                let write_size = (cluster_size - cluster_offset).min(remaining_data);
                cluster_data[new_offset + cluster_offset..new_offset + cluster_offset + write_size]
                    .copy_from_slice(&data[bytes_written..bytes_written + write_size]);
                bytes_written += write_size;
                data_offset += write_size;
            } else {
                // We can just skip this cluster
                data_offset += cluster_size;
            }
            cluster = self.entries[cluster as usize];
            if cluster == constants::FAT32_CLUSTER_LAST {
                break;
            }
        }

        bytes_written
    }

    pub fn read_data(
        &self,
        cluster_data: &[u8],
        cluster_size: usize,
        mut cluster: u32,
        offset: usize,
        data: &mut [u8],
    ) -> usize {
        let mut data_offset = 0;
        let mut bytes_read = 0;

        while bytes_read < data.len() {
            let new_offset = (cluster as usize - 2) * cluster_size;
            let remaining_data = data.len() - bytes_read;

            // We only start reading if the current cluster contains the offset
            if data_offset + cluster_size >= offset {
                // This is the offset within the cluster
                let cluster_offset = if data_offset >= offset {
                    0
                } else {
                    offset - data_offset
                };
                let read_size = (cluster_size - cluster_offset).min(remaining_data);
                data[bytes_read..bytes_read + read_size].copy_from_slice(
                    &cluster_data
                        [new_offset + cluster_offset..new_offset + cluster_offset + read_size],
                );
                bytes_read += read_size;
            }
            data_offset += cluster_size;
            cluster = self.entries[cluster as usize];
            if cluster == constants::FAT32_CLUSTER_LAST {
                break;
            }
        }

        bytes_read
    }
}
