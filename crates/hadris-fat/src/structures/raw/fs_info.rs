#[repr(C, packed)]
#[derive(Clone, Copy, bytemuck::NoUninit, bytemuck::AnyBitPattern)]
pub struct RawFsInfo {
    /// FSI_LeadSig
    ///
    /// The lead signature, this have to be 0x41615252, or 'RRaA'
    pub signature: [u8; 4],
    /// FSI_Reserved1
    pub reserved1_1: [u8; 256],
    pub reserved1_2: [u8; 128],
    pub reserved1_3: [u8; 64],
    pub reserved1_4: [u8; 32],
    /// FSI_StrucSig
    ///
    /// The structure signature, this have to be 0x61417272, or 'rrAa'
    pub structure_signature: [u8; 4],
    /// FSI_Free_Count
    ///
    /// The number of free clusters, this have to be bigger than 0, and less than or equal to the
    /// total number of clusters
    /// This should remove any used clusters for headers, FAT tables, etc...
    pub free_count: [u8; 4],
    /// FSI_Nxt_Free
    ///
    /// The next free cluster number, this have to be bigger than 2, and less than or equal to the
    pub next_free: [u8; 4],
    /// FSI_Reserved2
    pub reserved2: [u8; 12],
    /// FSI_TrailSig
    ///
    /// The trail signature, this have to be 0xAA550000
    pub trail_signature: [u8; 4],
}

impl RawFsInfo {
    pub fn from_bytes(bytes: &[u8; 512]) -> &RawFsInfo {
        bytemuck::cast_ref(bytes)
    }

    pub fn from_bytes_mut(bytes: &mut [u8; 512]) -> &RawFsInfo {
        bytemuck::cast_mut(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};
    use static_assertions::const_assert_eq;

    const_assert_eq!(size_of::<RawFsInfo>(), 512);
    const_assert_eq!(offset_of!(RawFsInfo, signature), 0);
    const_assert_eq!(offset_of!(RawFsInfo, reserved1_1), 4);
    const_assert_eq!(offset_of!(RawFsInfo, structure_signature), 484);
    const_assert_eq!(offset_of!(RawFsInfo, free_count), 488);
    const_assert_eq!(offset_of!(RawFsInfo, next_free), 492);
    const_assert_eq!(offset_of!(RawFsInfo, reserved2), 496);
    const_assert_eq!(offset_of!(RawFsInfo, trail_signature), 508);

    /// The MKFS.FAT fsinfo generated from the mkfs.fat -F 32 command on a 100MB FAT32 partition
    const MKFS_FAT_FSINFO: [u8; 512] = [
        82, 82, 97, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 114, 114, 65, 97, 143, 19, 3, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 85, 170,
    ];

    #[test]
    fn test_fs_info() {
        let fs_info = RawFsInfo::from_bytes(&MKFS_FAT_FSINFO);
        // RRaA
        assert_eq!(u32::from_le_bytes(fs_info.signature), 0x41615252);
        // rrAa
        assert_eq!(u32::from_le_bytes(fs_info.structure_signature), 0x61417272);
        assert_eq!(u32::from_le_bytes(fs_info.trail_signature), 0xAA550000);
    }
}
