#![cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]

use core::convert::Infallible;

use hadris_fs::Error;

/// What a device holds: a filesystem, a partition table or an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImageFormat {
    /// A FAT12, FAT16 or FAT32 volume.
    Fat(crate::fat::FatKind),
    /// An exFAT volume.
    ExFat,
    /// An ISO 9660 volume.
    Iso,
    /// A UDF volume.
    Udf,
    /// ISO 9660 and UDF volumes describing the same files.
    IsoUdfBridge,
    /// A cpio archive.
    Cpio(crate::cpio::Format),
    /// An MBR partition table.
    Mbr,
    /// A GUID partition table.
    Gpt,
    /// An NTFS volume. `open` does not mount it in 3.0.
    Ntfs,
}

/// One format `detect` found, with the error its mount would give when
/// its signature is present but its first structures are damaged.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    format: ImageFormat,
    damage: Option<Error<Infallible>>,
}

impl Candidate {
    /// The format.
    pub const fn format(&self) -> ImageFormat {
        self.format
    }

    /// `None` when the structures a mount reads first are sound, else the
    /// error mount gives (kind `Corrupt`, with the format's detail code),
    /// so a damaged volume never reads as another format.
    pub const fn damage(&self) -> Option<&Error<Infallible>> {
        self.damage.as_ref()
    }
}

/// The most formats one device can show at once: a bridge image's three,
/// then a hybrid table's two.
const MAX_FOUND: usize = 6;

/// Every format `detect` found, most specific first: a bridge image is
/// `IsoUdfBridge`, then `Iso`, then `Udf`, and a hybrid ISO image is `Iso`
/// then `Gpt` or `Mbr`. Empty when nothing was recognized. It does not
/// allocate.
#[derive(Debug, Clone, Copy)]
pub struct Detection {
    found: [Option<Candidate>; MAX_FOUND],
}

impl Detection {
    pub(crate) const fn new() -> Self {
        Self {
            found: [None; MAX_FOUND],
        }
    }

    pub(crate) fn push(&mut self, format: ImageFormat, damage: Option<Error<Infallible>>) {
        if let Some(slot) = self.found.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(Candidate { format, damage });
        }
    }

    /// The most specific format found.
    pub fn first(&self) -> Option<&Candidate> {
        self.found[0].as_ref()
    }

    /// Every format found, most specific first.
    pub fn iter(&self) -> impl Iterator<Item = &Candidate> {
        self.found.iter().flatten()
    }
}

/// The kind of partition table the MBR entries of sector 0 describe:
/// protective, ordinary, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Table {
    Mbr,
    Gpt,
    Hybrid,
}

pub(crate) fn partition_table(sector: &[u8; 512]) -> Option<Table> {
    if sector[510..512] != [0x55, 0xAA] {
        return None;
    }
    let mut protective = false;
    let mut ordinary = false;
    for entry in sector[446..510].chunks_exact(16) {
        if !matches!(entry[0], 0x00 | 0x80) {
            return None;
        }
        let kind = entry[4];
        let sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);
        if kind == 0 || sectors == 0 {
            continue;
        }
        protective |= kind == 0xEE;
        ordinary |= kind != 0xEE;
    }
    match (protective, ordinary) {
        (true, true) => Some(Table::Hybrid),
        (true, false) => Some(Table::Gpt),
        (false, true) => Some(Table::Mbr),
        (false, false) => None,
    }
}

/// The FAT variant of a boot sector: from the cluster count of a
/// plausible BIOS parameter block, else from the file system type label,
/// so a damaged boot sector still reads as FAT.
pub(crate) fn fat_kind(sector: &[u8; 512]) -> Option<crate::fat::FatKind> {
    use crate::fat::FatKind;
    if sector[510..512] != [0x55, 0xAA] {
        return None;
    }
    if let Some(kind) = fat_bpb(sector) {
        return Some(kind);
    }
    match (&sector[54..62], &sector[82..90]) {
        (_, b"FAT32   ") => Some(FatKind::Fat32),
        (b"FAT16   ", _) => Some(FatKind::Fat16),
        (b"FAT12   ", _) => Some(FatKind::Fat12),
        _ => None,
    }
}

fn fat_bpb(sector: &[u8; 512]) -> Option<crate::fat::FatKind> {
    use crate::fat::FatKind;
    let word = |at: usize| u32::from(u16::from_le_bytes([sector[at], sector[at + 1]]));
    let long = |at: usize| {
        u32::from_le_bytes([sector[at], sector[at + 1], sector[at + 2], sector[at + 3]])
    };
    let bytes = word(11);
    let per_cluster = u32::from(sector[13]);
    let reserved = word(14);
    let fats = u32::from(sector[16]);
    if !matches!(bytes, 512 | 1024 | 2048 | 4096)
        || !per_cluster.is_power_of_two()
        || reserved == 0
        || fats == 0
    {
        return None;
    }
    let total = if word(19) != 0 { word(19) } else { long(32) };
    let fat = if word(22) != 0 { word(22) } else { long(36) };
    let root = (word(17) * 32).div_ceil(bytes);
    let used = reserved
        .checked_add(fats.checked_mul(fat)?)?
        .checked_add(root)?;
    Some(match total.checked_sub(used)? / per_cluster {
        0..4085 => FatKind::Fat12,
        4085..65525 => FatKind::Fat16,
        _ => FatKind::Fat32,
    })
}

/// The cpio format whose magic starts `head`.
pub(crate) fn cpio_format(head: &[u8; 512]) -> Option<crate::cpio::Format> {
    use crate::cpio::Format;
    match &head[..6] {
        b"070701" => Some(Format::Newc),
        b"070702" => Some(Format::Crc),
        b"070707" => Some(Format::Odc),
        _ if head[..2] == [0xC7, 0x71] || head[..2] == [0x71, 0xC7] => Some(Format::Binary),
        _ => None,
    }
}

/// What the volume recognition area of an optical image holds.
#[derive(Default)]
pub(crate) struct Optical {
    pub(crate) iso: bool,
    pub(crate) udf: bool,
    extended: bool,
    nsr: bool,
}

impl Optical {
    /// The first sector of the recognition area, in 2048-byte sectors.
    pub(crate) const FIRST: u64 = 16;
    /// Sectors of the recognition area read.
    pub(crate) const SECTORS: u64 = 16;

    /// Takes the first 7 bytes of the next volume structure descriptor.
    pub(crate) fn inspect(&mut self, head: &[u8; 7]) {
        if head[6] != 1 {
            return;
        }
        match &head[1..6] {
            b"CD001" => self.iso = true,
            b"BEA01" if head[0] == 0 => {
                self.extended = true;
                self.nsr = false;
            }
            b"NSR02" | b"NSR03" if head[0] == 0 && self.extended => self.nsr = true,
            b"TEA01" if head[0] == 0 && self.nsr => self.udf = true,
            _ => {}
        }
    }
}
