use crate::FatType;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawBpb {
    /// BS_jmpBoot
    pub jump: [u8; 3],
    /// BS_OEMName
    /// The name of the program that formatted the partition
    pub oem_name: [u8; 8],
    /// BPB_BytsPerSec
    /// The number of bytes per sector
    pub bytes_per_sector: [u8; 2],
    /// BPB_SecPerClus
    /// The number of sectors per cluster
    pub sectors_per_cluster: u8,
    /// BPB_RsvdSecCnt
    ///
    /// The number of reserved sectors, should be nonzero, ans should be a multiple of the sectors per cluster
    /// This is used to:
    /// 1. align the start of the filesystem to the sectors per cluster
    /// 2. Move the data (cluster 2) to the end of fat tables, so that the data can be read from the start of the filesystem
    pub reserved_sector_count: [u8; 2],
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

#[repr(C, packed)]
#[derive(Clone, Copy)]
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
    pub padding1: [u8; 448],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: [u8; 2],
}

/// BPB
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawBpbExt32 {
    /// BPB_FatSz32
    ///
    /// The number of sectors per fat
    /// BPB_FATSz16 must be 0
    pub sectors_per_fat_32: [u8; 4],
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
    pub root_cluster: [u8; 4],
    /// BPB_FSInfo
    ///
    /// The sector number of the FSINFO structure
    /// NOTE: There is a copy of the FSINFO structure in the
    /// sequence of backup boot sectors, but only the copy
    /// pointed to by this field is kept up to date (i.e., both the
    /// primary and backup boot record point to the same
    /// FSINFO sector)
    pub fs_info_sector: [u8; 2],
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
    pub signature_word: [u8; 2],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub union RawBpbExt {
    pub bpb16: RawBpbExt16,
    pub bpb32: RawBpbExt32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawBootSector {
    pub bpb: RawBpb,
    pub bpb_ext: RawBpbExt,
}

impl RawBpb {
    pub fn check_jump_boot(&self) -> bool {
        (self.jump[0] == 0xEB && self.jump[2] == 0x90) || self.jump[0] == 0xE9
    }

    pub fn check_bytes_per_sector(&self) -> bool {
        let bytes_per_sector = u16::from_le_bytes(self.bytes_per_sector);
        matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
    }

    pub fn check_sectors_per_cluster(&self) -> bool {
        matches!(self.sectors_per_cluster, 1 | 2 | 4 | 8 | 16 | 32 | 64 | 128)
    }

    pub fn check_reserved_sector_count(&self) -> bool {
        // TODO: Maybe ensure it is aligned with data segment?
        u16::from_le_bytes(self.reserved_sector_count) != 0
    }

    pub fn check_fat_count(&self) -> bool {
        matches!(self.fat_count, 1 | 2)
    }
}

#[cfg(any(feature = "read", feature = "write"))]
impl RawBootSector {
    pub fn get_type(&self) -> FatType {
        let root_entry_count = u16::from_le_bytes(self.bpb.root_entry_count);
        let bytes_per_sector = u16::from_le_bytes(self.bpb.bytes_per_sector);
        let sectors_per_fat_16 = u16::from_le_bytes(self.bpb.sectors_per_fat_16);
        let total_sectors_16 = u16::from_le_bytes(self.bpb.total_sectors_16);

        // Based on FAT32 spec
        let root_dir_sectors = ((root_entry_count * 32) + bytes_per_sector) / bytes_per_sector;
        if root_dir_sectors == 0 || sectors_per_fat_16 == 0 {
            return FatType::Fat32;
        }

        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16 as u32
        } else {
            u32::from_le_bytes(self.bpb.total_sectors_32)
        };

        let data_sectors = total_sectors
            - (u16::from_le_bytes(self.bpb.reserved_sector_count) as u32
                + (self.bpb.fat_count as u32 * sectors_per_fat_16 as u32)
                + root_entry_count as u32);

        match data_sectors {
            0..4085 => FatType::Fat12,
            4085..65525 => FatType::Fat16,
            65525.. => panic!("Fat16 partition exceeds maximum size"),
        }
    }

}

impl RawBootSector {
    pub fn from_bytes(bytes: &[u8; 512]) -> &RawBootSector {
        bytemuck::cast_ref(bytes)
    }

    pub fn from_bytes_mut(bytes: &mut [u8;512]) -> &RawBootSector {
        bytemuck::cast_mut(bytes)
    }
}

// TODO: Bytemuck should be an optional feature
unsafe impl bytemuck::Zeroable for RawBootSector {}
unsafe impl bytemuck::NoUninit for RawBootSector {}
unsafe impl bytemuck::AnyBitPattern for RawBootSector {}

/// Static assertions are placed in tests to that it doesn't need to be compiled when not needed
#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};
    use static_assertions::const_assert_eq;

    const_assert_eq!(size_of::<RawBpb>(), 36);
    const_assert_eq!(size_of::<RawBpbExt16>(), 512 - 36);
    const_assert_eq!(size_of::<RawBpbExt32>(), 512 - 36);
    const_assert_eq!(size_of::<RawBootSector>(), 512);

    const_assert_eq!(align_of::<RawBpb>(), 1);
    const_assert_eq!(align_of::<RawBpbExt16>(), 1);
    const_assert_eq!(align_of::<RawBpbExt32>(), 1);
    // TODO: Maybe we can align this to 512 bytes?
    const_assert_eq!(align_of::<RawBootSector>(), 1);

    // Here we test for the alignment and the size for each of the fields according to the FAT spec
    const_assert_eq!(offset_of!(RawBpb, jump), 0);
    const_assert_eq!(offset_of!(RawBpb, oem_name), 3);
    const_assert_eq!(offset_of!(RawBpb, bytes_per_sector), 11);
    const_assert_eq!(offset_of!(RawBpb, sectors_per_cluster), 13);
    const_assert_eq!(offset_of!(RawBpb, reserved_sector_count), 14);
    const_assert_eq!(offset_of!(RawBpb, fat_count), 16);
    const_assert_eq!(offset_of!(RawBpb, root_entry_count), 17);
    const_assert_eq!(offset_of!(RawBpb, total_sectors_16), 19);
    const_assert_eq!(offset_of!(RawBpb, media_type), 21);
    const_assert_eq!(offset_of!(RawBpb, sectors_per_fat_16), 22);
    const_assert_eq!(offset_of!(RawBpb, sectors_per_track), 24);
    const_assert_eq!(offset_of!(RawBpb, num_heads), 26);
    const_assert_eq!(offset_of!(RawBpb, hidden_sector_count), 28);
    const_assert_eq!(offset_of!(RawBpb, total_sectors_32), 32);

    const_assert_eq!(offset_of!(RawBpbExt16, drive_number), 36 - 36);
    const_assert_eq!(offset_of!(RawBpbExt16, reserved1), 37 - 36);
    const_assert_eq!(offset_of!(RawBpbExt16, ext_boot_signature), 38 - 36);
    const_assert_eq!(offset_of!(RawBpbExt16, volume_id), 39 - 36);
    const_assert_eq!(offset_of!(RawBpbExt16, volume_label), 43 - 36);
    const_assert_eq!(offset_of!(RawBpbExt16, fs_type), 54 - 36);
    const_assert_eq!(offset_of!(RawBpbExt16, padding1), 62 - 36);
    const_assert_eq!(offset_of!(RawBpbExt16, signature_word), 510 - 36);

    const_assert_eq!(offset_of!(RawBpbExt32, sectors_per_fat_32), 36 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, ext_flags), 40 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, version), 42 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, root_cluster), 44 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, fs_info_sector), 48 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, boot_sector), 50 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, reserved), 52 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, drive_number), 64 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, reserved1), 65 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, ext_boot_signature), 66 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, volume_id), 67 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, volume_label), 71 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, fs_type), 82 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, padding1), 90 - 36);
    const_assert_eq!(offset_of!(RawBpbExt32, signature_word), 510 - 36);

    const_assert_eq!(offset_of!(RawBootSector, bpb), 0);
    const_assert_eq!(offset_of!(RawBootSector, bpb_ext), 36);
}
