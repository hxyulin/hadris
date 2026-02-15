//! MBR (Master Boot Record) partition table types.
//!
//! This module provides types for working with MBR partition tables, including:
//! - CHS (Cylinder-Head-Sector) addressing
//! - MBR partition entries
//! - MBR partition table (4 primary partitions)
//! - Partition type definitions

use core::fmt::Debug;
use core::ops::{Index, IndexMut};

/// A simplified enum for common MBR partition types.
///
/// For a complete list of partition types, see [`MbrPartitionTypeFull`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbrPartitionType {
    Empty,
    Fat12,
    Fat16,
    Extended,
    Fat16Lba,
    Ntfs,
    Fat32,
    Fat32Lba,
    ExtendedLba,
    /// ISO9660 filesystem and Hidden NTFS
    Iso9660,
    LinuxSwap,
    LinuxNative,
    LinuxLvm,
    LinuxRaid,
    ProtectiveMbr,
    EfiSystemPartition,
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
    /// Starting LBA (sector number).
    pub start_lba: u32,
    /// Number of sectors in this partition.
    pub sector_count: u32,
}

impl Default for MbrPartition {
    fn default() -> Self {
        Self {
            boot_indicator: 0x00,
            start_chs: Chs::new(0),
            part_type: 0x00,
            end_chs: Chs::new(0),
            start_lba: 0,
            sector_count: 0,
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
            start_lba,
            sector_count,
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
        if self.sector_count == 0 {
            self.start_lba
        } else {
            self.start_lba + self.sector_count - 1
        }
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
        start_lba: 0,
        sector_count: 0,
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

// I/O operations
#[cfg(feature = "read")]
impl MasterBootRecord {
    /// Reads an MBR from the beginning of a reader.
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails or if the MBR signature is invalid.
    pub fn read_from<R: hadris_io::Read>(reader: &mut R) -> crate::error::Result<Self> {
        let mut buf = [0u8; 512];
        reader
            .read_exact(&mut buf)
            .map_err(|_| crate::error::PartitionError::Io)?;
        let mbr: Self = bytemuck::cast(buf);
        if !mbr.has_valid_signature() {
            return Err(crate::error::PartitionError::InvalidMbrSignature {
                found: mbr.signature,
            });
        }
        Ok(mbr)
    }
}

#[cfg(feature = "write")]
impl MasterBootRecord {
    /// Writes this MBR to a writer.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_to<W: hadris_io::Write>(&self, writer: &mut W) -> crate::error::Result<()> {
        writer
            .write_all(bytemuck::bytes_of(self))
            .map_err(|_| crate::error::PartitionError::Io)
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
    Reserved0D = 0x0D,
    /// FAT16 partition (using int13 extensions)
    Fat16Lba = 0x0E,
    /// Extended partition (using int13 extensions)
    ExtendedLba = 0x0F,
    Opus = 0x10,
    HiddenFat12 = 0x11,
    CompaqDiagnosis = 0x12,
    Reserved13 = 0x13,
    HiddenFat16S = 0x14,
    Reserved15 = 0x15,
    HiddenFat16L = 0x16,
    /// Hidden IFS (HPFS, NTFS) / ISO9660
    HiddenIfs = 0x17,
    AstWindowsSwap = 0x18,
    WillowtechPhotonCos = 0x19,
    Reserved1A = 0x1A,
    HiddenFat32 = 0x1B,
    HiddenFat32Lba = 0x1C,
    Reserved1D = 0x1D,
    HiddenFat16Lba = 0x1E,
    Reserved1F = 0x1F,
    /// Willowsoft Overture File System
    Ofs1 = 0x20,
    Reserved21 = 0x21,
    OxygenExt = 0x22,
    Reserved23 = 0x23,
    NecMsDos = 0x24,
    Reserved25 = 0x25,
    Reserved26 = 0x26,
    Reserved27 = 0x27,
    Reserved28 = 0x28,
    Reserved29 = 0x29,
    Reserved2A = 0x2A,
    Reserved2B = 0x2B,
    Reserved2C = 0x2C,
    Reserved2D = 0x2D,
    Reserved2E = 0x2E,
    Reserved2F = 0x2F,
    Reserved30 = 0x30,
    Reserved31 = 0x31,
    Reserved32 = 0x32,
    Reserved33 = 0x33,
    Reserved34 = 0x34,
    Reserved35 = 0x35,
    Reserved36 = 0x36,
    Reserved37 = 0x37,
    Theos = 0x38,
    Reserved39 = 0x39,
    Reserved3A = 0x3A,
    Reserved3B = 0x3B,
    PowerQuestFiles = 0x3C,
    HiddenNetWare = 0x3D,
    Reserved3E = 0x3E,
    Reserved3F = 0x3F,
    Venix80286 = 0x40,
    PpcBoot = 0x41,
    SecureFileSystem = 0x42,
    AltExt2Fs = 0x43,
    Reserved44 = 0x44,
    Priam = 0x45,
    EumelElan46 = 0x46,
    EumelElan47 = 0x47,
    EumelElan48 = 0x48,
    Reserved49 = 0x49,
    Alfs = 0x4A,
    Reserved4B = 0x4B,
    Reserved4C = 0x4C,
    Qnx4D = 0x4D,
    Qnx4E = 0x4E,
    Qnx4F = 0x4F,
    OdmReadOnly = 0x50,
    OdmReadWrite = 0x51,
    CPM = 0x52,
    OdmWriteOnly = 0x53,
    Odm6 = 0x54,
    EzDrive = 0x55,
    GoldenBow = 0x56,
    Reserved57 = 0x57,
    Reserved58 = 0x58,
    Reserved59 = 0x59,
    Reserved5A = 0x5A,
    Reserved5B = 0x5B,
    PriamEDisk = 0x5C,
    Reserved5D = 0x5D,
    Reserved5E = 0x5E,
    Reserved5F = 0x5F,
    Reserved60 = 0x60,
    StorageDimension1 = 0x61,
    Reserved62 = 0x62,
    GnuHurd = 0x63,
    NovellNetware286 = 0x64,
    NovellNetware311 = 0x65,
    NovellNetware386 = 0x66,
    NovellNetware67 = 0x67,
    NovellNetware68 = 0x68,
    NovellNetware5 = 0x69,
    Reserved6A = 0x6A,
    Reserved6B = 0x6B,
    Reserved6C = 0x6C,
    Reserved6D = 0x6D,
    Reserved6E = 0x6E,
    Reserved6F = 0x6F,
    DiskSecureMultiBoot = 0x70,
    Reserved71 = 0x71,
    Reserved72 = 0x72,
    Reserved73 = 0x73,
    Reserved74 = 0x74,
    IbmPcIx = 0x75,
    Reserved76 = 0x76,
    Reserved77 = 0x77,
    Reserved78 = 0x78,
    Reserved79 = 0x79,
    Reserved7A = 0x7A,
    Reserved7B = 0x7B,
    Reserved7C = 0x7C,
    Reserved7D = 0x7D,
    Reserved7E = 0x7E,
    Reserved7F = 0x7F,
    OldMinix = 0x80,
    LinuxMinix = 0x81,
    /// Linux Swap partition
    LinuxSwap = 0x82,
    /// Linux native file systems (ext2/3/4, etc.)
    LinuxNative = 0x83,
    Os2Hidden = 0x84,
    LinuxExtended = 0x85,
    NtStripeSet = 0x86,
    HpfsFtMirrored = 0x87,
    Reserved88 = 0x88,
    Reserved89 = 0x89,
    Reserved8A = 0x8A,
    Reserved8B = 0x8B,
    Reserved8C = 0x8C,
    Reserved8D = 0x8D,
    /// Linux LVM
    LinuxLvm = 0x8E,
    Reserved8F = 0x8F,
    Reserved90 = 0x90,
    Reserved91 = 0x91,
    Reserved92 = 0x92,
    HiddenLinuxNative = 0x93,
    AmoebaBadBlockTable = 0x94,
    Reserved95 = 0x95,
    Reserved96 = 0x96,
    Reserved97 = 0x97,
    Reserved98 = 0x98,
    Mylex = 0x99,
    Reserved9A = 0x9A,
    Reserved9B = 0x9B,
    Reserved9C = 0x9C,
    Reserved9D = 0x9D,
    Reserved9E = 0x9E,
    Bsdi = 0x9F,
    IbmHibernation = 0xA0,
    HpVolumeExpA1 = 0xA1,
    ReservedA2 = 0xA2,
    HpVolumeExpA3 = 0xA3,
    HpVolumeExpA4 = 0xA4,
    FreeBsd386 = 0xA5,
    OpenBsd = 0xA6,
    HpVolumeExpA7 = 0xA7,
    MacOsX = 0xA8,
    NetBsd = 0xA9,
    Olivetti = 0xAA,
    MacOsXBoot = 0xAB,
    ReservedAC = 0xAC,
    ReservedAD = 0xAD,
    ReservedAE = 0xAE,
    MacOsXHfsPlus = 0xAF,
    BootMngrBootStar = 0xB0,
    HpVolumeExpB1 = 0xB1,
    HpVolumeExpB2 = 0xB2,
    HpVolumeExpB3 = 0xB3,
    HpVolumeExpB4 = 0xB4,
    ReservedB5 = 0xB5,
    HpVolumeExpB6 = 0xB6,
    BsdiFs = 0xB7,
    BsdiSwap = 0xB8,
    ReservedB9 = 0xB9,
    ReservedBA = 0xBA,
    PtsBootWizard = 0xBB,
    AcronisBackup = 0xBC,
    ReservedBD = 0xBD,
    SolarisBoot = 0xBE,
    Solaris = 0xBF,
    NovellDos = 0xC0,
    DrDos12 = 0xC1,
    ReservedC2 = 0xC2,
    ReservedC3 = 0xC3,
    DrDos16 = 0xC4,
    ReservedC5 = 0xC5,
    DrDosHuge = 0xC6,
    HpfsFtMirroredDisabled = 0xC7,
    ReservedC8 = 0xC8,
    ReservedC9 = 0xC9,
    ReservedCA = 0xCA,
    ReservedCB = 0xCB,
    ReservedCC = 0xCC,
    ReservedCD = 0xCD,
    ReservedCE = 0xCE,
    ReservedCF = 0xCF,
    MultiuserDos = 0xD0,
    OldMultiuserDos = 0xD1,
    ReservedD2 = 0xD2,
    ReservedD3 = 0xD3,
    OldMultiuserDos2 = 0xD4,
    OldMultiuserDos3 = 0xD5,
    OldMultiuserDos4 = 0xD6,
    ReservedD7 = 0xD7,
    Cpm86 = 0xD8,
    ReservedD9 = 0xD9,
    ReservedDA = 0xDA,
    Cpm = 0xDB,
    ReservedDC = 0xDC,
    ReservedDD = 0xDD,
    Dell = 0xDE,
    Embrm = 0xDF,
    ReservedE0 = 0xE0,
    SpeedStorFat12Ext = 0xE1,
    DosReadOnly = 0xE2,
    SpeedStor = 0xE3,
    SpeedStor16Ext = 0xE4,
    ReservedE5 = 0xE5,
    StorageDimension2 = 0xE6,
    ReservedE7 = 0xE7,
    ReservedE8 = 0xE8,
    ReservedE9 = 0xE9,
    ReservedEA = 0xEA,
    BeOs = 0xEB,
    ReservedEC = 0xEC,
    ReservedED = 0xED,
    /// GPT Protective MBR
    GptProtectiveMbr = 0xEE,
    /// EFI System Partition
    EfiSystemPartition = 0xEF,
    ReservedF0 = 0xF0,
    SpeedStorDimensions = 0xF1,
    UnisysDos = 0xF2,
    StorageDimension3 = 0xF3,
    SpeedStorDimensions2 = 0xF4,
    Prolugue = 0xF5,
    StorageDimension4 = 0xF6,
    ReservedF7 = 0xF7,
    ReservedF8 = 0xF8,
    ReservedF9 = 0xF9,
    ReservedFA = 0xFA,
    ReservedFB = 0xFB,
    ReservedFC = 0xFC,
    /// Linux RAID
    LinuxRaid = 0xFD,
    LanStep = 0xFE,
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
        assert_eq!(pt[0].start_lba, 1);
        assert_eq!(pt[0].sector_count, 999);
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
