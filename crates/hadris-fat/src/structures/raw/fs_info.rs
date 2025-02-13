#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFsInfo {
    /// FSI_LeadSig
    ///
    /// The lead signature, this have to be 0x41615252, or 'RRaA'
    pub signature: [u8; 4],
    /// FSI_Reserved1
    pub reserved1: [u8; 480],
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};
    use static_assertions::const_assert_eq;

    const_assert_eq!(size_of::<RawFsInfo>(), 512);
    const_assert_eq!(offset_of!(RawFsInfo, signature), 0);
    const_assert_eq!(offset_of!(RawFsInfo, reserved1), 4);
    const_assert_eq!(offset_of!(RawFsInfo, structure_signature), 484);
    const_assert_eq!(offset_of!(RawFsInfo, free_count), 488);
    const_assert_eq!(offset_of!(RawFsInfo, next_free), 492);
    const_assert_eq!(offset_of!(RawFsInfo, reserved2), 496);
    const_assert_eq!(offset_of!(RawFsInfo, trail_signature), 508);
}
