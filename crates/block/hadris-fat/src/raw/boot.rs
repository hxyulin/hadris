//! Boot sector and FSInfo layouts.

use hadris_common::types::{
    endian::LittleEndian,
    number::{U16, U32},
};

/// The common BIOS parameter block of every FAT boot sector.
///
/// It holds only the fields shared by FAT12, FAT16 and FAT32. The extended
/// fields that follow it are [`RawBpbExt16`] and [`RawBpbExt32`].
///
/// @hadris-spec FAT:BPB
/// @hadris-compliance full
/// @hadris-tests boot::tests::check_bpb_rejects_bad_sizes, fatfs_format::rejects_bad_options_and_devices
/// @hadris-fuzz fat_read
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RawBpb {
    /// BS_jmpBoot
    pub jump: [u8; 3],
    /// BS_OEMName
    /// The name of the program that formatted the partition
    pub oem_name: [u8; 8],
    /// BPB_BytsPerSec
    /// The number of bytes per sector
    pub bytes_per_sector: U16<LittleEndian>,
    /// BPB_SecPerClus
    /// The number of sectors per cluster
    pub sectors_per_cluster: u8,
    /// BPB_RsvdSecCnt
    ///
    /// The number of reserved sectors, should be nonzero, ans should be a multiple of the sectors per cluster
    /// This is used to:
    /// 1. align the start of the filesystem to the sectors per cluster
    /// 2. Move the data (cluster 2) to the end of fat tables, so that the data can be read from the start of the filesystem
    pub reserved_sector_count: U16<LittleEndian>,
    /// BPB_NumFATs
    ///
    /// The number of fats, 1 is acceptable, but 2 is recommended
    pub fat_count: u8,
    /// BPB_RootEntCnt
    ///
    /// The number of root directory entries
    /// For FAT32, this should be 0
    /// For FAT12/16, this value multiplied by 32 should be a multiple of the bytes per sector
    /// For FAT16, it is recommended to set this to 512 for maximum compatibility
    pub root_entry_count: [u8; 2],
    /// BPB_TotSec16
    ///
    /// The number of sectors
    /// For FAT32, this should be 0
    /// For FAT16, if the number of sectors is greater than 0x10000, you should use total_sectors_32
    pub total_sectors_16: [u8; 2],
    /// BPB_Media
    ///
    /// See the MediaType enum for more information
    pub media_type: u8,
    /// BPB_FATSz16
    ///
    /// The number of sectors per fat
    /// For FAT32, this should be 0
    pub sectors_per_fat_16: [u8; 2],
    /// BPB_SecPerTrk
    ///
    /// The number of sectors per track
    /// This is only relevant for media with have a geometry and used by BIOS interrupt 0x13
    pub sectors_per_track: [u8; 2],
    /// BPB_NumHeads
    ///
    /// Similar situation as sectors_per_track
    pub num_heads: [u8; 2],
    /// BPB_HiddSec
    ///
    /// The number of hidden sectors predicing the partition that contains the FAT volume.
    /// This must be 0 on media that isn't partitioned
    pub hidden_sector_count: [u8; 4],
    /// BPB_TotSec32
    ///
    /// The total number of sectors for FAT32
    /// For FAT16 use, see total_sectors_16
    pub total_sectors_32: [u8; 4],
}

// Safety: RawBpb is a C-repr struct with no padding issues for its fields
unsafe impl bytemuck::NoUninit for RawBpb {}
unsafe impl bytemuck::Zeroable for RawBpb {}
unsafe impl bytemuck::AnyBitPattern for RawBpb {}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
/// FAT12/16 extended BIOS parameter block and boot-sector payload.
pub struct RawBpbExt16 {
    /// BS_DrvNum
    pub drive_number: u8,
    /// BS_Reserved1
    pub reserved1: u8,
    /// BS_BootSig
    ///
    /// The extended boot signature, should be 0x29
    pub ext_boot_signature: u8,
    /// BS_VolID
    ///
    /// Volumme Serial Number
    /// This ID should be unique for each volume
    pub volume_id: [u8; 4],
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: [u8; 11],
    /// BS_FilSysType
    ///
    /// Must be set to the strings "FAT12   ","FAT16   ", or "FAT     "
    pub fs_type: [u8; 8],
    /// Zeros
    /// To make it compatible with bytemuck, instead of using [u8; 448], we use 256 + 128 + 64
    pub padding1: [u8; 448],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: [u8; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
/// FAT32 extended BIOS parameter block and boot-sector payload.
pub struct RawBpbExt32 {
    /// BPB_FatSz32
    ///
    /// The number of sectors per fat
    /// BPB_FATSz16 must be 0
    pub sectors_per_fat_32: U32<LittleEndian>,
    /// BPB_ExtFlags
    ///
    /// See the BpbExt32Flags struct for more information
    pub ext_flags: [u8; 2],
    /// BPB_FSVer
    ///
    /// The version of the file system
    /// This must be set to 0x00
    pub version: [u8; 2],
    /// BPB_RootClus
    ///
    /// The cluster number of the root directory
    /// This should be 2, or the first usable (not bad) cluster usable
    pub root_cluster: U32<LittleEndian>,
    /// BPB_FSInfo
    ///
    /// The sector number of the FSINFO structure
    /// NOTE: There is a copy of the FSINFO structure in the
    /// sequence of backup boot sectors, but only the copy
    /// pointed to by this field is kept up to date (i.e., both the
    /// primary and backup boot record point to the same
    /// FSINFO sector)
    pub fs_info_sector: U16<LittleEndian>,
    /// BPB_BkBootSec
    ///
    /// The sector number of the backup boot sector
    /// If set to 6 (only valid non-zero value), the boot sector
    /// in the reserved area is used to store the backup boot sector
    pub boot_sector: [u8; 2],
    /// BPB_Reserved
    /// Reserved, should be zero
    pub reserved: [u8; 12],
    /// BS_DrvNum
    ///
    /// The BIOS interrupt 0x13 drive number
    /// Should be 0x80 or 0x00
    pub drive_number: u8,
    /// BS_Reserved1
    /// Reserved, should be zero
    pub reserved1: u8,
    /// BS_BootSig
    ///
    /// The extended boot signature, should be 0x29
    pub ext_boot_signature: u8,
    /// BS_VolID
    ///
    /// Volumme Serial Number
    /// This ID should be unique for each volume
    pub volume_id: [u8; 4],
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: [u8; 11],
    /// BS_FilSysType
    ///
    /// Must be set to the string "FAT32   "
    pub fs_type: [u8; 8],
    /// Zeros
    pub padding1: [u8; 420],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: U16<LittleEndian>,
}

// Safety: RawBpbExt32 is a C-repr struct with no padding issues
unsafe impl bytemuck::NoUninit for RawBpbExt32 {}
unsafe impl bytemuck::Zeroable for RawBpbExt32 {}
unsafe impl bytemuck::AnyBitPattern for RawBpbExt32 {}

/// BPB_ExtFlags
///
/// This is a union of the flags that are set in the BPB_ExtFlags field
/// The flags are the following:
/// bits 0-3: zero based index of the active FAT, mirroring must be disabled
/// bits 4-6: reserved
/// bit 7: FAT mirroring is enabled
/// bits 8-15: reserved
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BpbExt32Flags(u16);

/// Raw FAT32 FSInfo sector.
///
/// @hadris-spec FAT:FSInfo
/// @hadris-compliance full
/// @hadris-tests boot::tests::fs_info_signatures, fatfs_read::fsinfo_unknown_values_mount_and_count_by_scanning
/// @hadris-fuzz fat_read
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
    pub free_count: U32<LittleEndian>,
    /// FSI_Nxt_Free
    ///
    /// The next free cluster number, this have to be bigger than 2, and less than or equal to the
    pub next_free: U32<LittleEndian>,
    /// FSI_Reserved2
    pub reserved2: [u8; 12],
    /// FSI_TrailSig
    ///
    /// The trail signature, this have to be 0xAA550000
    pub trail_signature: U32<LittleEndian>,
}

// Safety: RawFsInfo is a C-repr packed struct
unsafe impl bytemuck::NoUninit for RawFsInfo {}
unsafe impl bytemuck::Zeroable for RawFsInfo {}
unsafe impl bytemuck::AnyBitPattern for RawFsInfo {}
