use crate::{
    structures::raw::boot_sector::{RawBpb, RawBpbExt, RawBpbExt32},
    FatType,
};

use super::{raw::boot_sector::RawBootSector, FatStr};

/// BPB_Media
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

/// The info variant of the BootSector structure, which contains the info of the boot sector
/// in the current endianness. The alignment and size is not guaranteed, so converting between
/// raw and info structs requires the use of a conversion method instead of simply casting bytes
///
/// Fields which aren't relevant for FAT32 are not included,
/// for a raw and byte compatible representation, see the 'raw' module
#[derive(Debug, Clone, Copy)]
pub struct BootSectorInfoFat32 {
    pub oem_name: FatStr<8>,
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sector_count: u16,
    pub fat_count: u8,
    pub media_type: MediaType,
    pub hidden_sector_count: u32,
    pub total_sectors: u32,
    pub sectors_per_fat: u32,
    pub ext_flags: BpbExt32Flags,
    pub version: u16,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    pub boot_sector: u16,
    pub drive_number: u8,
    pub volume_id: u32,
    pub volume_label: FatStr<11>,
    pub fs_type: FatStr<8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootSectorConversionError {
    InvalidFatType(FatType),
    InvalidValue(&'static str),
}

impl core::fmt::Display for BootSectorConversionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFatType(ty) => write!(f, "Invalid FAT type: {:?}", ty),
            Self::InvalidValue(val) => write!(f, "Invalid value: {}", val),
        }
    }
}

impl TryFrom<&RawBootSector> for BootSectorInfoFat32 {
    type Error = BootSectorConversionError;

    fn try_from(value: &RawBootSector) -> Result<Self, Self::Error> {
        let (bpb, bpb_ext) = match value.get_type() {
            FatType::Fat32 => (value.bpb, unsafe { value.bpb_ext.bpb32 }),
            ty => return Err(Self::Error::InvalidFatType(ty)),
        };

        if !bpb.check_jump_boot() {
            return Err(Self::Error::InvalidValue("JumpBoot"));
        }

        if !bpb.check_bytes_per_sector() {
            return Err(Self::Error::InvalidValue("BytesPerSector"));
        }

        if !bpb.check_sectors_per_cluster() {
            return Err(Self::Error::InvalidValue("SectorsPerCluster"));
        }

        if !bpb.check_reserved_sector_count() {
            return Err(Self::Error::InvalidValue("ReservedSectorCount"));
        }

        if !bpb.check_fat_count() {
            return Err(Self::Error::InvalidValue("FatCount"));
        }

        if u16::from_le_bytes(bpb.root_entry_count) != 0 {
            return Err(Self::Error::InvalidValue("RootEntryCount"));
        }

        if u16::from_le_bytes(bpb.total_sectors_16) != 0 {
            return Err(Self::Error::InvalidValue("TotalSectors16"));
        }

        let media_type = MediaType::try_from(bpb.media_type)
            .map_err(|_| Self::Error::InvalidValue("MediaType"))?;

        if u32::from_le_bytes(bpb.total_sectors_32) == 0 {
            return Err(Self::Error::InvalidValue("TotalSectors32"));
        }

        if u16::from_le_bytes(bpb_ext.version) != 0 {
            return Err(Self::Error::InvalidValue("Version"));
        }

        if !matches!(u16::from_le_bytes(bpb_ext.boot_sector), 0 | 6) {
            return Err(Self::Error::InvalidValue("BootSector"));
        }

        if bpb_ext.ext_boot_signature != 0x29 {
            return Err(Self::Error::InvalidValue("ExtBootSignature"));
        }

        if bpb_ext.signature_word != 0xAA55u16.to_le_bytes() {
            return Err(Self::Error::InvalidValue("SignatureWord"));
        }

        Ok(BootSectorInfoFat32 {
            oem_name: FatStr::from_slice_unchecked(&bpb.oem_name),
            bytes_per_sector: u16::from_le_bytes(bpb.bytes_per_sector),
            sectors_per_cluster: bpb.sectors_per_cluster,
            reserved_sector_count: u16::from_le_bytes(bpb.reserved_sector_count),
            fat_count: bpb.fat_count,
            media_type,
            hidden_sector_count: u32::from_le_bytes(bpb.hidden_sector_count),
            total_sectors: u32::from_le_bytes(bpb.total_sectors_32),
            boot_sector: u16::from_le_bytes(bpb_ext.boot_sector),
            drive_number: bpb_ext.drive_number,
            ext_flags: BpbExt32Flags(u16::from_le_bytes(bpb_ext.ext_flags)),
            fs_info_sector: u16::from_le_bytes(bpb_ext.fs_info_sector),
            fs_type: FatStr::from_slice_unchecked(&bpb_ext.fs_type),
            root_cluster: u32::from_le_bytes(bpb_ext.root_cluster),
            sectors_per_fat: u32::from_le_bytes(bpb_ext.sectors_per_fat_32),
            version: u16::from_le_bytes(bpb_ext.version),
            volume_id: u32::from_le_bytes(bpb_ext.volume_id),
            volume_label: FatStr::from_slice_unchecked(&bpb_ext.volume_label),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BootSectorInfo {
    Fat32(BootSectorInfoFat32),
}

impl BootSectorInfo {
    #[inline]
    pub fn bytes_per_sector(&self) -> u16 {
        match self {
            BootSectorInfo::Fat32(info) => info.bytes_per_sector,
        }
    }

    #[inline]
    pub fn sectors_per_fat(&self) -> u32 {
        match self {
            BootSectorInfo::Fat32(info) => info.sectors_per_fat,
        }
    }

    #[inline]
    pub fn sectors_per_cluster(&self) -> u8 {
        match self {
            BootSectorInfo::Fat32(info) => info.sectors_per_cluster,
        }
    }

    #[inline]
    pub fn total_sectors(&self) -> u32 {
        match self {
            BootSectorInfo::Fat32(info) => info.total_sectors,
        }
    }

    #[inline]
    pub fn reserved_sector_count(&self) -> u16 {
        match self {
            BootSectorInfo::Fat32(info) => info.reserved_sector_count,
        }
    }

    #[inline]
    pub fn root_cluster(&self) -> u32 {
        match self {
            BootSectorInfo::Fat32(info) => info.root_cluster,
        }
    }

    #[inline]
    pub fn fs_info_sector(&self) -> u16 {
        match self {
            BootSectorInfo::Fat32(info) => info.fs_info_sector,
        }
    }
}

impl TryFrom<&RawBootSector> for BootSectorInfo {
    type Error = BootSectorConversionError;

    fn try_from(raw: &RawBootSector) -> Result<Self, Self::Error> {
        match raw.get_type() {
            FatType::Fat32 => Ok(BootSectorInfo::Fat32(BootSectorInfoFat32::try_from(raw)?)),
            _ => unimplemented!(),
        }
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, bytemuck::AnyBitPattern, bytemuck::NoUninit)]
pub struct BootSectorFat32 {
    data: RawBootSector,
}

impl BootSectorFat32 {
    pub fn from_bytes(bytes: &[u8; 512]) -> &Self {
        bytemuck::cast_ref(bytes)
    }

    pub fn from_bytes_mut(bytes: &mut [u8; 512]) -> &mut Self {
        bytemuck::cast_mut(bytes)
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub union BootSector {
    fat32: BootSectorFat32,
}

impl BootSector {
    pub fn from_bytes(bytes: &[u8; 512]) -> &Self {
        bytemuck::cast_ref(bytes)
    }

    pub fn from_bytes_mut(bytes: &mut [u8; 512]) -> &mut Self {
        bytemuck::cast_mut(bytes)
    }

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
        let fat32 = BootSectorFat32 {
            data: RawBootSector {
                bpb: RawBpb {
                    jump: [0xEB, 0x00, 0x90],
                    oem_name: *b"HADRISRS",
                    bytes_per_sector: bytes_per_sector.to_le_bytes(),
                    sectors_per_cluster,
                    reserved_sector_count: reserved_sector_count.to_le_bytes(),
                    fat_count,
                    root_entry_count: root_entry_count.to_le_bytes(),
                    total_sectors_16: total_sectors_16.to_le_bytes(),
                    media_type: media_type as u8,
                    sectors_per_fat_16: sectors_per_fat_16.to_le_bytes(),
                    sectors_per_track: sectors_per_track.to_le_bytes(),
                    num_heads: num_heads.to_le_bytes(),
                    hidden_sector_count: hidden_sector_count.to_le_bytes(),
                    total_sectors_32: total_sectors_32.to_le_bytes(),
                },
                bpb_ext: RawBpbExt {
                    bpb32: RawBpbExt32 {
                        sectors_per_fat_32: sectors_per_fat_32.to_le_bytes(),
                        ext_flags: ext_flags.0.to_le_bytes(),
                        version: VERSION.to_le_bytes(),
                        root_cluster: root_cluster.to_le_bytes(),
                        fs_info_sector: fs_info_sector.to_le_bytes(),
                        boot_sector: boot_sector.to_le_bytes(),
                        drive_number,
                        volume_id: volume_id.to_le_bytes(),
                        volume_label: volume_label.raw,
                        fs_type: *b"FAT32   ",

                        ext_boot_signature: 0x29,
                        padding1_1: [0u8; 256],
                        padding1_2: [0u8; 128],
                        padding1_3: [0u8; 32],
                        padding1_4: [0u8; 4],
                        reserved: [0u8; 12],
                        reserved1: 0,
                        signature_word: 0xAA55u16.to_le_bytes(),
                    },
                },
            },
        };
        Self { fat32 }
    }

    pub fn info(&self) -> BootSectorInfo {
        let raw_bs: &RawBootSector = bytemuck::cast_ref(self);
        raw_bs.try_into().unwrap()
    }

    pub fn copy_to_bytes(&self, bytes: &mut [u8; 512]) {
        bytes.copy_from_slice(bytemuck::bytes_of(self));
    }
}

unsafe impl bytemuck::NoUninit for BootSector {}
unsafe impl bytemuck::Zeroable for BootSector {}
unsafe impl bytemuck::AnyBitPattern for BootSector {}

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

/*
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

*/

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
}
