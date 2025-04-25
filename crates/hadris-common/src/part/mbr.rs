// FIXME: Use hadris_io instead std::io
use std::{
    fmt::Debug,
    io::{Error, Read},
    ops::{Index, IndexMut},
};

use crate::types::{
    endian::{Endian, LittleEndian},
    number::U32,
};

#[derive(Debug, Clone, Copy)]
pub enum MbrPartitionType {
    Empty,
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
    ProtectiveMbr,
    EfiSystemPartition,
    Unknown(u8),
}

impl MbrPartitionType {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0x00 => Self::Empty,
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
            0xEE => Self::ProtectiveMbr,
            0xEF => Self::EfiSystemPartition,
            _ => Self::Unknown(value),
        }
    }

    pub fn to_u8(&self) -> u8 {
        match self {
            Self::Empty => 0x00,
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
            Self::ProtectiveMbr => 0xEE,
            Self::EfiSystemPartition => 0xEF,
            Self::Unknown(value) => *value,
        }
    }
}

/// A 3-byte representation of a CHS address
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct Chs([u8; 3]);

impl Debug for Chs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cylinder = self.cylinder();
        let head = self.head();
        let sector = self.sector();

        f.debug_struct("Chs")
            .field("c", &cylinder)
            .field("h", &head)
            .field("s", &sector)
            .finish()
    }
}

impl Chs {
    pub const OUT_OF_RANGE: Chs = Chs([0xFF, 0xFF, 0xFF]);
    const SECTORS_PER_TRACK: u32 = 63;
    const HEADS_PER_CYLINDER: u32 = 255;

    /// Creates a new CHS value from the LBA (512 block size)
    pub const fn new(lba: u32) -> Self {
        let cylinder = lba / (Self::SECTORS_PER_TRACK * Self::HEADS_PER_CYLINDER);
        if cylinder > 0x03FF {
            return Self([0xFF, 0xFF, 0xFF]);
        }
        let tmp = lba % (Self::SECTORS_PER_TRACK * Self::HEADS_PER_CYLINDER);
        let head = tmp / Self::SECTORS_PER_TRACK;
        let sector = tmp % Self::SECTORS_PER_TRACK + 1;
        assert!(
            sector <= 0b00111111,
            "Sector overflow, this should never happen, please report this bug"
        );
        Self([
            (head & 0x00ff) as u8,
            (sector & 0b00111111) as u8 | ((cylinder & 0x0300) >> 2) as u8,
            (cylinder & 0xFF) as u8,
        ])
    }

    pub fn head(&self) -> u8 {
        self.0[0]
    }

    pub fn sector(&self) -> u8 {
        self.0[1] & 0b00111111
    }

    pub fn cylinder(&self) -> u16 {
        ((self.0[1] as u16 & 0b11000000) << 2) | (self.0[2] as u16)
    }

    pub fn as_lba(&self) -> u32 {
        if self.0 == [0xFF, 0xFF, 0xFF] {
            return u32::MAX;
        }

        self.cylinder() as u32 * Self::SECTORS_PER_TRACK * Self::HEADS_PER_CYLINDER
            + self.head() as u32 * Self::SECTORS_PER_TRACK
            + self.sector() as u32
            - 1
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct MbrPartition {
    pub boot_indicator: u8,
    pub start_head: Chs,
    pub part_type: u8,
    pub end_head: Chs,
    pub start_sector: U32<LittleEndian>,
    pub block_count: U32<LittleEndian>,
}

impl Default for MbrPartition {
    fn default() -> Self {
        // TODO: Maybe we can make some sort of U24 type?
        Self {
            boot_indicator: 0x00,
            start_head: Chs::new(0x00),
            part_type: 0x00,
            end_head: Chs::new(0x00),
            start_sector: U32::new(0x00),
            block_count: U32::new(0x00),
        }
    }
}

impl MbrPartition {
    /// Returns whether the partition is empty
    ///
    /// This currently only checks for the partition type, and does not check for any other
    /// properties. This is technically not spec compliant, but we want to account for user
    /// error and not have a corrupted partition.
    pub fn is_empty(&self) -> bool {
        // We try to leniant when checking for empty partitions, because we rather miss some
        // partitions than have a corrupt one
        self.part_type == 0x00
    }
}

#[repr(transparent)]
#[derive(Clone, Copy)]
#[cfg_attr(feature = "bytemuck", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct MbrPartitionTable {
    pub partitions: [MbrPartition; 4],
}

impl Default for MbrPartitionTable {
    fn default() -> Self {
        Self {
            partitions: [MbrPartition::default(); 4],
        }
    }
}

impl core::fmt::Debug for MbrPartitionTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MbrPartitionTable")
            .field("partitions", &&self.partitions[..self.len()])
            .finish()
    }
}

impl MbrPartitionTable {
    pub fn parse<T: Read>(reader: &mut T) -> Result<Self, Error> {
        let mut buf = [0u8; size_of::<Self>()];
        reader.read_exact(&mut buf)?;
        Ok(bytemuck::cast(buf))
    }

    pub fn len(&self) -> usize {
        let mut count = 0;
        for partition in self.partitions {
            if partition.is_empty() {
                break;
            }
            count += 1;
        }
        count
    }

    pub fn is_valid(&self) -> bool {
        // FIXME: Implement a more robust validation

        let mut empty = false;
        for partition in self.partitions {
            // Boot indicator is not 0x00, or 0x80
            if (partition.boot_indicator & !0x80) != 0 {
                return false;
            }
            let is_empty = partition.is_empty();
            if empty && !is_empty {
                // Bytes exist after an empty entry
                return false;
            }
            if is_empty {
                empty = true;
            }
        }
        true
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

/// An enum representing the full list of partition types.
///
/// Available at https://thestarman.pcministry.com/asm/mbr/PartTypes.htm
/// For a more useful struct, use [`MbrPartitionType`].
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
    /// OS/2 Boot Manager partition and coherent swap partition
    Os2Boot = 0x0a,
    /// FAT32 partition
    Fat32 = 0x0b,
    /// FAT32 partition (using int13 extensions)
    Fat32Bios = 0x0c,
    Reserved0D = 0x0d,
    /// FAT16 partition (using int13 extensions)
    Fat16Bios = 0x0e,
    /// Extended partition (using int13 extensions)
    ExtendedLBA = 0x0f,
    Opus = 0x10,
    HiddenFat12 = 0x11,
    CompaqDiagnosis = 0x12,
    Reserved13 = 0x13,
    HiddenFat16S = 0x14,
    Reserved15 = 0x15,
    HiddenFat16L = 0x16,
    /// Hidden IFS (HPFS, NTFS)
    HiddenIfs = 0x17,
    /// AST Windows Swap file
    AstWindowsSwap = 0x18,
    /// Willowtech Photon coS
    WillowtechPhotonCos = 0x19,
    Reserved1A = 0x1a,
    HiddenFat32 = 0x1b,
    HiddenFat32Bios = 0x1c,
    Reserved1D = 0x1d,
    HiddenFat16Bios = 0x1e,
    Reserved1F = 0x1f,
    /// Willowsoft Overture File System
    Ofs1 = 0x20,
    /// Reserved (HP Volume Expansion, SpeedStor variant) Oxygen FSo2
    Reserved21 = 0x21,
    /// Oxygen Extended File System
    OxygenExt = 0x22,
    /// Reserved (HP Volume Expansion, SpeedStor variant?)
    Reserved23 = 0x23,
    /// NEC MS-DOS 3.x
    NecMsDos = 0x24,
    Reserved25 = 0x25,
    /// Reserved (HP Volume Expansion, SpeedStor variant?)
    Reserved26 = 0x26,
    Reserved27 = 0x27,
    Reserved28 = 0x28,
    Reserved29 = 0x29,
    Reserved2A = 0x2a,
    Reserved2B = 0x2b,
    Reserved2C = 0x2c,
    Reserved2D = 0x2d,
    Reserved2E = 0x2e,
    Reserved2F = 0x2f,
    Reserved30 = 0x30,
    /// Reserved (HP Volume Expansion, SpeedStor variant?)
    Reserved31 = 0x31,
    Reserved32 = 0x32,
    /// Reserved (HP Volume Expansion, SpeedStor variant?)
    Reserved33 = 0x33,
    /// Reserved (HP Volume Expansion, SpeedStor variant?)
    Reserved34 = 0x34,
    Reserved35 = 0x35,
    /// Reserved (HP Volume Expansion, SpeedStor variant?)
    Reserved36 = 0x36,
    Reserved37 = 0x37,
    Theos = 0x38,
    Reserved39 = 0x39,
    Reserved3A = 0x3a,
    Reserved3B = 0x3b,
    /// PowerQuest Files Partition Format
    PowerQuestFiles = 0x3c,
    /// Hidden NetWare
    HiddenNetWare = 0x3d,
    Reserved3E = 0x3e,
    Reserved3F = 0x3f,
    /// Venix 80286 partition
    Venix80286 = 0x40,
    /// PowerPC Boot Partition and Personal RISC Boot
    /// PTS-DOS 6.70 & BootWizard: Alternative Linux, Minix, DR-DOS
    PpcBoot = 0x41,
    /// Secure file system
    /// Windows 2000/XP: Dynamic Extended Partition
    /// PTS-DOS 6.70 & BootWizard: Alternative Linux swap, DR-DOS
    SecureFileSystem = 0x42,
    /// Alternative Linux native file system (EXT2fs)
    /// PTS-DOS 6.70 & BootWizard: DR-DOS
    AltExt2Fs = 0x43,
    Reserved44 = 0x44,
    /// Priam, EUMEL/Elan
    Priam = 0x45,
    EumelElan46 = 0x46,
    EumelElan47 = 0x47,
    EumelElan48 = 0x48,
    Reserved49 = 0x49,
    /// ALFS/THIN lightweight filesystem for DOS
    Alfs = 0x4a,
    Reserved4B = 0x4b,
    Reserved4C = 0x4c,
    Qnx4D = 0x4d,
    Qnx4E = 0x4e,
    /// QNX, Oberon boot/data partition
    Qnx4F = 0x4f,
    /// Ontrack Disk Manager, read-only partition, FAT partition (Logical sector size varies)
    OdmReadOnly = 0x50,
    /// Ontrack Disk Manager, read/write partition, FAT partition (Logical sector size varies), Novell ?
    OdmReadWrite = 0x51,
    /// CP/M partition, Microport System V/386 partition
    CPM = 0x52,
    /// Ontrakck Disk Manager, write-only partition
    OdmWriteOnly = 0x53,
    /// Ontrack Disk Manager 6.0
    Odm6 = 0x54,
    /// EZ-Drive 3.05
    EzDrive = 0x55,
    /// GoldenBow VFeature
    GoldenBow = 0x56,
    Reserved57 = 0x57,
    Reserved58 = 0x58,
    Reserved59 = 0x59,
    Reserved5A = 0x5a,
    Reserved5B = 0x5b,
    /// Priam EDISK
    PriamEDisk = 0x5c,
    Reserved5D = 0x5d,
    Reserved5E = 0x5e,
    Reserved5F = 0x5f,
    Reserved60 = 0x60,
    /// Storage Dimensions SpeedStor
    StorageDimension1 = 0x61,
    Reserved62 = 0x62,
    /// GNU HURD, Mach, MtXinu BSD 4.2 on Mach, Unix Sys V/386, 386/ix.
    GnuHurd = 0x63,
    /// Novell NetWare 286, SpeedStore.
    NovellNetware286 = 0x64,
    /// Novell NetWare (3.11 and 4.1)
    NovellNetware311 = 0x65,
    /// Novell NetWare 386
    NovellNetware386 = 0x66,
    NovellNetware67 = 0x67,
    NovellNetware68 = 0x68,
    /// Novell NetWare 5.0+, Novell Storage Services (NSS)
    NovellNetware5 = 0x69,
    Reserved6A = 0x6a,
    Reserved6B = 0x6b,
    Reserved6C = 0x6c,
    Reserved6D = 0x6d,
    Reserved6E = 0x6e,
    Reserved6F = 0x6f,
    /// DiskSecure Multi-Boot
    DiskSecureMultiBoot = 0x70,
    Reserved71 = 0x71,
    Reserved72 = 0x72,
    Reserved73 = 0x73,
    Reserved74 = 0x74,
    /// IBM PC/IX
    IbmPcIx = 0x75,
    Reserved76 = 0x76,
    Reserved77 = 0x77,
    Reserved78 = 0x78,
    Reserved79 = 0x79,
    Reserved7A = 0x7a,
    Reserved7B = 0x7b,
    Reserved7C = 0x7c,
    Reserved7D = 0x7d,
    Reserved7E = 0x7e,
    Reserved7F = 0x7f,
    /// Minix v1.1 - 1.4a, Old MINIX (Linux).
    OldMinix = 0x80,
    /// Linux/Minix v1.4b+, Mitac Advanced Disk Manager.
    LinuxMinix = 0x81,
    /// Linux Swap partition, Prime or Solaris (Unix).
    LinuxSwap = 0x82,
    /// Linux native file systems (ext2/3/4, JFS, Reiser, xiafs, and others).
    LinuxNative = 0x83,
    /// OS/2 hiding type 04h partition;
    /// Win98: APM Hibernation
    Os2Hidden = 0x84,
    Reserved85 = 0x85,
    /// NT Stripe Set, Volume Set?
    NtStripeSet = 0x86,
    /// NT Stripe Set, Volume Set?, HPFS FT mirrored partition.
    HpfsFtMirrored = 0x87,
    Reserved88 = 0x88,
    Reserved89 = 0x89,
    Reserved8A = 0x8a,
    Reserved8B = 0x8b,
    Reserved8C = 0x8c,
    Reserved8D = 0x8d,
    Reserved8E = 0x8e,
    Reserved8F = 0x8f,
    Reserved90 = 0x90,
    Reserved91 = 0x91,
    Reserved92 = 0x92,
    /// Amoeba file system, Hidden Linux EXT2 partition (PowerQuest).
    HiddenLinuxNative = 0x93,
    AmoebaBadBlockTable = 0x94,
    Reserved95 = 0x95,
    Reserved96 = 0x96,
    Reserved97 = 0x97,
    Reserved98 = 0x98,
    /// Mylex EISA SCSI
    Mylex = 0x99,
    Reserved9A = 0x9a,
    Reserved9B = 0x9b,
    Reserved9C = 0x9c,
    Reserved9D = 0x9d,
    Reserved9E = 0x9e,
    /// BSDI
    Bsdi = 0x9f,
    /// Phoenix NoteBios Power Management "Save to Disk", IBM hibernation.
    IbmHibernation = 0xa0,
    /// HP Volume Expansion (SpeedStor variant)
    HpVolumeExpA1 = 0xa1,
    ReservedA2 = 0xa2,
    /// HP Volume Expansion (SpeedStor variant)
    HpVolumeExpA3 = 0xa3,
    /// HP Volume Expansion (SpeedStor variant)
    HpVolumeExpA4 = 0xa4,
    /// FreeBSD/386
    FreeBsd386 = 0xa5,
    OpenBsd = 0xa6,
    /// HP Volume Expansion (SpeedStor variant)
    /// NetStep partition
    HpVolumeExpA7 = 0xa7,
    ReservedA8 = 0xa8,
    Netbsd = 0xa9,
    /// Olivetti DOS with Fat12
    Olivetti = 0xaa,
    ReservedAB = 0xab,
    ReservedAC = 0xac,
    ReservedAD = 0xad,
    ReservedAE = 0xae,
    ReservedAF = 0xaf,
    /// Bootmanager BootStar by Star-Tools GmbH
    BootMngrBootStar = 0xb0,
    HpVolumeExpB1 = 0xb1,
    HpVolumeExpB2 = 0xb2,
    HpVolumeExpB3 = 0xb3,
    HpVolumeExpB4 = 0xb4,
    ReservedB5 = 0xb5,
    HpVolumeExpB6 = 0xb6,
    /// BSDI file system or secondarily swap
    BsdiFs = 0xb7,
    /// BSDI swap partition or secondarily file system
    BsdiSwap = 0xb8,
    ReservedB9 = 0xb9,
    ReservedBA = 0xba,
    /// PTS BootWizard (hidden) 4.0; but now also used by Acronis OS Selector to hide or create some partitions.
    PtsBootWizard = 0xbb,
    /// May be an Acronis 'Backup' or 'Secure Zone' partition, when labeled 'ACRONIS SZ' (FAT32, LBA mapped, primary).
    AcronisBackup = 0xbc,
    ReservedBD = 0xbd,
    SolarisBoot = 0xbe,
    ReservedBF = 0xbf,
    /// Novell DOS/OpenDOS/DR-OpenDOS/DR-DOS secured partition, or CTOS (reported by a client).
    NovellDos = 0xc0,
    /// DR-DOS 6.0 LOGIN.EXE-secured 12-bit FAT partition
    DrDos12 = 0xc1,
    /// Reserved for DR-DOS 7+
    ReservedC2 = 0xc2,
    /// Reserved for DR-DOS 7+
    ReservedC3 = 0xc3,
    /// DR-DOS 6.0 LOGIN.EXE-secured 16-bit FAT partition
    DrDos16 = 0xc4,
    ReservedC5 = 0xc5,
    /// DR-DOS 6.0 LOGIN.EXE-secured Huge partition, or Corrupted FAT16 volume/stripe (V/S) set (Windows NT).
    DrDosHuge = 0xc6,
    /// Syrinx, Cyrnix, HPFS FT disabled mirrored partition, or Corrupted NTFS volume/stripe set.
    HpfsFtMirroredDisabled = 0xc7,
    /// Reserved for DR-DOS 7+
    ReservedC8 = 0xc8,
    /// Reserved for DR-DOS 7+
    ReservedC9 = 0xc9,
    /// Reserved for DR-DOS 7+
    ReservedCA = 0xca,
    /// Reserved for DR-DOS secured FAT-32
    ReservedCB = 0xcb,
    /// Reserved for DR-DOS secured FAT-32 LBA
    ReservedCC = 0xcc,
    /// Reserved for DR-DOS 7+
    ReservedCD = 0xcd,
    /// Reserved for DR-DOS secured FAT-16
    ReservedCE = 0xce,
    /// Reserved for DR-DOS secured FAT-16 LBA
    ReservedCF = 0xcf,
    /// Multiuser DOS secured (FAT12???)
    MultiuserDos = 0xd0,
    /// Old Multiuser DOS secured FAT12
    OldMultiuserDos = 0xd1,
    ReservedD2 = 0xd2,
    ReservedD3 = 0xd3,
    /// Old Multiuser DOS secured FAT16 (<= 32M)
    OldMultiuserDos2 = 0xd4,
    /// Old Multiuser DOS secured extended partition
    OldMultiuserDos3 = 0xd5,
    /// Old Multiuser DOS secured FAT16 (BIGDOS > 32 Mb)
    OldMultiuserDos4 = 0xd6,
    ReservedD7 = 0xd7,
    /// CP/M 86
    Cpm86 = 0xd8,
    ReservedD9 = 0xd9,
    ReservedDA = 0xda,
    /// CP/M, Concurrent CP/M, Concurrent DOS, or CTOS (Convergent Technologies OS)
    Cpm = 0xdb,
    ReservedDC = 0xdc,
    ReservedDD = 0xdd,
    /// Dell partition, normally a 32MB FAT16 partition
    Dell = 0xde,
    /// BootIt EMBRM
    Embrm = 0xdf,
    ReservedE0 = 0xe0,
    /// SpeedStor 12-bit FAT Extended partition, DOS access (Linux).
    SpeedStorFat12Ext = 0xe1,
    /// DOS read-only (Florian Painke's XFDISK 1.0.4)
    DosReadOnly = 0xe2,
    /// SpeedStor (Norton, Linux says DOS R/O)
    SpeedStor = 0xe3,
    /// SpeedStor 16-bit FAT Extended partition
    SpeedStor16Ext = 0xe4,
    /// Tandy DOS with logical sectored FAT
    ReservedE5 = 0xe5,
    StorageDimension2 = 0xe6,
    ReservedE7 = 0xe7,
    ReservedE8 = 0xe8,
    ReservedE9 = 0xe9,
    ReservedEA = 0xea,
    /// BeOS file system
    BeOs = 0xeb,
    ReservedEC = 0xec,
    ReservedED = 0xed,
    /// Protective MBR
    GptProtectiveMbr = 0xee,
    /// EFI System Partition
    EfiSystemPartition = 0xef,
    ReservedF0 = 0xf0,
    /// SpeedStor Dimensions (Norton,Landis)
    SpeedStorDimensions = 0xf1,
    /// DOS 3.3+ second partition, Unisys DOS with logical sectored FAT.
    UnisysDos = 0xf2,
    /// Storage Dimensions SpeedStor
    StorageDimension3 = 0xf3,
    /// SpeedStor Storage Dimensions (Norton,Landis)
    SpeedStorDimensions2 = 0xf4,
    Prolugue = 0xf5,
    StorageDimension4 = 0xf6,
    ReservedF7 = 0xf7,
    ReservedF8 = 0xf8,
    ReservedF9 = 0xf9,
    ReservedFA = 0xfa,
    ReservedFB = 0xfb,
    ReservedFC = 0xfc,
    /// Reserved for FreeDOS
    FreeDosReserved = 0xfd,
    /// LANstep, IBM PS/2 IML (Initial Microcode Load) partition, or...
    LanStep = 0xfe,
    /// BadBlockTable
    /// Currently only used by Xenix
    BadBlockTable = 0xff,
}

impl MbrPartitionTypeFull {
    /// Create a new [`MbrPartitionTypeFull`] from a u8.
    pub fn from_u8(value: u8) -> Self {
        // SAFETY: This is safe because all the variants in an u8 are defined
        unsafe { std::mem::transmute(value) }
    }

    pub fn to_u8(&self) -> u8 {
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
        assert_eq!(Chs::new(63 * 255 * 1024), Chs([0xFF, 0xFF, 0xFF]));
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
        assert_eq!(Chs([0xFF, 0xFF, 0xFF]).as_lba(), u32::MAX);
    }
}
