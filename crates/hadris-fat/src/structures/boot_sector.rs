use crate::FatType;

use super::FatStr;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// 3.5" Floppy 1.44MB
    HighDensityFloppy = 0xF0,
    /// Hard Disk
    HardDisk = 0xF8,
    /// 3.5" Double-Density Floppy 720kB
    DoubleDensityFloppy = 0xF9,
    Reserved1 = 0xFA,
    Reserved2 = 0xFB,
    Reserved3 = 0xFC,
    Reserved4 = 0xFD,
    Reserved5 = 0xFE,
    Reserved6 = 0xFF,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BootSectorCommon {
    /// BS_jmpBoot
    pub jump: [u8; 3],
    /// BS_OEMName
    /// The name of the program that formatted the partition
    pub oem_name: FatStr<8>,
    /// BPB_BytsPerSec
    /// The number of bytes per sector
    pub bytes_per_sector: u16,
    /// BPB_SecPerClus
    /// The number of sectors per cluster
    pub sectors_per_cluster: u8,
    /// BPB_RsvdSecCnt
    ///
    /// The number of reserved sectors, should be nonzero, ans should be a multiple of the sectors per cluster
    /// This is used to:
    /// 1. align the start of the filesystem to the sectors per cluster
    /// 2. Move the data (cluster 2) to the end of fat tables, so that the data can be read from the start of the filesystem
    pub reserved_sector_count: u16,
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
    pub root_entry_count: u16,
    /// BPB_TotSec16
    ///
    /// The number of sectors
    /// For FAT32, this should be 0
    /// For FAT16, if the number of sectors is greater than 0x10000, you should use total_sectors_32
    pub total_sectors_16: u16,
    /// BPB_Media
    ///
    /// See the MediaType enum for more information
    pub media_type: MediaType,
    /// BPB_FATSz16
    ///
    /// The number of sectors per fat
    /// For FAT32, this should be 0
    pub sectors_per_fat_16: u16,
    /// BPB_SecPerTrk
    ///
    /// The number of sectors per track
    /// This is only relevant for media with have a geometry and used by BIOS interrupt 0x13
    pub sectors_per_track: u16,
    /// BPB_NumHeads
    ///
    /// Similar situation as sectors_per_track
    pub num_heads: u16,
    /// BPB_HiddSec
    ///
    /// The number of hidden sectors predicing the partition that contains the FAT volume.
    /// This must be 0 on media that isn't partitioned
    pub hidden_sector_count: u32,
    /// BPB_TotSec32
    ///
    /// The total number of sectors for FAT32
    /// For FAT16 use, see total_sectors_16
    pub total_sectors_32: u32,
}

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
pub struct BpbExtended16 {
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
    pub volume_id: u32,
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: FatStr<11>,
    /// BS_FilSysType
    ///
    /// Must be set to the strings "FAT12   ","FAT16   ", or "FAT     "
    pub fs_type: FatStr<8>,
    /// Zeros
    pub padding1: [u8; 448],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: u16,
}

/// BPB
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BpbExtended32 {
    /// BPB_FatSz32
    ///
    /// The number of sectors per fat
    /// BPB_FATSz16 must be 0
    pub sectors_per_fat_32: u32,
    /// BPB_ExtFlags
    ///
    /// See the BpbExt32Flags struct for more information
    pub ext_flags: BpbExt32Flags,
    /// BPB_FSVer
    ///
    /// The version of the file system
    /// This must be set to 0x00
    pub version: u16,
    /// BPB_RootClus
    ///
    /// The cluster number of the root directory
    /// This should be 2, or the first usable (not bad) cluster usable
    pub root_cluster: u32,
    /// BPB_FSInfo
    ///
    /// The sector number of the FSINFO structure
    /// NOTE: There is a copy of the FSINFO structure in the
    /// sequence of backup boot sectors, but only the copy
    /// pointed to by this field is kept up to date (i.e., both the
    /// primary and backup boot record point to the same
    /// FSINFO sector)
    pub fs_info_sector: u16,
    /// BPB_BkBootSec
    ///
    /// The sector number of the backup boot sector
    /// If set to 6 (only valid non-zero value), the boot sector
    /// in the reserved area is used to store the backup boot sector
    pub boot_sector: u16,
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
    pub volume_id: u32,
    /// BS_VolLab
    ///
    /// Volume label
    /// This should be "NO NAME    " if the volume is not labeled
    pub volume_label: FatStr<11>,
    /// BS_FilSysType
    ///
    /// Must be set to the string "FAT32   "
    pub fs_type: FatStr<8>,
    /// Zeros
    pub padding1: [u8; 420],
    /// Signature_word
    ///
    /// The signature word, should be 0xAA55
    pub signature_word: u16,
}

/// A union of the boot sector extended fields
/// This is used to read the boot sector into a generic boot sector struct
#[repr(packed)]
#[allow(dead_code)]
#[derive(Clone, Copy, bytemuck::AnyBitPattern)]
union BootSectorExtended {
    fat16: BpbExtended16,
    fat32: BpbExtended32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct BootSector {
    common: BootSectorCommon,
    extended: BootSectorExtended,
}

#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct BootSector32 {
    pub common: BootSectorCommon,
    pub extended: BpbExtended32,
}

impl TryFrom<u8> for MediaType {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0xF0 => Ok(Self::HighDensityFloppy),
            0xF8 => Ok(Self::HardDisk),
            0xF9 => Ok(Self::DoubleDensityFloppy),
            0xFA => Ok(Self::Reserved1),
            0xFB => Ok(Self::Reserved2),
            0xFC => Ok(Self::Reserved3),
            0xFD => Ok(Self::Reserved4),
            0xFE => Ok(Self::Reserved5),
            0xFF => Ok(Self::Reserved6),
            _ => Err(()),
        }
    }
}

impl core::fmt::Debug for BootSectorCommon {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let oem_name = self.oem_name;
        let bytes_per_sector = self.bytes_per_sector;
        let sectors_per_cluster = self.sectors_per_cluster;
        let reserved_sector_count = self.reserved_sector_count;
        let fat_count = self.fat_count;
        let root_entry_count = self.root_entry_count;
        let total_sectors_16 = self.total_sectors_16;
        let media_type = self.media_type;
        let sectors_per_fat_16 = self.sectors_per_fat_16;
        let sectors_per_track = self.sectors_per_track;
        let num_heads = self.num_heads;
        let hidden_sector_count = self.hidden_sector_count;
        let total_sectors_32 = self.total_sectors_32;

        f.debug_struct("BootSectorCommon")
            .field("oem_name", &oem_name)
            .field("bytes_per_sector", &bytes_per_sector)
            .field("sectors_per_cluster", &sectors_per_cluster)
            .field("reserved_sector_count", &reserved_sector_count)
            .field("fat_count", &fat_count)
            .field("root_entry_count", &root_entry_count)
            .field("total_sectors_16", &total_sectors_16)
            .field("media_type", &media_type)
            .field("sectors_per_fat_16", &sectors_per_fat_16)
            .field("sectors_per_track", &sectors_per_track)
            .field("num_heads", &num_heads)
            .field("hidden_sector_count", &hidden_sector_count)
            .field("total_sectors_32", &total_sectors_32)
            .finish()
    }
}

impl BootSectorCommon {
    pub fn new(
        bytes_per_sector: u16,
        sectors_per_cluster: u8,
        reserved_sector_count: u16,
        fat_count: u8,
        root_entry_count: u16,
        total_sectors_16: u16,
        media_type: MediaType,
        sectors_per_fat_16: u16,
        sectors_per_track: u16,
        num_heads: u16,
        hidden_sector_count: u32,
        total_sectors_32: u32,
    ) -> Self {
        // ==========================
        // Assertions for the FAT specification
        // ==========================
        debug_assert!(
            [512, 1024, 2048, 4096].contains(&bytes_per_sector),
            "bytes per sector can only be 512, 1024, 2048, or 4096"
        );
        debug_assert!(
            [1, 2, 4, 8, 16, 32, 64, 128].contains(&sectors_per_cluster),
            "sectors per cluster can only be 1, 2, 4, 8, 16, 32, 64, or 128"
        );
        debug_assert_ne!(
            reserved_sector_count, 0,
            "reserved sector count cannot be 0"
        );
        // We make this a hard error as well
        debug_assert_eq!(
            reserved_sector_count % (sectors_per_cluster as u16),
            0,
            "reserved sector count should be aligned to sectors per cluster"
        );
        // Number of fats can only be 1, or 2
        debug_assert!(
            [1, 2].contains(&fat_count),
            "a fat partition must have 1 or 2 fat tables"
        );
        debug_assert!(
            (root_entry_count as u32) * 32 % (bytes_per_sector as u32) == 0,
            "root entry count * 32 must be a multiple of bytes per sector"
        );
        // One of the total sector couut must be nonzero
        debug_assert!(
            (total_sectors_16 == 0 && total_sectors_32 != 0)
                || (total_sectors_16 != 0 && total_sectors_32 == 0),
            "total sectors must only be set on either total sectors 16 or total sectors 32, got {} and {}",
            total_sectors_16,
            total_sectors_32
        );

        Self {
            // FIXME: Actually calculate the jump offset
            jump: [0xEB, 0x00, 0x90],
            oem_name: FatStr::from_slice_unchecked(b"HADRISRS"),
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sector_count,
            fat_count,
            root_entry_count,
            total_sectors_16,
            media_type,
            sectors_per_fat_16,
            sectors_per_track,
            num_heads,
            hidden_sector_count,
            total_sectors_32,
        }
    }
}

impl core::fmt::Debug for BpbExtended32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let sectors_per_fat_32 = self.sectors_per_fat_32;
        let ext_flags = self.ext_flags;
        let version = self.version;
        let root_cluster = self.root_cluster;
        let fs_info_sector = self.fs_info_sector;
        let boot_sector = self.boot_sector;
        let drive_number = self.drive_number;
        let ext_boot_signature = self.ext_boot_signature;
        let volume_id = self.volume_id;
        let volume_label = self.volume_label;
        let fs_type = self.fs_type;
        f.debug_struct("BpbExtended32")
            .field("sectors_per_fat_32", &sectors_per_fat_32)
            .field("ext_flags", &ext_flags)
            .field("version", &version)
            .field("root_cluster", &root_cluster)
            .field("fs_info_sector", &fs_info_sector)
            .field("boot_sector", &boot_sector)
            .field("drive_number", &drive_number)
            .field("ext_boot_signature", &ext_boot_signature)
            .field("volume_id", &volume_id)
            .field("volume_label", &volume_label)
            .field("fs_type", &fs_type)
            .finish()
    }
}

impl BpbExtended32 {
    pub fn new(
        sectors_per_fat_32: u32,
        ext_flags: BpbExt32Flags,
        version: u16,
        root_cluster: u32,
        fs_info_sector: u16,
        boot_sector: u16,
        drive_number: u8,
        volume_id: u32,
        volume_label: FatStr<11>,
    ) -> Self {
        Self {
            sectors_per_fat_32,
            ext_flags,
            version,
            root_cluster,
            fs_info_sector,
            boot_sector,
            drive_number,
            ext_boot_signature: 0x29,
            volume_id,
            volume_label,
            fs_type: FatStr::from_slice_unchecked(b"FAT32   "),
            signature_word: 0xAA55,

            reserved: [0; 12],
            reserved1: 0,
            padding1: [0; 420],
        }
    }

    #[cfg(feature = "write")]
    pub fn current_volume_id(seed: u32) -> u32 {
        // We get the current time in seconds since the epoch
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let time_part = (now.as_secs() as u32) ^ (now.as_secs().wrapping_shr(32) as u32);
        // We make it seem 'random' by xoring it with the seed
        time_part ^ seed
    }

    /// # Safety
    ///
    /// This function is unsafe because it does not check if the bytes are actually a valid, and if
    /// it is the same endianness as the current system
    ///
    /// This function is only safe to use if you are sure that the bytes are a valid FAT32 boot,
    /// and the bytes passed in are the bytes 0x24 to the end of the boot sector
    pub unsafe fn read<'a>(bytes: &'a [u8]) -> &'a Self {
        unsafe { &*(bytes as *const [u8] as *const Self) }
    }

    pub fn write(&self, bytes: &mut [u8]) {
        bytes[0x24..0x28].copy_from_slice(&self.sectors_per_fat_32.to_le_bytes());
        bytes[0x28..0x2A].copy_from_slice(&self.ext_flags.0.to_le_bytes());
        bytes[0x2A..0x2C].copy_from_slice(&self.version.to_le_bytes());
        bytes[0x2C..0x30].copy_from_slice(&self.root_cluster.to_le_bytes());
        bytes[0x30..0x32].copy_from_slice(&self.fs_info_sector.to_le_bytes());
        bytes[0x32..0x34].copy_from_slice(&self.boot_sector.to_le_bytes());
        // Reserved
        bytes[0x34..0x40].fill(0x00);
        bytes[0x40..0x41].copy_from_slice(&self.drive_number.to_le_bytes());
        // Reserved
        bytes[0x41..0x42].fill(0x00);
        bytes[0x42..0x43].copy_from_slice(&self.ext_boot_signature.to_le_bytes());
        bytes[0x43..0x47].copy_from_slice(&self.volume_id.to_le_bytes());
        bytes[0x47..0x52].copy_from_slice(self.volume_label.as_slice());
        bytes[0x52..0x5A].copy_from_slice(self.fs_type.as_slice());
        // 90 - 420 are zero
        // Signature_word
        bytes[0x1FE..0x200].copy_from_slice(&[0x55, 0xAA]);
    }
}

impl BootSectorExtended {
    pub fn with_fat32(boot_sector: BpbExtended32) -> Self {
        Self { fat32: boot_sector }
    }
}

impl BootSector32 {
    pub fn bytes_per_sector(&self) -> u16 {
        self.common.bytes_per_sector
    }

    pub fn fs_info_sector(&self) -> u16 {
        self.extended.fs_info_sector
    }

    pub fn root_sector(&self) -> u32 {
        self.extended.root_cluster
    }

    pub fn reserved_sector_count(&self) -> u16 {
        self.common.reserved_sector_count
    }

    pub fn sectors_per_fat(&self) -> u32 {
        self.extended.sectors_per_fat_32
    }

    pub fn sectors_per_cluster(&self) -> u8 {
        self.common.sectors_per_cluster
    }

    pub fn total_sectors(&self) -> u32 {
        self.common.total_sectors_32
    }
}

impl BootSector {
    /// Create a new FAT32 boot sector
    pub fn create_fat32(
        bytes_per_sector: u16,
        sectors_per_cluster: u8,
        reserved_sector_count: u16,
        fat_count: u8,
        media_type: MediaType,
        hidden_sector_count: u32,
        total_sectors_32: u32,
        sectors_per_fat_32: u32,
        root_cluster: u32,
        fs_info_sector: u16,
        boot_sector: u16,
        drive_number: u8,
        volume_id: u32,
        volume_label: Option<&str>,
    ) -> Self {
        assert!(
            volume_label.is_none() || !volume_label.as_ref().unwrap().is_empty(),
            "Volume label provided, but is empty string"
        );
        assert!(
            reserved_sector_count % sectors_per_cluster as u16 == 0,
            "Reserved sector count must be a multiple of sectors per cluster"
        );

        // TODO: Add calculations for EXT flags
        Self::create_fat32_ext(
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sector_count,
            fat_count,
            0,
            0,
            media_type,
            0,
            63,
            255,
            hidden_sector_count,
            total_sectors_32,
            sectors_per_fat_32,
            BpbExt32Flags(0),
            root_cluster,
            fs_info_sector,
            boot_sector,
            drive_number,
            volume_id,
            volume_label,
        )
    }
    /// Create a new FAT12 boot sector, with extended parameters
    /// To use a more simplified interface, see the create_fat32 function
    pub fn create_fat32_ext(
        bytes_per_sector: u16,
        sectors_per_cluster: u8,
        reserved_sector_count: u16,
        fat_count: u8,
        root_entry_count: u16,
        total_sectors_16: u16,
        media_type: MediaType,
        sectors_per_fat_16: u16,
        sectors_per_track: u16,
        num_heads: u16,
        hidden_sector_count: u32,
        total_sectors_32: u32,
        sectors_per_fat_32: u32,
        ext_flags: BpbExt32Flags,
        root_cluster: u32,
        fs_info_sector: u16,
        boot_sector: u16,
        drive_number: u8,
        volume_id: u32,
        volume_label: Option<&str>,
    ) -> Self {
        const VERSION: u16 = 0x00;
        let volume_label = volume_label.map_or(
            FatStr::from_slice_unchecked(b"NO NAME    "),
            FatStr::new_truncate,
        );
        Self {
            common: BootSectorCommon::new(
                bytes_per_sector,
                sectors_per_cluster,
                reserved_sector_count,
                fat_count,
                root_entry_count,
                total_sectors_16,
                media_type,
                sectors_per_fat_16,
                sectors_per_track,
                num_heads,
                hidden_sector_count,
                total_sectors_32,
            ),
            extended: BootSectorExtended::with_fat32(BpbExtended32::new(
                sectors_per_fat_32,
                ext_flags,
                VERSION,
                root_cluster,
                fs_info_sector,
                boot_sector,
                drive_number,
                volume_id,
                volume_label,
            )),
        }
    }

    /// # Safety
    ///
    /// This function is unsafe because it does not check if the bytes are actually a valid, and if
    /// it is the same endianness as the current system
    pub unsafe fn from_bytes_unchecked<'a>(bytes: &'a [u8]) -> &'a Self {
        bytemuck::from_bytes(bytes)
    }

    /// # Safety
    ///
    /// This function is unsafe, because it does not check if it is actually FAT32, instead of
    /// FAT12 or FAT16
    pub unsafe fn as_fat32_unchecked<'a>(&'a self) -> &'a BootSector32 {
        bytemuck::cast_ref(self)
    }

    pub fn try_as_fat32<'a>(&'a self) -> Result<&'a BootSector32, ()> {
        if self.get_type() == FatType::Fat32 {
            Ok(unsafe { self.as_fat32_unchecked() })
        } else {
            Err(())
        }
    }

    pub fn as_fat32<'a>(&'a self) -> &'a BootSector32 {
        assert!(self.get_type() == FatType::Fat32);
        unsafe { self.as_fat32_unchecked() }
    }

    pub fn write(&self, bytes: &mut [u8]) {
        assert!(bytes.len() == 512, "bytes must be at least 512 bytes");
        bytes.copy_from_slice(bytemuck::bytes_of(self));
    }

    pub fn get_type(&self) -> FatType {
        // Based on FAT32 spec
        let root_dir_sectors = ((self.common.root_entry_count * 32) + self.common.bytes_per_sector)
            / self.common.bytes_per_sector;
        if root_dir_sectors == 0 || self.common.sectors_per_fat_16 == 0 {
            return FatType::Fat32;
        }

        let total_sectors = if self.common.total_sectors_16 != 0 {
            self.common.total_sectors_16 as u32
        } else {
            self.common.total_sectors_32
        };

        let data_sectors = total_sectors
            - (self.common.reserved_sector_count as u32
                + (self.common.fat_count as u32 * self.common.sectors_per_fat_16 as u32)
                + self.common.root_entry_count as u32);

        match data_sectors {
            0..4085 => FatType::Fat12,
            4085..65525 => FatType::Fat16,
            65525.. => panic!("Fat16 partition exceeds maximum size"),
        }
    }

    pub fn bytes_per_sector(&self) -> u16 {
        self.common.bytes_per_sector
    }
}

unsafe impl bytemuck::Zeroable for BootSector {}
unsafe impl bytemuck::NoUninit for BootSector {}
unsafe impl bytemuck::AnyBitPattern for BootSector {}

unsafe impl bytemuck::Zeroable for BootSector32 {}
unsafe impl bytemuck::NoUninit for BootSector32 {}
unsafe impl bytemuck::AnyBitPattern for BootSector32 {}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_media_type_parsing() {
        assert_eq!(MediaType::try_from(0xF0), Ok(MediaType::HighDensityFloppy));
        assert_eq!(MediaType::try_from(0xF8), Ok(MediaType::HardDisk));
        assert_eq!(
            MediaType::try_from(0xF9),
            Ok(MediaType::DoubleDensityFloppy)
        );
        assert_eq!(MediaType::try_from(0xFA), Ok(MediaType::Reserved1));
        assert_eq!(MediaType::try_from(0xFB), Ok(MediaType::Reserved2));
        assert_eq!(MediaType::try_from(0xFC), Ok(MediaType::Reserved3));
        assert_eq!(MediaType::try_from(0xFD), Ok(MediaType::Reserved4));
        assert_eq!(MediaType::try_from(0xFE), Ok(MediaType::Reserved5));
        assert_eq!(MediaType::try_from(0xFF), Ok(MediaType::Reserved6));
        assert_eq!(MediaType::try_from(0x00), Err(())); // Invalid media type
    }

    #[test]
    fn test_boot_sector_size() {
        use core::mem::size_of;
        assert_eq!(size_of::<BootSector>(), 512);
        assert_eq!(size_of::<BootSector32>(), 512);
    }

    #[test]
    fn test_boot_sector_creation() {
        let boot_sector = BootSector::create_fat32(
            512,                 // bytes_per_sector
            1,                   // sectors_per_cluster
            32,                  // reserved_sector_count
            1,                   // fat_count
            MediaType::HardDisk, // media_type
            0,                   // hidden_sector_count
            65536,               // total_sectors_32
            504,                 // sectors_per_fat_32
            2,                   // root_cluster
            1,                   // fs_info_sector
            6,                   // boot_sector
            0x80,                // drive_number
            0xDEADBEEF,          // volume_id
            Some("TEST_LABEL"),
        );

        let bps = boot_sector.common.bytes_per_sector;
        assert_eq!(bps, 512);
        let spc = boot_sector.common.sectors_per_cluster;
        assert_eq!(spc, 1);
        let rs = boot_sector.common.reserved_sector_count;
        assert_eq!(rs, 32);
        let fc = boot_sector.common.fat_count;
        assert_eq!(fc, 1);
        let ts32 = boot_sector.common.total_sectors_32;
        assert_eq!(ts32, 65536);
        let mt = boot_sector.common.media_type;
        assert_eq!(mt, MediaType::HardDisk);
        let fs32 = unsafe { boot_sector.extended.fat32.sectors_per_fat_32 };
        assert_eq!(fs32, 504);
        let rc = unsafe { boot_sector.extended.fat32.root_cluster };
        assert_eq!(rc, 2);
        let fsis = unsafe { boot_sector.extended.fat32.fs_info_sector };
        assert_eq!(fsis, 1);
        let bs = unsafe { boot_sector.extended.fat32.boot_sector };
        assert_eq!(bs, 6);
        let dn = unsafe { boot_sector.extended.fat32.drive_number };
        assert_eq!(dn, 0x80);
        let vid = unsafe { boot_sector.extended.fat32.volume_id };
        assert_eq!(vid, 0xDEADBEEF);
        let vl = unsafe { boot_sector.extended.fat32.volume_label };
        assert_eq!(vl.as_str(), "TEST_LABEL ");
        let ft = unsafe { boot_sector.extended.fat32.fs_type };
        assert_eq!(ft.as_str(), "FAT32   ");
    }

    #[test]
    fn test_fat_type_detection() {
        // TODO: Add tests for FAT16 and FAT12
        let fat32_sector = BootSector::create_fat32_ext(
            512,
            1,
            1,
            1,
            0,
            0,
            MediaType::HardDisk,
            0,
            32,
            64,
            0,
            70000,
            504,
            BpbExt32Flags(0),
            2,
            1,
            6,
            0x80,
            0x12345678,
            None,
        );
        assert_eq!(fat32_sector.get_type(), FatType::Fat32);
    }

    #[test]
    fn test_boot_sector_serialization() {
        let boot_sector = BootSector::create_fat32(
            512,
            1,
            32,
            1,
            MediaType::HardDisk,
            0,
            65536,
            504,
            2,
            1,
            6,
            0x80,
            0xDEADBEEF,
            None,
        );

        let mut buffer = [0u8; 512];
        boot_sector.write(&mut buffer);

        let deserialized: &BootSector = unsafe { BootSector::from_bytes_unchecked(&buffer) };
        let bps = deserialized.common.bytes_per_sector;
        assert_eq!(bps, 512);
        let spc = deserialized.common.sectors_per_cluster;
        assert_eq!(spc, 1);
        let rsc = deserialized.common.reserved_sector_count;
        assert_eq!(rsc, 32);
        let spf = unsafe { deserialized.extended.fat32.sectors_per_fat_32 };
        assert_eq!(spf, 504);
    }

    #[test]
    fn test_boot_sector_read() {
        let boot_sector = BootSector::create_fat32(
            512,
            1,
            32,
            1,
            MediaType::HardDisk,
            0,
            65536,
            504,
            2,
            1,
            6,
            0x80,
            0xDEADBEEF,
            None,
        );

        let mut buffer = [0u8; 512];
        boot_sector.write(&mut buffer);

        let parsed: &BootSector = unsafe { BootSector::from_bytes_unchecked(&buffer) };

        let bps = parsed.common.bytes_per_sector;
        assert_eq!(bps, 512);
        let spf = unsafe { parsed.extended.fat32.sectors_per_fat_32 };
        assert_eq!(spf, 504);
        let dn = unsafe { parsed.extended.fat32.drive_number };
        assert_eq!(dn, 0x80);
        let fs = unsafe { parsed.extended.fat32.fs_type };
        assert_eq!(fs.as_str(), "FAT32   ");
    }
}

// TODO: Tests which are run without std
