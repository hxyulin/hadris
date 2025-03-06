//! Structures for the FAT32 file system
//!
//! Raw byte structures for the FAT32 file system are defined in the `raw` module.
//! The `boot_sector` module defines the boot sector for FAT32 file systems.
//! The `fat` module defines the FAT table for FAT32 file systems.
//! The `fs_info` module defines the FsInfo structure for FAT32 file systems.
//! The `directory` module defines the directory structure for FAT32 file systems.
//! The `file` module defines the file structure for FAT32 file systems.
//! Each module, which is now raw, has a strcture for reading and writing, and a info variant,
//! which contains the info of that module, in the current endianness

use core::str;

use hadris_core::{str::FixedByteStr, bpb::JumpInstruction};

pub mod raw;

#[cfg(feature = "read")]
pub mod boot_sector;
#[cfg(feature = "read")]
pub mod directory;
#[cfg(feature = "read")]
pub mod fat;
pub mod fs_info;
#[cfg(feature = "read")]
pub mod time;

#[cfg(feature = "write")]
#[derive(Debug, Clone)]
pub struct Fat32Ops {
    pub jmp_boot_code: [u8; 3],
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sector_count: u16,
    pub fat_count: u8,
    pub media_type: boot_sector::MediaType,
    pub hidden_sector_count: u32,
    pub total_sectors_32: u32,
    pub sectors_per_fat_32: u32,
    pub root_cluster: u32,
    pub fs_info_sector: u16,
    pub boot_sector: u16,
    pub drive_number: u8,
    pub volume_id: u32,
    pub volume_label: Option<FixedByteStr<11>>,
}

#[cfg(feature = "write")]
impl Fat32Ops {
    #[cfg(feature = "std")]
    fn current_volume_id(seed: u32) -> u32 {
        // TODO: Use an actual one, maybe from the MS-DOS FAT32 spec
        // We get the current time in seconds since the epoch
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let time_part = (now.as_secs() as u32) ^ (now.as_secs().wrapping_shr(32) as u32);
        // We make it seem 'random' by xoring it with the seed
        time_part ^ seed
    }

    #[cfg(not(feature = "std"))]
    fn current_volume_id(seed: u32) -> u32 {
        // We atttempt to make it seem random
        let part_1 = seed ^ 0x12345678;
        let part_2 = part_1 ^ (part_1 >> 3);
        part_2 ^ (part_2 >> 5)
    }

    pub fn recommended_config_for(total_sectors: u32) -> Self {
        let sectors_per_cluster = Self::recommended_sectors_per_cluster(total_sectors);
        let total_clusters = total_sectors / sectors_per_cluster as u32;
        // TODO: Make a proper seeding mechanism and volume id
        let volume_id = Self::current_volume_id(0);

        let mut ops = Self {
            sectors_per_cluster,
            total_sectors_32: total_sectors,
            volume_id,
            ..Default::default()
        };

        ops.sectors_per_fat_32 = Self::approximate_sectors_per_fat(
            total_clusters,
            ops.bytes_per_sector as u32,
            ops.reserved_sector_count as u32,
        );

        ops
    }

    pub fn with_reserved_sectors(mut self, reserved_sectors: u16) -> Self {
        self.reserved_sector_count = reserved_sectors;
        let total_clusters = self.total_sectors_32 / self.sectors_per_cluster as u32;
        // Recalculate the sectors per FAT
        self.sectors_per_fat_32 = Self::approximate_sectors_per_fat(
            total_clusters,
            self.bytes_per_sector as u32,
            self.reserved_sector_count as u32,
        );
        self
    }

    fn approximate_sectors_per_fat(
        total_clusters: u32,
        bytes_per_sector: u32,
        reserved_count: u32,
    ) -> u32 {
        let fat_entries = total_clusters + 2 - reserved_count;
        // sizeof(u32) = 4
        (fat_entries * 4 + bytes_per_sector - 1) / bytes_per_sector
    }

    fn recommended_sectors_per_cluster(total_sectors: u32) -> u8 {
        match total_sectors {
            0..=524_287 => 1,              // < 256MB
            524_288..=1_048_575 => 2,      // < 512MB
            1_048_576..=4_194_303 => 4,    // < 2GB
            4_194_304..=8_388_607 => 8,    // < 4GB
            8_388_608..=16_777_215 => 16,  // < 8GB
            16_777_216..=33_554_431 => 32, // < 16GB
            _ => 64,                       // > 16GB
        }
    }
}

#[cfg(feature = "write")]
impl Default for Fat32Ops {
    /// Create a new FAT32 file system with the recommended configuration (except for anything
    /// dependent on the total number of sectors)
    fn default() -> Self {
        use core::mem::{offset_of, size_of};
        Self {
            jmp_boot_code: JumpInstruction::ShortJump(
                size_of::<raw::boot_sector::RawBpb>() as u8
                    + offset_of!(raw::boot_sector::RawBpbExt32, padding1_1) as u8
                    - 2,
            )
            .to_bytes(),
            bytes_per_sector: 512,
            sectors_per_cluster: 1,
            reserved_sector_count: 32,
            // Only 1 FAT table is supported
            fat_count: 1,
            media_type: boot_sector::MediaType::HardDisk,
            // Not supported
            hidden_sector_count: 0,
            total_sectors_32: 0,
            sectors_per_fat_32: 0,
            root_cluster: 2,
            fs_info_sector: 1,
            boot_sector: 6,
            drive_number: 0x80,
            volume_id: 0,
            volume_label: None,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FatStr<const N: usize> {
    pub raw: [u8; N],
}

impl<const N: usize> core::fmt::Debug for FatStr<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let str = str::from_utf8(&self.raw).unwrap();
        f.debug_struct("FatStr")
            .field("max_len", &Self::MAX_LEN)
            .field("str", &str)
            .finish()
    }
}

impl<const N: usize> Default for FatStr<N> {
    fn default() -> Self {
        Self {
            // Fat often uses spaces for padding
            raw: [b' '; N],
        }
    }
}

impl<const N: usize> FatStr<N> {
    pub const MAX_LEN: usize = N;

    pub fn new_truncate(s: &str) -> Self {
        // TODO: We need to convert everything to uppercase
        if s.len() > N {
            Self::from_slice_unchecked(&s.as_bytes()[..N])
        } else {
            Self::from_slice_unchecked(s.as_bytes())
        }
    }

    pub fn clear(&mut self) {
        self.raw = [b' '; N];
    }

    pub fn try_new(s: &str) -> Result<Self, ()> {
        if s.len() > N {
            return Err(());
        }

        Ok(Self::from_slice_unchecked(s.as_bytes()))
    }

    pub fn from_bytes(bytes: [u8; N]) -> Self {
        Self::from_slice_unchecked(&bytes)
    }

    pub fn from_slice_unchecked(slice: &[u8]) -> Self {
        let mut str = Self::default();
        str.raw[..slice.len()].copy_from_slice(slice);
        str
    }

    pub fn len(&self) -> usize {
        self.raw
            .iter()
            .position(|b| *b == b' ')
            .unwrap_or(Self::MAX_LEN)
    }

    pub fn as_str(&self) -> &str {
        str::from_utf8(&self.raw).unwrap()
    }

    pub fn as_slice(&self) -> &[u8; N] {
        &self.raw
    }

    pub fn copy_from_slice(&mut self, slice: &[u8]) {
        self.raw[..slice.len()].copy_from_slice(slice);
    }
}

unsafe impl<const N: usize> bytemuck::Zeroable for FatStr<N> {}
unsafe impl<const N: usize> bytemuck::NoUninit for FatStr<N> {}
unsafe impl<const N: usize> bytemuck::AnyBitPattern for FatStr<N> {}
