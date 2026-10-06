use core::fmt;

/// An MBR partition type code.
///
/// Every byte is a valid code; the associated constants name the common
/// ones. [`Display`](fmt::Display) prints the name of a known code and
/// `0xNN` otherwise.
///
/// ```rust
/// use hadris_part::MbrType;
///
/// assert_eq!(MbrType::new(0x83), MbrType::LINUX);
/// assert!(MbrType::EXTENDED_LBA.is_extended());
/// assert_eq!(MbrType::new(0x42).to_string(), "0x42");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct MbrType(u8);

impl MbrType {
    /// An unused entry.
    pub const EMPTY: Self = Self(0x00);
    /// FAT12.
    pub const FAT12: Self = Self(0x01);
    /// FAT16 below 32 MiB.
    pub const FAT16_SMALL: Self = Self(0x04);
    /// Extended partition, CHS addressed.
    pub const EXTENDED: Self = Self(0x05);
    /// FAT16 of 32 MiB or more.
    pub const FAT16: Self = Self(0x06);
    /// NTFS, exFAT or HPFS.
    pub const NTFS: Self = Self(0x07);
    /// FAT32, CHS addressed.
    pub const FAT32: Self = Self(0x0B);
    /// FAT32, LBA addressed.
    pub const FAT32_LBA: Self = Self(0x0C);
    /// FAT16, LBA addressed.
    pub const FAT16_LBA: Self = Self(0x0E);
    /// Extended partition, LBA addressed.
    pub const EXTENDED_LBA: Self = Self(0x0F);
    /// Hidden NTFS; also used for ISO 9660 in hybrid images.
    pub const ISO9660: Self = Self(0x17);
    /// Linux swap.
    pub const LINUX_SWAP: Self = Self(0x82);
    /// Linux native filesystem.
    pub const LINUX: Self = Self(0x83);
    /// Linux extended partition.
    pub const LINUX_EXTENDED: Self = Self(0x85);
    /// Linux LVM.
    pub const LINUX_LVM: Self = Self(0x8E);
    /// FreeBSD.
    pub const FREEBSD: Self = Self(0xA5);
    /// OpenBSD.
    pub const OPENBSD: Self = Self(0xA6);
    /// NetBSD.
    pub const NETBSD: Self = Self(0xA9);
    /// Apple HFS and HFS+.
    pub const APPLE_HFS: Self = Self(0xAF);
    /// GPT protective entry.
    pub const GPT_PROTECTIVE: Self = Self(0xEE);
    /// EFI system partition.
    pub const EFI_SYSTEM: Self = Self(0xEF);
    /// Linux RAID autodetect.
    pub const LINUX_RAID: Self = Self(0xFD);

    /// The type with code `code`.
    pub const fn new(code: u8) -> Self {
        Self(code)
    }

    /// The type code.
    pub const fn code(self) -> u8 {
        self.0
    }

    /// Whether the entry is unused.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the code marks an extended partition (`0x05`, `0x0F` or
    /// `0x85`).
    pub const fn is_extended(self) -> bool {
        matches!(self.0, 0x05 | 0x0F | 0x85)
    }

    /// Whether the code marks a GPT protective entry.
    pub const fn is_protective(self) -> bool {
        self.0 == 0xEE
    }

    const fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x00 => "Empty",
            0x01 => "FAT12",
            0x04 => "FAT16 <32M",
            0x05 => "Extended",
            0x06 => "FAT16",
            0x07 => "NTFS/exFAT",
            0x0B => "FAT32",
            0x0C => "FAT32 LBA",
            0x0E => "FAT16 LBA",
            0x0F => "Extended LBA",
            0x17 => "Hidden NTFS/ISO 9660",
            0x82 => "Linux swap",
            0x83 => "Linux",
            0x85 => "Linux extended",
            0x8E => "Linux LVM",
            0xA5 => "FreeBSD",
            0xA6 => "OpenBSD",
            0xA9 => "NetBSD",
            0xAF => "Apple HFS",
            0xEE => "GPT protective",
            0xEF => "EFI System",
            0xFD => "Linux RAID",
            _ => return None,
        })
    }
}

impl From<u8> for MbrType {
    fn from(code: u8) -> Self {
        Self(code)
    }
}

impl From<MbrType> for u8 {
    fn from(kind: MbrType) -> Self {
        kind.0
    }
}

impl fmt::Display for MbrType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "0x{:02X}", self.0),
        }
    }
}

impl fmt::Debug for MbrType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MbrType(0x{:02X})", self.0)
    }
}
