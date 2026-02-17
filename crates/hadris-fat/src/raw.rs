use hadris_common::types::{
    endian::LittleEndian,
    number::{U16, U32},
};

/// The RawBpb struct represents the boot sector of any FAT partition
///
/// This only contains the common fields of the boot sector, and is not meant to be used directly
/// for reading or writing to the boot sector, for that, see `RawBootSector`, which contains
/// the boot sector and the extended boot sector
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

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFileEntry {
    /// DIR_Name
    ///
    /// The name of the file, padded with spaces, and in the 8.3 format
    /// A value of 0xE5 indicates that the directory is free. For kanji, 0x05 is used instead of 0xE5
    /// The special value 0x00 also indicates that the directory is free, but also all the entries
    /// following it are free
    /// The name cannot start with a space
    /// Only upper case letters, digits, and the following characters are allowed:
    /// $ % ' - _ @ ~ ` ! ( ) { } ^ # &
    pub name: [u8; 11],
    /// DIR_Attr
    ///
    /// The file attributes
    pub attributes: u8,
    /// DIR_NTRes
    ///
    /// Reserved for use by Windows NT
    pub reserved: u8,
    /// DIR_CrtTimeTenth
    ///
    /// The creation time, in tenths of a second
    pub creation_time_tenth: u8,
    /// DIR_CrtTime
    ///
    /// The creation time, granularity is 2 seconds
    pub creation_time: [u8; 2],
    /// DIR_CrtDate
    ///
    /// The creation date
    pub creation_date: [u8; 2],
    /// DIR_LstAccDate
    ///
    /// The last access date
    pub last_access_date: [u8; 2],
    /// DIR_FstClusHI
    ///
    /// The high word of the first cluster number
    pub first_cluster_high: U16<LittleEndian>,
    /// DIR_WrtTime
    ///
    /// The last write time, granularity is 2 seconds
    pub last_write_time: [u8; 2],
    /// DIR_WrtDate
    ///
    /// The last write date
    pub last_write_date: [u8; 2],
    /// DIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: U16<LittleEndian>,
    /// DIR_FileSize
    ///
    /// The size of the file, in bytes
    pub size: U32<LittleEndian>,
}

unsafe impl bytemuck::NoUninit for RawFileEntry {}
unsafe impl bytemuck::Zeroable for RawFileEntry {}
unsafe impl bytemuck::AnyBitPattern for RawFileEntry {}

/// A long file name entry
/// The maximum length of a long file name is 255 characters, not including the null terminator
/// The characters allowed extend these characters:
///  . + , ; = [ ]
/// Embedded paces are also allowed
/// The name is stored in UTF-16 encoding (UNICODE)
/// When the unicode character cannot be translated to ANSI, an underscore is used
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawLfnEntry {
    /// LFN_Ord
    ///
    /// The order of the LFN entry, the contents must be masked with 0x40 for the last entry
    pub sequence_number: u8,
    /// LFN_Name1
    ///
    /// The first part of the long file name
    pub name1: [u8; 10],
    /// LDIR_Attr
    ///
    /// Attributes, must be set to: ATTR_LONG_NAME, which is:
    /// ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID
    pub attributes: u8,
    /// LFN_Type
    ///
    /// The type of the LFN entry, must be set to 0
    pub ty: u8,
    /// LFN_Chksum
    ///
    /// Checksum of name in the associated short name directory entry at the end of the LFN sequence
    /// THe algorithm described in the FAT spec is:
    /// unsigned char ChkSum (unsigned char \*pFcbName)
    /// {
    ///     short FcbNameLen;
    ///     unsigned char Sum;
    ///     Sum = 0;
    ///     for (FcbNameLen=11; FcbNameLen!=0; FcbNameLen--) {
    ///         // NOTE: The operation is an unsigned char rotate right
    ///         Sum = ((Sum & 1) ? 0x80 : 0) + (Sum >> 1) + *pFcbName++;
    ///     }
    ///     return (Sum);
    /// }
    pub checksum: u8,
    /// LFN_Name2
    ///
    /// The second part of the long file name
    pub name2: [u8; 12],
    /// LDIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: [u8; 2],
    /// LFN_Name3
    ///
    /// The third part of the long file name
    pub name3: [u8; 4],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub union RawDirectoryEntry {
    pub file: RawFileEntry,
    #[cfg(feature = "lfn")]
    pub lfn: RawLfnEntry,
    pub bytes: [u8; 32],
}

impl RawDirectoryEntry {
    pub fn attributes(&self) -> u8 {
        unsafe { self.file }.attributes
    }
}

// Bytemuck implementations for RawDirectoryEntry union
// These are needed for read_struct to work
unsafe impl bytemuck::NoUninit for RawDirectoryEntry {}
unsafe impl bytemuck::Zeroable for RawDirectoryEntry {}
unsafe impl bytemuck::AnyBitPattern for RawDirectoryEntry {}

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

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DirEntryAttrFlags: u8 {
        const READ_ONLY = 1 << 0;
        const HIDDEN = 1 << 1;
        const SYSTEM = 1 << 2;
        const VOLUME_ID = 1 << 3;
        const DIRECTORY = 1 << 4;
        const ARCHIVE = 1 << 5;
    }
}

impl DirEntryAttrFlags {
    pub const LONG_NAME: Self = Self::from_bits_truncate(
        Self::READ_ONLY.bits() | Self::HIDDEN.bits() | Self::SYSTEM.bits() | Self::VOLUME_ID.bits(),
    );
}
