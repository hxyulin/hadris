#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct FsInfo {
    /// FSI_LeadSig
    ///
    /// The lead signature, this have to be 0x41615252, or 'RRaA'
    pub signature: u32,
    /// FSI_Reserved1
    pub reserved1: [u8; 480],
    /// FSI_StrucSig
    ///
    /// The structure signature, this have to be 0x61417272, or 'rrAa'
    pub structure_signature: u32,
    /// FSI_Free_Count
    ///
    /// The number of free clusters, this have to be bigger than 0, and less than or equal to the
    /// total number of clusters
    /// This should remove any used clusters for headers, FAT tables, etc...
    pub free_count: u32,
    /// FSI_Nxt_Free
    ///
    /// The next free cluster number, this have to be bigger than 2, and less than or equal to the
    pub next_free: u32,
    /// FSI_Reserved2
    pub reserved2: [u8; 12],
    /// FSI_TrailSig
    ///
    /// The trail signature, this have to be 0xAA550000
    pub trail_signature: u32,
}

impl core::fmt::Debug for FsInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let signature = self.signature;
        let structure_signature = self.structure_signature;
        let free_count = self.free_count;
        let next_free = self.next_free;
        let trail_signature = self.trail_signature;
        f.debug_struct("Fat32FsInfo")
            .field("signature", &signature)
            .field("structure_signature", &structure_signature)
            .field("free_count", &free_count)
            .field("next_free", &next_free)
            .field("trail_signature", &trail_signature)
            .finish()
    }
}

impl FsInfo {
    pub fn from_bytes<'a>(bytes: &'a [u8]) -> &'a Self {
        bytemuck::from_bytes::<Self>(bytes)
    }
}

#[cfg(feature = "write")]
impl FsInfo {
    pub fn from_bytes_mut<'a>(bytes: &'a mut [u8]) -> &'a mut Self {
        bytemuck::from_bytes_mut::<Self>(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    pub fn with_ops(ops: &super::Fat32Ops, used_clusters: u32) -> Self {
        let clusters = ops.total_sectors_32 / ops.sectors_per_cluster as u32;
        const FIRST_FREE: u32 = 2;
        Self {
            signature: 0x41615252,
            reserved1: [0; 480],
            structure_signature: 0x61417272,
            free_count: clusters - used_clusters,
            next_free: FIRST_FREE,
            reserved2: [0; 12],
            trail_signature: 0xAA550000,
        }
    }

    pub fn write(&self, bytes: &mut [u8]) {
        bytes.copy_from_slice(bytemuck::bytes_of(self));
    }
}

unsafe impl bytemuck::Zeroable for FsInfo {}
unsafe impl bytemuck::NoUninit for FsInfo {}
unsafe impl bytemuck::AnyBitPattern for FsInfo {}
