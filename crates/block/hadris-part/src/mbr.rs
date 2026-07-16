//! MBR (Master Boot Record) partition table types.
//!
//! This module provides types for working with MBR partition tables, including:
//! - CHS (Cylinder-Head-Sector) addressing
//! - MBR partition entries
//! - MBR partition table (4 primary partitions)
//! - Partition type definitions

use core::fmt::Debug;
use core::ops::{Index, IndexMut};

use endian_num::Le;

/// A simplified enum for common MBR partition types.
///
/// For a complete list of partition types, see [`MbrPartitionTypeFull`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbrPartitionType {
    /// An unused partition-table entry.
    Empty,
    /// A FAT12 partition.
    Fat12,
    /// A FAT16 partition.
    Fat16,
    /// A legacy extended partition.
    Extended,
    /// A FAT16 partition addressed through LBA extensions.
    Fat16Lba,
    /// An NTFS or other installable-file-system partition.
    Ntfs,
    /// A FAT32 partition.
    Fat32,
    /// A FAT32 partition addressed through LBA extensions.
    Fat32Lba,
    /// An extended partition addressed through LBA extensions.
    ExtendedLba,
    /// An ISO9660 filesystem or hidden NTFS partition.
    Iso9660,
    /// A Linux swap partition.
    LinuxSwap,
    /// A native Linux filesystem partition.
    LinuxNative,
    /// A Linux Logical Volume Manager partition.
    LinuxLvm,
    /// A Linux software RAID partition.
    LinuxRaid,
    /// A GPT protective MBR entry.
    ProtectiveMbr,
    /// An EFI System Partition.
    EfiSystemPartition,
    /// A partition type not represented by a named variant.
    Unknown(u8),
}

impl MbrPartitionType {
    /// Create a `MbrPartitionType` from a raw byte value.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0x00 => Self::Empty,
            0x01 => Self::Fat12,
            0x04 => Self::Fat16,
            0x05 => Self::Extended,
            0x06 => Self::Fat16Lba,
            0x07 => Self::Ntfs,
            0x0b => Self::Fat32,
            0x0c => Self::Fat32Lba,
            0x0f => Self::ExtendedLba,
            0x17 => Self::Iso9660,
            0x82 => Self::LinuxSwap,
            0x83 => Self::LinuxNative,
            0x8E => Self::LinuxLvm,
            0xFD => Self::LinuxRaid,
            0xEE => Self::ProtectiveMbr,
            0xEF => Self::EfiSystemPartition,
            _ => Self::Unknown(value),
        }
    }

    /// Convert this partition type to its raw byte value.
    pub const fn to_u8(&self) -> u8 {
        match self {
            Self::Empty => 0x00,
            Self::Fat12 => 0x01,
            Self::Fat16 => 0x04,
            Self::Extended => 0x05,
            Self::Fat16Lba => 0x06,
            Self::Ntfs => 0x07,
            Self::Fat32 => 0x0b,
            Self::Fat32Lba => 0x0c,
            Self::ExtendedLba => 0x0f,
            Self::Iso9660 => 0x17,
            Self::LinuxSwap => 0x82,
            Self::LinuxNative => 0x83,
            Self::LinuxLvm => 0x8E,
            Self::LinuxRaid => 0xFD,
            Self::ProtectiveMbr => 0xEE,
            Self::EfiSystemPartition => 0xEF,
            Self::Unknown(value) => *value,
        }
    }

    /// Returns whether this partition type represents an empty/unused partition.
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Returns whether this is a protective MBR (used for GPT disks).
    pub const fn is_protective(&self) -> bool {
        matches!(self, Self::ProtectiveMbr)
    }
}

impl core::fmt::Display for MbrPartitionType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "Empty"),
            Self::Fat12 => write!(f, "FAT12"),
            Self::Fat16 => write!(f, "FAT16"),
            Self::Extended => write!(f, "Extended"),
            Self::Fat16Lba => write!(f, "FAT16 LBA"),
            Self::Ntfs => write!(f, "NTFS"),
            Self::Fat32 => write!(f, "FAT32"),
            Self::Fat32Lba => write!(f, "FAT32 LBA"),
            Self::ExtendedLba => write!(f, "Extended LBA"),
            Self::Iso9660 => write!(f, "ISO 9660"),
            Self::LinuxSwap => write!(f, "Linux swap"),
            Self::LinuxNative => write!(f, "Linux"),
            Self::LinuxLvm => write!(f, "Linux LVM"),
            Self::LinuxRaid => write!(f, "Linux RAID"),
            Self::ProtectiveMbr => write!(f, "GPT Protective"),
            Self::EfiSystemPartition => write!(f, "EFI System"),
            Self::Unknown(id) => write!(f, "Unknown (0x{id:02X})"),
        }
    }
}

/// A 3-byte CHS (Cylinder-Head-Sector) address.
///
/// CHS addressing is largely obsolete but is still required for MBR compatibility.
/// Modern systems use LBA addressing, and values exceeding the CHS limit (approximately
/// 8GB with 255 heads, 63 sectors, and 1024 cylinders) are represented as 0xFF, 0xFF, 0xFF.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Chs([u8; 3]);

impl Debug for Chs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Chs")
            .field("c", &self.cylinder())
            .field("h", &self.head())
            .field("s", &self.sector())
            .finish()
    }
}

impl Default for Chs {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Chs {
    /// The CHS value representing "out of range" (beyond CHS addressing limits).
    pub const OUT_OF_RANGE: Chs = Chs([0xFF, 0xFF, 0xFF]);

    /// Standard sectors per track for CHS calculations.
    const SECTORS_PER_TRACK: u32 = 63;
    /// Standard heads per cylinder for CHS calculations.
    const HEADS_PER_CYLINDER: u32 = 255;

    /// Creates a new CHS value from an LBA address (assuming 512-byte sectors).
    ///
    /// If the LBA exceeds the CHS addressing limit (approximately 8GB),
    /// returns [`Chs::OUT_OF_RANGE`].
    pub const fn new(lba: u32) -> Self {
        let cylinder = lba / (Self::SECTORS_PER_TRACK * Self::HEADS_PER_CYLINDER);
        if cylinder > 0x03FF {
            return Self::OUT_OF_RANGE;
        }
        let tmp = lba % (Self::SECTORS_PER_TRACK * Self::HEADS_PER_CYLINDER);
        let head = tmp / Self::SECTORS_PER_TRACK;
        let sector = tmp % Self::SECTORS_PER_TRACK + 1;
        // Sector must fit in 6 bits (0-63)
        assert!(
            sector <= 0b00111111,
            "Sector overflow, this should never happen"
        );
        Self([
            (head & 0x00ff) as u8,
            (sector & 0b00111111) as u8 | ((cylinder & 0x0300) >> 2) as u8,
            (cylinder & 0xFF) as u8,
        ])
    }

    /// Returns the head component (0-255).
    pub const fn head(&self) -> u8 {
        self.0[0]
    }

    /// Returns the sector component (1-63).
    pub const fn sector(&self) -> u8 {
        self.0[1] & 0b00111111
    }

    /// Returns the cylinder component (0-1023).
    pub const fn cylinder(&self) -> u16 {
        ((self.0[1] as u16 & 0b11000000) << 2) | (self.0[2] as u16)
    }

    /// Converts this CHS address to an LBA address.
    ///
    /// Returns `u32::MAX` for out-of-range CHS values.
    pub const fn as_lba(&self) -> u32 {
        if self.0[0] == 0xFF && self.0[1] == 0xFF && self.0[2] == 0xFF {
            return u32::MAX;
        }

        self.cylinder() as u32 * Self::SECTORS_PER_TRACK * Self::HEADS_PER_CYLINDER
            + self.head() as u32 * Self::SECTORS_PER_TRACK
            + self.sector() as u32
            - 1
    }
}

/// An MBR partition entry (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MbrPartition {
    /// Boot indicator: 0x00 = non-bootable, 0x80 = bootable.
    pub boot_indicator: u8,
    /// Starting CHS address.
    pub start_chs: Chs,
    /// Partition type code.
    pub part_type: u8,
    /// Ending CHS address.
    pub end_chs: Chs,
    /// Starting LBA (sector number), little-endian on disk.
    pub start_lba: Le<u32>,
    /// Number of sectors in this partition, little-endian on disk.
    pub sector_count: Le<u32>,
}

impl Default for MbrPartition {
    fn default() -> Self {
        Self {
            boot_indicator: 0x00,
            start_chs: Chs::new(0),
            part_type: 0x00,
            end_chs: Chs::new(0),
            start_lba: Le::<u32>::from_ne(0),
            sector_count: Le::<u32>::from_ne(0),
        }
    }
}

impl MbrPartition {
    /// Creates a new MBR partition entry.
    pub const fn new(part_type: MbrPartitionType, start_lba: u32, sector_count: u32) -> Self {
        let end_lba = if sector_count > 0 {
            start_lba + sector_count - 1
        } else {
            start_lba
        };
        Self {
            boot_indicator: 0x00,
            start_chs: Chs::new(start_lba),
            part_type: part_type.to_u8(),
            end_chs: Chs::new(end_lba),
            start_lba: Le::<u32>::from_ne(start_lba),
            sector_count: Le::<u32>::from_ne(sector_count),
        }
    }

    /// Creates a protective MBR partition entry for GPT disks.
    ///
    /// The protective MBR covers the entire disk (or up to the 32-bit limit).
    pub const fn protective(disk_sectors: u64) -> Self {
        // Protective MBR starts at LBA 1 and covers the entire disk
        // (or 0xFFFFFFFF if disk is larger than 32-bit can represent)
        let size = if disk_sectors > 0xFFFFFFFF {
            0xFFFFFFFF
        } else if disk_sectors > 1 {
            (disk_sectors - 1) as u32
        } else {
            1
        };
        Self::new(MbrPartitionType::ProtectiveMbr, 1, size)
    }

    /// Returns whether this partition entry is empty (unused).
    pub const fn is_empty(&self) -> bool {
        self.part_type == 0x00
    }

    /// Returns the partition type.
    pub const fn partition_type(&self) -> MbrPartitionType {
        MbrPartitionType::from_u8(self.part_type)
    }

    /// Returns whether this partition is marked as bootable.
    pub const fn is_bootable(&self) -> bool {
        self.boot_indicator == 0x80
    }

    /// Sets this partition as bootable or non-bootable.
    pub fn set_bootable(&mut self, bootable: bool) {
        self.boot_indicator = if bootable { 0x80 } else { 0x00 };
    }

    /// Returns the ending LBA (inclusive).
    pub const fn end_lba(&self) -> u32 {
        let start = self.start_lba.to_ne();
        let count = self.sector_count.to_ne();
        if count == 0 { start } else { start + count - 1 }
    }
}

/// The MBR partition table (4 primary partition entries).
#[repr(transparent)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct MbrPartitionTable {
    /// The four primary partition entries.
    pub partitions: [MbrPartition; 4],
}

impl Default for MbrPartitionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for MbrPartitionTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let non_empty: usize = self.partitions.iter().filter(|p| !p.is_empty()).count();
        f.debug_struct("MbrPartitionTable")
            .field("partitions", &&self.partitions[..non_empty])
            .finish()
    }
}

impl MbrPartitionTable {
    /// A zeroed (empty) MBR partition table entry.
    const EMPTY_PARTITION: MbrPartition = MbrPartition {
        boot_indicator: 0,
        start_chs: Chs([0, 0, 0]),
        part_type: 0,
        end_chs: Chs([0, 0, 0]),
        start_lba: Le::<u32>::from_ne(0),
        sector_count: Le::<u32>::from_ne(0),
    };

    /// Creates a new empty MBR partition table.
    pub const fn new() -> Self {
        Self {
            partitions: [Self::EMPTY_PARTITION; 4],
        }
    }

    /// Creates a protective MBR partition table for GPT disks.
    ///
    /// This creates a single partition entry covering the entire disk,
    /// with type 0xEE (GPT Protective).
    pub const fn protective(disk_sectors: u64) -> Self {
        let mut table = Self::new();
        table.partitions[0] = MbrPartition::protective(disk_sectors);
        table
    }

    /// Returns the number of non-empty partition entries.
    pub fn count(&self) -> usize {
        self.partitions.iter().filter(|p| !p.is_empty()).count()
    }

    /// Returns whether this appears to be a valid MBR partition table.
    ///
    /// Checks that boot indicators are valid (0x00 or 0x80).
    pub fn is_valid(&self) -> bool {
        for partition in &self.partitions {
            // Boot indicator must be 0x00 or 0x80
            if (partition.boot_indicator & !0x80) != 0 {
                return false;
            }
        }
        true
    }

    /// Returns whether this is a protective MBR (indicating a GPT disk).
    pub fn is_protective(&self) -> bool {
        !self.partitions[0].is_empty() && self.partitions[0].partition_type().is_protective()
    }

    /// Returns an iterator over non-empty partitions.
    pub fn iter(&self) -> impl Iterator<Item = &MbrPartition> {
        self.partitions.iter().filter(|p| !p.is_empty())
    }

    /// Returns a mutable iterator over all partition slots.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut MbrPartition> {
        self.partitions.iter_mut()
    }
}

impl Index<usize> for MbrPartitionTable {
    type Output = MbrPartition;

    fn index(&self, index: usize) -> &Self::Output {
        &self.partitions[index]
    }
}

impl IndexMut<usize> for MbrPartitionTable {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.partitions[index]
    }
}

/// The complete Master Boot Record structure (512 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct MasterBootRecord {
    /// Bootstrap code area (446 bytes).
    pub bootstrap: [u8; 446],
    /// Partition table (64 bytes - 4 entries of 16 bytes each).
    pub partition_table: MbrPartitionTable,
    /// Boot signature (must be 0x55, 0xAA).
    pub signature: [u8; 2],
}

// SAFETY: MasterBootRecord is #[repr(C, packed)] with 512 bytes total,
// containing only byte arrays and MbrPartitionTable (which is Pod).
// All bit patterns are valid.
unsafe impl bytemuck::Pod for MasterBootRecord {}
unsafe impl bytemuck::Zeroable for MasterBootRecord {}

impl Default for MasterBootRecord {
    fn default() -> Self {
        Self {
            bootstrap: [0; 446],
            partition_table: MbrPartitionTable::default(),
            signature: [0x55, 0xAA],
        }
    }
}

impl MasterBootRecord {
    /// The required boot signature bytes.
    pub const SIGNATURE: [u8; 2] = [0x55, 0xAA];

    /// Creates a new MBR with the given partition table.
    pub const fn new(partition_table: MbrPartitionTable) -> Self {
        Self {
            bootstrap: [0; 446],
            partition_table,
            signature: Self::SIGNATURE,
        }
    }

    /// Creates a protective MBR for GPT disks.
    pub const fn protective(disk_sectors: u64) -> Self {
        Self::new(MbrPartitionTable::protective(disk_sectors))
    }

    /// Returns whether this MBR has a valid boot signature.
    pub const fn has_valid_signature(&self) -> bool {
        self.signature[0] == 0x55 && self.signature[1] == 0xAA
    }

    /// Returns a copy of the partition table.
    ///
    /// This method exists because the struct is packed and direct field access
    /// would create a misaligned reference.
    pub fn get_partition_table(&self) -> MbrPartitionTable {
        // Copy the partition table to avoid alignment issues
        self.partition_table
    }

    /// Sets the partition table.
    ///
    /// This method exists because the struct is packed and direct field access
    /// would create a misaligned reference.
    pub fn set_partition_table(&mut self, table: MbrPartitionTable) {
        self.partition_table = table;
    }

    /// Modifies the partition table using a closure.
    ///
    /// This is a convenience method that gets the partition table, allows
    /// modification via a closure, and sets it back.
    pub fn with_partition_table<F>(&mut self, f: F)
    where
        F: FnOnce(&mut MbrPartitionTable),
    {
        let mut pt = self.get_partition_table();
        f(&mut pt);
        self.set_partition_table(pt);
    }

    /// Returns whether this MBR is valid (has correct signature and valid partition table).
    pub fn is_valid(&self) -> bool {
        let pt = self.get_partition_table();
        self.has_valid_signature() && pt.is_valid()
    }
}

impl Debug for MasterBootRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let pt = self.get_partition_table();
        f.debug_struct("MasterBootRecord")
            .field("partition_table", &pt)
            .field(
                "signature",
                &format_args!("0x{:02X}{:02X}", self.signature[0], self.signature[1]),
            )
            .finish()
    }
}

/// An enum representing the full list of MBR partition types.
///
/// Based on the comprehensive list at <https://thestarman.pcministry.com/asm/mbr/PartTypes.htm>
/// For a simpler interface, use [`MbrPartitionType`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbrPartitionTypeFull {
    /// Empty partition
    Empty = 0x00,
    /// FAT12 partition
    Fat12 = 0x01,
    /// XENIX root partition
    XenixRoot = 0x02,
    /// XENIX /usr partition (obsolete)
    XenixUsr = 0x03,
    /// FAT16 partition (less than 32M)
    Fat16S = 0x04,
    /// Extended partition
    Extended = 0x05,
    /// FAT16 partition (more than 32M)
    Fat16L = 0x06,
    /// Installable file systems (HPFS, NTFS)
    Installable = 0x07,
    /// AIX bootable partition
    AixBoot = 0x08,
    /// AIX data partition
    AixData = 0x09,
    /// OS/2 Boot Manager partition
    Os2Boot = 0x0A,
    /// FAT32 partition
    Fat32 = 0x0B,
    /// FAT32 partition (using int13 extensions)
    Fat32Lba = 0x0C,
    /// Legacy MBR partition type `Reserved0D` (0x0D).
    Reserved0D = 0x0D,
    /// FAT16 partition (using int13 extensions)
    Fat16Lba = 0x0E,
    /// Extended partition (using int13 extensions)
    ExtendedLba = 0x0F,
    /// Legacy MBR partition type `Opus` (0x10).
    Opus = 0x10,
    /// Legacy MBR partition type `HiddenFat12` (0x11).
    HiddenFat12 = 0x11,
    /// Legacy MBR partition type `CompaqDiagnosis` (0x12).
    CompaqDiagnosis = 0x12,
    /// Legacy MBR partition type `Reserved13` (0x13).
    Reserved13 = 0x13,
    /// Legacy MBR partition type `HiddenFat16S` (0x14).
    HiddenFat16S = 0x14,
    /// Legacy MBR partition type `Reserved15` (0x15).
    Reserved15 = 0x15,
    /// Legacy MBR partition type `HiddenFat16L` (0x16).
    HiddenFat16L = 0x16,
    /// Hidden IFS (HPFS, NTFS) / ISO9660
    HiddenIfs = 0x17,
    /// Legacy MBR partition type `AstWindowsSwap` (0x18).
    AstWindowsSwap = 0x18,
    /// Legacy MBR partition type `WillowtechPhotonCos` (0x19).
    WillowtechPhotonCos = 0x19,
    /// Legacy MBR partition type `Reserved1A` (0x1A).
    Reserved1A = 0x1A,
    /// Legacy MBR partition type `HiddenFat32` (0x1B).
    HiddenFat32 = 0x1B,
    /// Legacy MBR partition type `HiddenFat32Lba` (0x1C).
    HiddenFat32Lba = 0x1C,
    /// Legacy MBR partition type `Reserved1D` (0x1D).
    Reserved1D = 0x1D,
    /// Legacy MBR partition type `HiddenFat16Lba` (0x1E).
    HiddenFat16Lba = 0x1E,
    /// Legacy MBR partition type `Reserved1F` (0x1F).
    Reserved1F = 0x1F,
    /// Willowsoft Overture File System
    Ofs1 = 0x20,
    /// Legacy MBR partition type `Reserved21` (0x21).
    Reserved21 = 0x21,
    /// Legacy MBR partition type `OxygenExt` (0x22).
    OxygenExt = 0x22,
    /// Legacy MBR partition type `Reserved23` (0x23).
    Reserved23 = 0x23,
    /// Legacy MBR partition type `NecMsDos` (0x24).
    NecMsDos = 0x24,
    /// Legacy MBR partition type `Reserved25` (0x25).
    Reserved25 = 0x25,
    /// Legacy MBR partition type `Reserved26` (0x26).
    Reserved26 = 0x26,
    /// Legacy MBR partition type `Reserved27` (0x27).
    Reserved27 = 0x27,
    /// Legacy MBR partition type `Reserved28` (0x28).
    Reserved28 = 0x28,
    /// Legacy MBR partition type `Reserved29` (0x29).
    Reserved29 = 0x29,
    /// Legacy MBR partition type `Reserved2A` (0x2A).
    Reserved2A = 0x2A,
    /// Legacy MBR partition type `Reserved2B` (0x2B).
    Reserved2B = 0x2B,
    /// Legacy MBR partition type `Reserved2C` (0x2C).
    Reserved2C = 0x2C,
    /// Legacy MBR partition type `Reserved2D` (0x2D).
    Reserved2D = 0x2D,
    /// Legacy MBR partition type `Reserved2E` (0x2E).
    Reserved2E = 0x2E,
    /// Legacy MBR partition type `Reserved2F` (0x2F).
    Reserved2F = 0x2F,
    /// Legacy MBR partition type `Reserved30` (0x30).
    Reserved30 = 0x30,
    /// Legacy MBR partition type `Reserved31` (0x31).
    Reserved31 = 0x31,
    /// Legacy MBR partition type `Reserved32` (0x32).
    Reserved32 = 0x32,
    /// Legacy MBR partition type `Reserved33` (0x33).
    Reserved33 = 0x33,
    /// Legacy MBR partition type `Reserved34` (0x34).
    Reserved34 = 0x34,
    /// Legacy MBR partition type `Reserved35` (0x35).
    Reserved35 = 0x35,
    /// Legacy MBR partition type `Reserved36` (0x36).
    Reserved36 = 0x36,
    /// Legacy MBR partition type `Reserved37` (0x37).
    Reserved37 = 0x37,
    /// Legacy MBR partition type `Theos` (0x38).
    Theos = 0x38,
    /// Legacy MBR partition type `Reserved39` (0x39).
    Reserved39 = 0x39,
    /// Legacy MBR partition type `Reserved3A` (0x3A).
    Reserved3A = 0x3A,
    /// Legacy MBR partition type `Reserved3B` (0x3B).
    Reserved3B = 0x3B,
    /// Legacy MBR partition type `PowerQuestFiles` (0x3C).
    PowerQuestFiles = 0x3C,
    /// Legacy MBR partition type `HiddenNetWare` (0x3D).
    HiddenNetWare = 0x3D,
    /// Legacy MBR partition type `Reserved3E` (0x3E).
    Reserved3E = 0x3E,
    /// Legacy MBR partition type `Reserved3F` (0x3F).
    Reserved3F = 0x3F,
    /// Legacy MBR partition type `Venix80286` (0x40).
    Venix80286 = 0x40,
    /// Legacy MBR partition type `PpcBoot` (0x41).
    PpcBoot = 0x41,
    /// Legacy MBR partition type `SecureFileSystem` (0x42).
    SecureFileSystem = 0x42,
    /// Legacy MBR partition type `AltExt2Fs` (0x43).
    AltExt2Fs = 0x43,
    /// Legacy MBR partition type `Reserved44` (0x44).
    Reserved44 = 0x44,
    /// Legacy MBR partition type `Priam` (0x45).
    Priam = 0x45,
    /// Legacy MBR partition type `EumelElan46` (0x46).
    EumelElan46 = 0x46,
    /// Legacy MBR partition type `EumelElan47` (0x47).
    EumelElan47 = 0x47,
    /// Legacy MBR partition type `EumelElan48` (0x48).
    EumelElan48 = 0x48,
    /// Legacy MBR partition type `Reserved49` (0x49).
    Reserved49 = 0x49,
    /// Legacy MBR partition type `Alfs` (0x4A).
    Alfs = 0x4A,
    /// Legacy MBR partition type `Reserved4B` (0x4B).
    Reserved4B = 0x4B,
    /// Legacy MBR partition type `Reserved4C` (0x4C).
    Reserved4C = 0x4C,
    /// Legacy MBR partition type `Qnx4D` (0x4D).
    Qnx4D = 0x4D,
    /// Legacy MBR partition type `Qnx4E` (0x4E).
    Qnx4E = 0x4E,
    /// Legacy MBR partition type `Qnx4F` (0x4F).
    Qnx4F = 0x4F,
    /// Legacy MBR partition type `OdmReadOnly` (0x50).
    OdmReadOnly = 0x50,
    /// Legacy MBR partition type `OdmReadWrite` (0x51).
    OdmReadWrite = 0x51,
    /// Legacy MBR partition type `CPM` (0x52).
    CPM = 0x52,
    /// Legacy MBR partition type `OdmWriteOnly` (0x53).
    OdmWriteOnly = 0x53,
    /// Legacy MBR partition type `Odm6` (0x54).
    Odm6 = 0x54,
    /// Legacy MBR partition type `EzDrive` (0x55).
    EzDrive = 0x55,
    /// Legacy MBR partition type `GoldenBow` (0x56).
    GoldenBow = 0x56,
    /// Legacy MBR partition type `Reserved57` (0x57).
    Reserved57 = 0x57,
    /// Legacy MBR partition type `Reserved58` (0x58).
    Reserved58 = 0x58,
    /// Legacy MBR partition type `Reserved59` (0x59).
    Reserved59 = 0x59,
    /// Legacy MBR partition type `Reserved5A` (0x5A).
    Reserved5A = 0x5A,
    /// Legacy MBR partition type `Reserved5B` (0x5B).
    Reserved5B = 0x5B,
    /// Legacy MBR partition type `PriamEDisk` (0x5C).
    PriamEDisk = 0x5C,
    /// Legacy MBR partition type `Reserved5D` (0x5D).
    Reserved5D = 0x5D,
    /// Legacy MBR partition type `Reserved5E` (0x5E).
    Reserved5E = 0x5E,
    /// Legacy MBR partition type `Reserved5F` (0x5F).
    Reserved5F = 0x5F,
    /// Legacy MBR partition type `Reserved60` (0x60).
    Reserved60 = 0x60,
    /// Legacy MBR partition type `StorageDimension1` (0x61).
    StorageDimension1 = 0x61,
    /// Legacy MBR partition type `Reserved62` (0x62).
    Reserved62 = 0x62,
    /// Legacy MBR partition type `GnuHurd` (0x63).
    GnuHurd = 0x63,
    /// Legacy MBR partition type `NovellNetware286` (0x64).
    NovellNetware286 = 0x64,
    /// Legacy MBR partition type `NovellNetware311` (0x65).
    NovellNetware311 = 0x65,
    /// Legacy MBR partition type `NovellNetware386` (0x66).
    NovellNetware386 = 0x66,
    /// Legacy MBR partition type `NovellNetware67` (0x67).
    NovellNetware67 = 0x67,
    /// Legacy MBR partition type `NovellNetware68` (0x68).
    NovellNetware68 = 0x68,
    /// Legacy MBR partition type `NovellNetware5` (0x69).
    NovellNetware5 = 0x69,
    /// Legacy MBR partition type `Reserved6A` (0x6A).
    Reserved6A = 0x6A,
    /// Legacy MBR partition type `Reserved6B` (0x6B).
    Reserved6B = 0x6B,
    /// Legacy MBR partition type `Reserved6C` (0x6C).
    Reserved6C = 0x6C,
    /// Legacy MBR partition type `Reserved6D` (0x6D).
    Reserved6D = 0x6D,
    /// Legacy MBR partition type `Reserved6E` (0x6E).
    Reserved6E = 0x6E,
    /// Legacy MBR partition type `Reserved6F` (0x6F).
    Reserved6F = 0x6F,
    /// Legacy MBR partition type `DiskSecureMultiBoot` (0x70).
    DiskSecureMultiBoot = 0x70,
    /// Legacy MBR partition type `Reserved71` (0x71).
    Reserved71 = 0x71,
    /// Legacy MBR partition type `Reserved72` (0x72).
    Reserved72 = 0x72,
    /// Legacy MBR partition type `Reserved73` (0x73).
    Reserved73 = 0x73,
    /// Legacy MBR partition type `Reserved74` (0x74).
    Reserved74 = 0x74,
    /// Legacy MBR partition type `IbmPcIx` (0x75).
    IbmPcIx = 0x75,
    /// Legacy MBR partition type `Reserved76` (0x76).
    Reserved76 = 0x76,
    /// Legacy MBR partition type `Reserved77` (0x77).
    Reserved77 = 0x77,
    /// Legacy MBR partition type `Reserved78` (0x78).
    Reserved78 = 0x78,
    /// Legacy MBR partition type `Reserved79` (0x79).
    Reserved79 = 0x79,
    /// Legacy MBR partition type `Reserved7A` (0x7A).
    Reserved7A = 0x7A,
    /// Legacy MBR partition type `Reserved7B` (0x7B).
    Reserved7B = 0x7B,
    /// Legacy MBR partition type `Reserved7C` (0x7C).
    Reserved7C = 0x7C,
    /// Legacy MBR partition type `Reserved7D` (0x7D).
    Reserved7D = 0x7D,
    /// Legacy MBR partition type `Reserved7E` (0x7E).
    Reserved7E = 0x7E,
    /// Legacy MBR partition type `Reserved7F` (0x7F).
    Reserved7F = 0x7F,
    /// Legacy MBR partition type `OldMinix` (0x80).
    OldMinix = 0x80,
    /// Legacy MBR partition type `LinuxMinix` (0x81).
    LinuxMinix = 0x81,
    /// Linux Swap partition
    LinuxSwap = 0x82,
    /// Linux native file systems (ext2/3/4, etc.)
    LinuxNative = 0x83,
    /// Legacy MBR partition type `Os2Hidden` (0x84).
    Os2Hidden = 0x84,
    /// Legacy MBR partition type `LinuxExtended` (0x85).
    LinuxExtended = 0x85,
    /// Legacy MBR partition type `NtStripeSet` (0x86).
    NtStripeSet = 0x86,
    /// Legacy MBR partition type `HpfsFtMirrored` (0x87).
    HpfsFtMirrored = 0x87,
    /// Legacy MBR partition type `Reserved88` (0x88).
    Reserved88 = 0x88,
    /// Legacy MBR partition type `Reserved89` (0x89).
    Reserved89 = 0x89,
    /// Legacy MBR partition type `Reserved8A` (0x8A).
    Reserved8A = 0x8A,
    /// Legacy MBR partition type `Reserved8B` (0x8B).
    Reserved8B = 0x8B,
    /// Legacy MBR partition type `Reserved8C` (0x8C).
    Reserved8C = 0x8C,
    /// Legacy MBR partition type `Reserved8D` (0x8D).
    Reserved8D = 0x8D,
    /// Linux LVM
    LinuxLvm = 0x8E,
    /// Legacy MBR partition type `Reserved8F` (0x8F).
    Reserved8F = 0x8F,
    /// Legacy MBR partition type `Reserved90` (0x90).
    Reserved90 = 0x90,
    /// Legacy MBR partition type `Reserved91` (0x91).
    Reserved91 = 0x91,
    /// Legacy MBR partition type `Reserved92` (0x92).
    Reserved92 = 0x92,
    /// Legacy MBR partition type `HiddenLinuxNative` (0x93).
    HiddenLinuxNative = 0x93,
    /// Legacy MBR partition type `AmoebaBadBlockTable` (0x94).
    AmoebaBadBlockTable = 0x94,
    /// Legacy MBR partition type `Reserved95` (0x95).
    Reserved95 = 0x95,
    /// Legacy MBR partition type `Reserved96` (0x96).
    Reserved96 = 0x96,
    /// Legacy MBR partition type `Reserved97` (0x97).
    Reserved97 = 0x97,
    /// Legacy MBR partition type `Reserved98` (0x98).
    Reserved98 = 0x98,
    /// Legacy MBR partition type `Mylex` (0x99).
    Mylex = 0x99,
    /// Legacy MBR partition type `Reserved9A` (0x9A).
    Reserved9A = 0x9A,
    /// Legacy MBR partition type `Reserved9B` (0x9B).
    Reserved9B = 0x9B,
    /// Legacy MBR partition type `Reserved9C` (0x9C).
    Reserved9C = 0x9C,
    /// Legacy MBR partition type `Reserved9D` (0x9D).
    Reserved9D = 0x9D,
    /// Legacy MBR partition type `Reserved9E` (0x9E).
    Reserved9E = 0x9E,
    /// Legacy MBR partition type `Bsdi` (0x9F).
    Bsdi = 0x9F,
    /// Legacy MBR partition type `IbmHibernation` (0xA0).
    IbmHibernation = 0xA0,
    /// Legacy MBR partition type `HpVolumeExpA1` (0xA1).
    HpVolumeExpA1 = 0xA1,
    /// Legacy MBR partition type `ReservedA2` (0xA2).
    ReservedA2 = 0xA2,
    /// Legacy MBR partition type `HpVolumeExpA3` (0xA3).
    HpVolumeExpA3 = 0xA3,
    /// Legacy MBR partition type `HpVolumeExpA4` (0xA4).
    HpVolumeExpA4 = 0xA4,
    /// Legacy MBR partition type `FreeBsd386` (0xA5).
    FreeBsd386 = 0xA5,
    /// Legacy MBR partition type `OpenBsd` (0xA6).
    OpenBsd = 0xA6,
    /// Legacy MBR partition type `HpVolumeExpA7` (0xA7).
    HpVolumeExpA7 = 0xA7,
    /// Legacy MBR partition type `MacOsX` (0xA8).
    MacOsX = 0xA8,
    /// Legacy MBR partition type `NetBsd` (0xA9).
    NetBsd = 0xA9,
    /// Legacy MBR partition type `Olivetti` (0xAA).
    Olivetti = 0xAA,
    /// Legacy MBR partition type `MacOsXBoot` (0xAB).
    MacOsXBoot = 0xAB,
    /// Legacy MBR partition type `ReservedAC` (0xAC).
    ReservedAC = 0xAC,
    /// Legacy MBR partition type `ReservedAD` (0xAD).
    ReservedAD = 0xAD,
    /// Legacy MBR partition type `ReservedAE` (0xAE).
    ReservedAE = 0xAE,
    /// Legacy MBR partition type `MacOsXHfsPlus` (0xAF).
    MacOsXHfsPlus = 0xAF,
    /// Legacy MBR partition type `BootMngrBootStar` (0xB0).
    BootMngrBootStar = 0xB0,
    /// Legacy MBR partition type `HpVolumeExpB1` (0xB1).
    HpVolumeExpB1 = 0xB1,
    /// Legacy MBR partition type `HpVolumeExpB2` (0xB2).
    HpVolumeExpB2 = 0xB2,
    /// Legacy MBR partition type `HpVolumeExpB3` (0xB3).
    HpVolumeExpB3 = 0xB3,
    /// Legacy MBR partition type `HpVolumeExpB4` (0xB4).
    HpVolumeExpB4 = 0xB4,
    /// Legacy MBR partition type `ReservedB5` (0xB5).
    ReservedB5 = 0xB5,
    /// Legacy MBR partition type `HpVolumeExpB6` (0xB6).
    HpVolumeExpB6 = 0xB6,
    /// Legacy MBR partition type `BsdiFs` (0xB7).
    BsdiFs = 0xB7,
    /// Legacy MBR partition type `BsdiSwap` (0xB8).
    BsdiSwap = 0xB8,
    /// Legacy MBR partition type `ReservedB9` (0xB9).
    ReservedB9 = 0xB9,
    /// Legacy MBR partition type `ReservedBA` (0xBA).
    ReservedBA = 0xBA,
    /// Legacy MBR partition type `PtsBootWizard` (0xBB).
    PtsBootWizard = 0xBB,
    /// Legacy MBR partition type `AcronisBackup` (0xBC).
    AcronisBackup = 0xBC,
    /// Legacy MBR partition type `ReservedBD` (0xBD).
    ReservedBD = 0xBD,
    /// Legacy MBR partition type `SolarisBoot` (0xBE).
    SolarisBoot = 0xBE,
    /// Legacy MBR partition type `Solaris` (0xBF).
    Solaris = 0xBF,
    /// Legacy MBR partition type `NovellDos` (0xC0).
    NovellDos = 0xC0,
    /// Legacy MBR partition type `DrDos12` (0xC1).
    DrDos12 = 0xC1,
    /// Legacy MBR partition type `ReservedC2` (0xC2).
    ReservedC2 = 0xC2,
    /// Legacy MBR partition type `ReservedC3` (0xC3).
    ReservedC3 = 0xC3,
    /// Legacy MBR partition type `DrDos16` (0xC4).
    DrDos16 = 0xC4,
    /// Legacy MBR partition type `ReservedC5` (0xC5).
    ReservedC5 = 0xC5,
    /// Legacy MBR partition type `DrDosHuge` (0xC6).
    DrDosHuge = 0xC6,
    /// Legacy MBR partition type `HpfsFtMirroredDisabled` (0xC7).
    HpfsFtMirroredDisabled = 0xC7,
    /// Legacy MBR partition type `ReservedC8` (0xC8).
    ReservedC8 = 0xC8,
    /// Legacy MBR partition type `ReservedC9` (0xC9).
    ReservedC9 = 0xC9,
    /// Legacy MBR partition type `ReservedCA` (0xCA).
    ReservedCA = 0xCA,
    /// Legacy MBR partition type `ReservedCB` (0xCB).
    ReservedCB = 0xCB,
    /// Legacy MBR partition type `ReservedCC` (0xCC).
    ReservedCC = 0xCC,
    /// Legacy MBR partition type `ReservedCD` (0xCD).
    ReservedCD = 0xCD,
    /// Legacy MBR partition type `ReservedCE` (0xCE).
    ReservedCE = 0xCE,
    /// Legacy MBR partition type `ReservedCF` (0xCF).
    ReservedCF = 0xCF,
    /// Legacy MBR partition type `MultiuserDos` (0xD0).
    MultiuserDos = 0xD0,
    /// Legacy MBR partition type `OldMultiuserDos` (0xD1).
    OldMultiuserDos = 0xD1,
    /// Legacy MBR partition type `ReservedD2` (0xD2).
    ReservedD2 = 0xD2,
    /// Legacy MBR partition type `ReservedD3` (0xD3).
    ReservedD3 = 0xD3,
    /// Legacy MBR partition type `OldMultiuserDos2` (0xD4).
    OldMultiuserDos2 = 0xD4,
    /// Legacy MBR partition type `OldMultiuserDos3` (0xD5).
    OldMultiuserDos3 = 0xD5,
    /// Legacy MBR partition type `OldMultiuserDos4` (0xD6).
    OldMultiuserDos4 = 0xD6,
    /// Legacy MBR partition type `ReservedD7` (0xD7).
    ReservedD7 = 0xD7,
    /// Legacy MBR partition type `Cpm86` (0xD8).
    Cpm86 = 0xD8,
    /// Legacy MBR partition type `ReservedD9` (0xD9).
    ReservedD9 = 0xD9,
    /// Legacy MBR partition type `ReservedDA` (0xDA).
    ReservedDA = 0xDA,
    /// Legacy MBR partition type `Cpm` (0xDB).
    Cpm = 0xDB,
    /// Legacy MBR partition type `ReservedDC` (0xDC).
    ReservedDC = 0xDC,
    /// Legacy MBR partition type `ReservedDD` (0xDD).
    ReservedDD = 0xDD,
    /// Legacy MBR partition type `Dell` (0xDE).
    Dell = 0xDE,
    /// Legacy MBR partition type `Embrm` (0xDF).
    Embrm = 0xDF,
    /// Legacy MBR partition type `ReservedE0` (0xE0).
    ReservedE0 = 0xE0,
    /// Legacy MBR partition type `SpeedStorFat12Ext` (0xE1).
    SpeedStorFat12Ext = 0xE1,
    /// Legacy MBR partition type `DosReadOnly` (0xE2).
    DosReadOnly = 0xE2,
    /// Legacy MBR partition type `SpeedStor` (0xE3).
    SpeedStor = 0xE3,
    /// Legacy MBR partition type `SpeedStor16Ext` (0xE4).
    SpeedStor16Ext = 0xE4,
    /// Legacy MBR partition type `ReservedE5` (0xE5).
    ReservedE5 = 0xE5,
    /// Legacy MBR partition type `StorageDimension2` (0xE6).
    StorageDimension2 = 0xE6,
    /// Legacy MBR partition type `ReservedE7` (0xE7).
    ReservedE7 = 0xE7,
    /// Legacy MBR partition type `ReservedE8` (0xE8).
    ReservedE8 = 0xE8,
    /// Legacy MBR partition type `ReservedE9` (0xE9).
    ReservedE9 = 0xE9,
    /// Legacy MBR partition type `ReservedEA` (0xEA).
    ReservedEA = 0xEA,
    /// Legacy MBR partition type `BeOs` (0xEB).
    BeOs = 0xEB,
    /// Legacy MBR partition type `ReservedEC` (0xEC).
    ReservedEC = 0xEC,
    /// Legacy MBR partition type `ReservedED` (0xED).
    ReservedED = 0xED,
    /// GPT Protective MBR
    GptProtectiveMbr = 0xEE,
    /// EFI System Partition
    EfiSystemPartition = 0xEF,
    /// Legacy MBR partition type `ReservedF0` (0xF0).
    ReservedF0 = 0xF0,
    /// Legacy MBR partition type `SpeedStorDimensions` (0xF1).
    SpeedStorDimensions = 0xF1,
    /// Legacy MBR partition type `UnisysDos` (0xF2).
    UnisysDos = 0xF2,
    /// Legacy MBR partition type `StorageDimension3` (0xF3).
    StorageDimension3 = 0xF3,
    /// Legacy MBR partition type `SpeedStorDimensions2` (0xF4).
    SpeedStorDimensions2 = 0xF4,
    /// Legacy MBR partition type `Prolugue` (0xF5).
    Prolugue = 0xF5,
    /// Legacy MBR partition type `StorageDimension4` (0xF6).
    StorageDimension4 = 0xF6,
    /// Legacy MBR partition type `ReservedF7` (0xF7).
    ReservedF7 = 0xF7,
    /// Legacy MBR partition type `ReservedF8` (0xF8).
    ReservedF8 = 0xF8,
    /// Legacy MBR partition type `ReservedF9` (0xF9).
    ReservedF9 = 0xF9,
    /// Legacy MBR partition type `ReservedFA` (0xFA).
    ReservedFA = 0xFA,
    /// Legacy MBR partition type `ReservedFB` (0xFB).
    ReservedFB = 0xFB,
    /// Legacy MBR partition type `ReservedFC` (0xFC).
    ReservedFC = 0xFC,
    /// Linux RAID
    LinuxRaid = 0xFD,
    /// Legacy MBR partition type `LanStep` (0xFE).
    LanStep = 0xFE,
    /// Legacy MBR partition type `BadBlockTable` (0xFF).
    BadBlockTable = 0xFF,
}

impl MbrPartitionTypeFull {
    /// Create a new [`MbrPartitionTypeFull`] from a raw byte.
    ///
    /// # Safety
    ///
    /// This is safe because all 256 possible u8 values are valid enum variants.
    pub const fn from_u8(value: u8) -> Self {
        // SAFETY: All u8 values are valid variants
        unsafe { core::mem::transmute(value) }
    }

    /// Convert this partition type to its raw byte value.
    pub const fn to_u8(&self) -> u8 {
        *self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chs_create() {
        assert_eq!(Chs::new(0), Chs([0, 1, 0]));
        assert_eq!(Chs::new(1), Chs([0, 2, 0]));
        assert_eq!(Chs::new(62), Chs([0, 63, 0]));
        assert_eq!(Chs::new(63), Chs([1, 1, 0]));
        assert_eq!(Chs::new(63 * 254), Chs([254, 1, 0]));
        assert_eq!(Chs::new(63 * 255), Chs([0, 1, 1]));
        assert_eq!(Chs::new(63 * 255 * 255), Chs([0, 1, 255]));
        assert_eq!(Chs::new(63 * 255 * 1023), Chs([0, (15 << 6) + 1, 255]));
        // Out of range
        assert_eq!(Chs::new(63 * 255 * 1024), Chs::OUT_OF_RANGE);
    }

    #[test]
    fn test_chs_get_lba() {
        assert_eq!(Chs([0, 1, 0]).as_lba(), 0);
        assert_eq!(Chs([0, 2, 0]).as_lba(), 1);
        assert_eq!(Chs([0, 63, 0]).as_lba(), 62);
        assert_eq!(Chs([1, 1, 0]).as_lba(), 63);
        assert_eq!(Chs([254, 1, 0]).as_lba(), 63 * 254);
        assert_eq!(Chs([0, 1, 1]).as_lba(), 63 * 255);
        assert_eq!(Chs([0, 1, 255]).as_lba(), 63 * 255 * 255);
        assert_eq!(Chs([0, (15 << 6) + 1, 255]).as_lba(), 63 * 255 * 1023);
        // Out of range
        assert_eq!(Chs::OUT_OF_RANGE.as_lba(), u32::MAX);
    }

    #[test]
    fn test_mbr_partition_table_size() {
        assert_eq!(core::mem::size_of::<MbrPartitionTable>(), 64);
        assert_eq!(core::mem::size_of::<MbrPartition>(), 16);
        assert_eq!(core::mem::size_of::<MasterBootRecord>(), 512);
    }

    #[test]
    fn test_protective_mbr() {
        let mbr = MasterBootRecord::protective(1000);
        assert!(mbr.has_valid_signature());
        let pt = mbr.get_partition_table();
        assert!(pt.is_protective());
        assert_eq!(pt[0].start_lba.to_ne(), 1);
        assert_eq!(pt[0].sector_count.to_ne(), 999);
    }

    #[test]
    fn test_partition_type_full_transmute() {
        // Verify all 256 values are valid
        for i in 0u8..=255 {
            let pt = MbrPartitionTypeFull::from_u8(i);
            assert_eq!(pt.to_u8(), i);
        }
    }
}
