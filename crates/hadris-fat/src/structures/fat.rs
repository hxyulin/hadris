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
    pub fn from_bytes<'a>(bytes: &'a [u8]) -> &'a Self {
        assert!(bytes.len() % 4 == 0);
        let entries = bytemuck::cast_slice::<u8, u32>(bytes);
        // SAFETY: 'Directory' is repr(transparent) over '[DirectoryEntry]'
        // so the fat pointer is safe to cast to a thin pointer
        unsafe { &*(entries as *const [u32] as *const Fat32) }
    }

    pub fn from_bytes_mut<'a>(bytes: &'a mut [u8]) -> &'a mut Self {
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

    pub fn allocate_clusters(
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

    pub fn write_data(
        &self,
        cluster_data: &mut [u8],
        cluster_size: usize,
        cluster: u32,
        data: &[u8],
    ) {
        let mut next_cluster = cluster;
        let mut offset = (cluster as usize - 2) * cluster_size;
        let mut data_offset = 0;
        loop {
            let len = data.len().min(cluster_size);
            let data_len = len.min(data.len() - data_offset);
            cluster_data[offset..offset + len]
                .copy_from_slice(&data[data_offset..data_offset + data_len]);
            next_cluster = self.entries[next_cluster as usize] as u32;
            if next_cluster == constants::FAT32_CLUSTER_LAST {
                break;
            }
            offset = (next_cluster as usize - 2) * cluster_size;
            data_offset += len;
        }
    }

    pub fn read_data(
        &self,
        cluster_data: &[u8],
        cluster_size: usize,
        mut cluster: u32,
        data: &mut [u8],
    ) {
        let mut data_offset = 0;

        while data_offset < data.len() {
            let offset = (cluster as usize - 2) * cluster_size;
            let remaining_data = data.len() - data_offset;
            let read_size = cluster_size.min(remaining_data);

            data[data_offset..data_offset + read_size]
                .copy_from_slice(&cluster_data[offset..offset + read_size]);

            data_offset += read_size;
            cluster = self.entries[cluster as usize];
            if cluster == constants::FAT32_CLUSTER_LAST {
                break;
            }
        }
    }
}
