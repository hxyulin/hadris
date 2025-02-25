use super::raw::fs_info::RawFsInfo;

#[repr(transparent)]
#[derive(Clone, Copy, bytemuck::NoUninit, bytemuck::AnyBitPattern)]
pub struct FsInfo {
    raw: RawFsInfo,
}

impl FsInfo {
    pub fn from_bytes<'a>(bytes: &'a [u8]) -> &'a Self {
        bytemuck::from_bytes::<Self>(bytes)
    }
}

#[cfg(feature = "write")]
impl FsInfo {
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    pub fn with_ops(ops: &super::Fat32Ops, used_clusters: u32) -> Self {
        let clusters = ops.total_sectors_32 / ops.sectors_per_cluster as u32;
        const FIRST_FREE: u32 = 3;
        Self {
            raw: RawFsInfo {
                signature: 0x41615252_u32.to_le_bytes(),
                reserved1_1: [0; 256],
                reserved1_2: [0; 128],
                reserved1_3: [0; 64],
                reserved1_4: [0; 32],
                structure_signature: 0x61417272_u32.to_le_bytes(),
                free_count: u32::from(clusters - used_clusters).to_le_bytes(),
                next_free: FIRST_FREE.to_le_bytes(),
                reserved2: [0; 12],
                trail_signature: 0xAA550000_u32.to_le_bytes(),
            },
        }
    }

    pub fn write(&self, bytes: &mut [u8]) {
        bytes.copy_from_slice(bytemuck::bytes_of(self));
    }

    pub fn set_free_clusters(&mut self, free_clusters: u32) {
        self.raw.free_count = free_clusters.to_le_bytes();
    }

    pub fn set_next_free_cluster(&mut self, next_free_cluster: u32) {
        self.raw.next_free = next_free_cluster.to_le_bytes();
    }
}

#[cfg(feature = "read")]
impl FsInfo {
    pub fn from_bytes_mut<'a>(bytes: &'a mut [u8]) -> &'a mut Self {
        bytemuck::from_bytes_mut::<Self>(bytes)
    }


    pub fn free_clusters(&self) -> u32 {
        u32::from_le_bytes(self.raw.free_count)
    }

    pub fn info(&self) -> FsInfoInfo {
        FsInfoInfo {
            free_clusters: u32::from_le_bytes(self.raw.free_count),
            next_free_cluster: u32::from_le_bytes(self.raw.next_free),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FsInfoInfo {
    pub free_clusters: u32,
    pub next_free_cluster: u32,
}
