//! Constants used in the FAT structures
//!
//! These constants are stored in native endianness, which should be converted to little endian, or
//! use the version with the 'LE' suffix, or 'BYTES' suffix for byte arrays
//!
//! The constants defined are:
//! - CLUSTER_FREE - indicates a free cluster
//! - CLUSTER_BAD - indicates a bad cluster, which should not be used
//! - CLUSTER_RESERVED - indicates a reserved cluster, which should not be used
//! - CLUSTER_END - indicates the end of the cluster chain
//! - CLEAN_SHUTDOWN_BIT_MASK - if the bit is set, the volume is clean
//! - RW_NOERROR_BIT_MASK - if the bit is set, the volume has not encountered an error

/// FAT16 cluster values
pub mod fat16 {
    pub const CLUSTER_FREE: u16 = 0x0000;
    pub const CLUSTER_BAD: u16 = 0xFFF7;
    pub const CLUSTER_RESERVED: u16 = 0xFFF8;
    pub const CLUSTER_END: u16 = 0xFFFF;
    pub const CLEAN_SHUTDOWN_BIT_MASK: u16 = 0x8000;
    pub const RW_NOERROR_BIT_MASK: u16 = 0x4000;

    pub const CLUSTER_FREE_LE: u16 = CLUSTER_FREE.to_le();
    pub const CLUSTER_BAD_LE: u16 = CLUSTER_BAD.to_le();
    pub const CLUSTER_RESERVED_LE: u16 = CLUSTER_RESERVED.to_le();
    pub const CLUSTER_END_LE: u16 = CLUSTER_END.to_le();
    pub const CLEAN_SHUTDOWN_BIT_MASK_LE: u16 = CLEAN_SHUTDOWN_BIT_MASK.to_le();
    pub const RW_NOERROR_BIT_MASK_LE: u16 = RW_NOERROR_BIT_MASK.to_le();

    pub const CLUSTER_FREE_BYTES: [u8; 2] = CLUSTER_FREE.to_le_bytes();
    pub const CLUSTER_BAD_BYTES: [u8; 2] = CLUSTER_BAD.to_le_bytes();
    pub const CLUSTER_RESERVED_BYTES: [u8; 2] = CLUSTER_RESERVED.to_le_bytes();
    pub const CLUSTER_END_BYTES: [u8; 2] = CLUSTER_END.to_le_bytes();
    pub const CLEAN_SHUTDOWN_BIT_MASK_BYTES: [u8; 2] = CLEAN_SHUTDOWN_BIT_MASK.to_le_bytes();
    pub const RW_NOERROR_BIT_MASK_BYTES: [u8; 2] = RW_NOERROR_BIT_MASK.to_le_bytes();
}

/// FAT32 cluster values
/// Note:
/// The top four bits must be preserved when reading and writing the cluster value
pub mod fat32 {
    pub const CLUSTER_FREE: u32 = 0x00000000;
    pub const CLUSTER_MAX: u32 = 0x0FFFFFF6;
    pub const CLUSTER_BAD: u32 = 0x0FFFFFF7;
    pub const CLUSTER_RESERVED: u32 = 0x0FFFFFF8;
    pub const CLUSTER_END: u32 = 0x0FFFFFFF;
    pub const CLEAN_SHUTDOWN_BIT_MASK: u32 = 0x08000000;
    pub const RW_NOERROR_BIT_MASK: u32 = 0x04000000;

    pub const CLUSTER_FREE_LE: u32 = CLUSTER_FREE.to_le();
    pub const CLUSTER_BAD_LE: u32 = CLUSTER_BAD.to_le();
    pub const CLUSTER_RESERVED_LE: u32 = CLUSTER_RESERVED.to_le();
    pub const CLUSTER_END_LE: u32 = CLUSTER_END.to_le();
    pub const CLEAN_SHUTDOWN_BIT_MASK_LE: u32 = CLEAN_SHUTDOWN_BIT_MASK.to_le();
    pub const RW_NOERROR_BIT_MASK_LE: u32 = RW_NOERROR_BIT_MASK.to_le();

    pub const CLUSTER_FREE_BYTES: [u8; 4] = CLUSTER_FREE.to_le_bytes();
    pub const CLUSTER_BAD_BYTES: [u8; 4] = CLUSTER_BAD.to_le_bytes();
    pub const CLUSTER_RESERVED_BYTES: [u8; 4] = CLUSTER_RESERVED.to_le_bytes();
    pub const CLUSTER_END_BYTES: [u8; 4] = CLUSTER_END.to_le_bytes();
    pub const CLEAN_SHUTDOWN_BIT_MASK_BYTES: [u8; 4] = CLEAN_SHUTDOWN_BIT_MASK.to_le_bytes();
    pub const RW_NOERROR_BIT_MASK_BYTES: [u8; 4] = RW_NOERROR_BIT_MASK.to_le_bytes();
}
