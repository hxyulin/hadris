//! 32-byte directory slots: classification and short-entry fields.

use hadris_common::types::number::{U16, U32};

use crate::dirent::{
    ATTR_DIRECTORY, ATTR_LONG_NAME, ATTR_LONG_NAME_MASK, ATTR_VOLUME_ID, ENTRY_END, ENTRY_FREE,
    LFN_UNITS_PER_ENTRY, RawDirEntry, RawLfnEntry,
};
use crate::entry::FatKind;
use crate::lfn;

/// The most entries one directory may hold.
pub const MAX_DIR_ENTRIES: u32 = 65_536;

/// What a directory slot holds. The four cases are all the specification
/// has, so the enum is exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// No entry here or after.
    End,
    /// A deleted entry.
    Free,
    /// A long-name fragment.
    Long(LongEntry),
    /// A short entry: a file, a directory, `.`/`..` or the volume label.
    Short(ShortEntry),
}

/// One long-name fragment, decoded from a [`RawLfnEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LongEntry {
    sequence: u8,
    checksum: u8,
    name1: [u8; 10],
    name2: [u8; 12],
    name3: [u8; 4],
}

/// The fields of a short entry, decoded from a [`RawDirEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShortEntry {
    name: [u8; 11],
    attr: u8,
    nt_case: u8,
    created_tenths: u8,
    created_time: u16,
    created_date: u16,
    accessed_date: u16,
    cluster_high: u16,
    modified_time: u16,
    modified_date: u16,
    cluster_low: u16,
    size: u32,
}

impl Slot {
    /// Classifies and decodes one 32-byte slot.
    pub fn parse(raw: &[u8; 32]) -> Self {
        match raw[0] {
            ENTRY_END => return Self::End,
            ENTRY_FREE => return Self::Free,
            _ => {}
        }
        if raw[11] & ATTR_LONG_NAME_MASK == ATTR_LONG_NAME {
            let long: RawLfnEntry = bytemuck::cast(*raw);
            return Self::Long(LongEntry {
                sequence: long.sequence,
                checksum: long.checksum,
                name1: long.name1,
                name2: long.name2,
                name3: long.name3,
            });
        }
        let short: RawDirEntry = bytemuck::cast(*raw);
        Self::Short(ShortEntry {
            name: short.name,
            attr: short.attributes,
            nt_case: short.nt_reserved,
            created_tenths: short.created_tenths,
            created_time: short.created_time.get(),
            created_date: short.created_date.get(),
            accessed_date: short.accessed_date.get(),
            cluster_high: short.first_cluster_high.get(),
            modified_time: short.modified_time.get(),
            modified_date: short.modified_date.get(),
            cluster_low: short.first_cluster_low.get(),
            size: short.size.get(),
        })
    }
}

impl LongEntry {
    /// A fragment with sequence byte `sequence` (the entry number, with
    /// [`LFN_LAST_ENTRY`](crate::LFN_LAST_ENTRY) on the first one on disk),
    /// the short name's `checksum` and 13 code units.
    pub fn new(sequence: u8, checksum: u8, units: &[u16; LFN_UNITS_PER_ENTRY]) -> Self {
        let (name1, name2, name3) = lfn::pack(units);
        Self {
            sequence,
            checksum,
            name1,
            name2,
            name3,
        }
    }

    /// `LDIR_Ord`: the entry number, with
    /// [`LFN_LAST_ENTRY`](crate::LFN_LAST_ENTRY) on the first one on disk.
    pub const fn sequence(&self) -> u8 {
        self.sequence
    }

    /// `LDIR_Chksum`: the [`lfn_checksum`](crate::lfn_checksum) of the
    /// short name.
    pub const fn checksum(&self) -> u8 {
        self.checksum
    }

    /// The 13 code units and how many of them precede the `0x0000`
    /// terminator or `0xFFFF` filler.
    pub fn units(&self) -> ([u16; LFN_UNITS_PER_ENTRY], usize) {
        lfn::unpack(&self.name1, &self.name2, &self.name3)
    }

    /// Encodes the fragment as a directory slot.
    pub fn encode(&self) -> [u8; 32] {
        bytemuck::cast(RawLfnEntry {
            sequence: self.sequence,
            name1: self.name1,
            attributes: ATTR_LONG_NAME,
            kind: 0,
            checksum: self.checksum,
            name2: self.name2,
            first_cluster_low: [0; 2],
            name3: self.name3,
        })
    }
}

impl ShortEntry {
    /// An entry with the given stored name and attributes, no times, no
    /// clusters and size 0.
    pub const fn new(name: [u8; 11], attributes: u8) -> Self {
        Self {
            name,
            attr: attributes,
            nt_case: 0,
            created_tenths: 0,
            created_time: 0,
            created_date: 0,
            accessed_date: 0,
            cluster_high: 0,
            modified_time: 0,
            modified_date: 0,
            cluster_low: 0,
            size: 0,
        }
    }

    /// Encodes the entry as a directory slot.
    pub fn encode(&self) -> [u8; 32] {
        bytemuck::cast(RawDirEntry {
            name: self.name,
            attributes: self.attr,
            nt_reserved: self.nt_case,
            created_tenths: self.created_tenths,
            created_time: U16::new(self.created_time),
            created_date: U16::new(self.created_date),
            accessed_date: U16::new(self.accessed_date),
            first_cluster_high: U16::new(self.cluster_high),
            modified_time: U16::new(self.modified_time),
            modified_date: U16::new(self.modified_date),
            first_cluster_low: U16::new(self.cluster_low),
            size: U32::new(self.size),
        })
    }

    /// The 11 name bytes as stored, so a leading `0xE5` reads `0x05`.
    pub const fn name(&self) -> [u8; 11] {
        self.name
    }

    /// Sets the stored name bytes.
    pub const fn set_name(&mut self, name: [u8; 11]) {
        self.name = name;
    }

    /// The checksum each long-name fragment of this entry stores.
    pub const fn lfn_checksum(&self) -> u8 {
        lfn::checksum(&self.name)
    }

    /// `DIR_Attr`.
    pub const fn attributes(&self) -> u8 {
        self.attr
    }

    /// Sets `DIR_Attr`.
    pub const fn set_attributes(&mut self, attributes: u8) {
        self.attr = attributes;
    }

    /// `DIR_NTRes`: the case bits of the base name and extension.
    pub const fn nt_case(&self) -> u8 {
        self.nt_case
    }

    /// Sets `DIR_NTRes`.
    pub const fn set_nt_case(&mut self, bits: u8) {
        self.nt_case = bits;
    }

    /// The creation date, time and 10 ms count.
    pub const fn created(&self) -> (u16, u16, u8) {
        (self.created_date, self.created_time, self.created_tenths)
    }

    /// Sets the creation date, time and 10 ms count.
    pub const fn set_created(&mut self, date: u16, time: u16, tenths: u8) {
        (self.created_date, self.created_time, self.created_tenths) = (date, time, tenths);
    }

    /// The modification date and time.
    pub const fn modified(&self) -> (u16, u16) {
        (self.modified_date, self.modified_time)
    }

    /// Sets the modification date and time.
    pub const fn set_modified(&mut self, date: u16, time: u16) {
        (self.modified_date, self.modified_time) = (date, time);
    }

    /// The access date.
    pub const fn accessed_date(&self) -> u16 {
        self.accessed_date
    }

    /// Sets the access date.
    pub const fn set_accessed_date(&mut self, date: u16) {
        self.accessed_date = date;
    }

    /// The file size in bytes; 0 for a directory.
    pub const fn size(&self) -> u32 {
        self.size
    }

    /// Sets the file size.
    pub const fn set_size(&mut self, size: u32) {
        self.size = size;
    }

    /// Sets the first cluster. FAT12/16 keep whatever the high word held.
    pub const fn set_first_cluster(&mut self, kind: FatKind, cluster: u32) {
        self.cluster_low = cluster as u16;
        if let FatKind::Fat32 = kind {
            self.cluster_high = (cluster >> 16) as u16;
        }
    }

    /// Whether this is a directory.
    pub const fn is_dir(&self) -> bool {
        self.attr & ATTR_DIRECTORY != 0
    }

    /// Whether this is the volume label rather than a file or directory.
    pub const fn is_label(&self) -> bool {
        self.attr & ATTR_VOLUME_ID != 0
    }

    /// Whether this is the `.` or `..` entry of a subdirectory.
    pub const fn is_dot(&self) -> bool {
        self.name[0] == b'.'
    }

    /// Whether this is the `..` entry.
    pub fn is_dot_dot(&self) -> bool {
        self.name == *b"..         "
    }

    /// Whether a listing shows this entry.
    pub const fn is_visible(&self) -> bool {
        !self.is_label() && !self.is_dot()
    }

    /// The first cluster. FAT12/16 have no high word; some tools keep other
    /// data there, so it is ignored.
    pub const fn first_cluster(&self, kind: FatKind) -> u32 {
        match kind {
            FatKind::Fat32 => {
                (((self.cluster_high as u32) << 16) | self.cluster_low as u32) & kind.mask()
            }
            FatKind::Fat12 | FatKind::Fat16 => self.cluster_low as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dirent::ATTR_ARCHIVE;

    fn short(name: &[u8; 11], attr: u8) -> [u8; 32] {
        let mut raw = [0u8; 32];
        raw[..11].copy_from_slice(name);
        raw[11] = attr;
        raw
    }

    #[test]
    fn classifies_slots() {
        assert_eq!(Slot::parse(&[0; 32]), Slot::End);
        let mut free = short(b"FILE    TXT", ATTR_ARCHIVE);
        free[0] = 0xE5;
        assert_eq!(Slot::parse(&free), Slot::Free);
        let mut long = [0xFFu8; 32];
        long[0] = 0x41;
        long[11] = ATTR_LONG_NAME;
        long[13] = 0x5A;
        let Slot::Long(entry) = Slot::parse(&long) else {
            panic!("long entry")
        };
        assert_eq!((entry.sequence(), entry.checksum()), (0x41, 0x5A));
    }

    #[test]
    fn decodes_short_fields() {
        let mut raw = short(b"\x05BC     TXT", ATTR_ARCHIVE);
        raw[12] = 0x18;
        raw[20..22].copy_from_slice(&0x0001u16.to_le_bytes());
        raw[26..28].copy_from_slice(&0x0002u16.to_le_bytes());
        raw[28..32].copy_from_slice(&1234u32.to_le_bytes());
        let Slot::Short(entry) = Slot::parse(&raw) else {
            panic!("short entry")
        };
        assert_eq!(entry.name[0], 0x05);
        assert_eq!(entry.nt_case, 0x18);
        assert_eq!(entry.size, 1234);
        assert_eq!(entry.first_cluster(FatKind::Fat32), 0x0001_0002);
        assert_eq!(entry.first_cluster(FatKind::Fat16), 2);
        assert!(entry.is_visible() && !entry.is_dir());
    }

    #[test]
    fn short_entries_round_trip() {
        let mut entry = ShortEntry::new(*b"\x05BC     TXT", ATTR_ARCHIVE);
        entry.nt_case = 0x08;
        entry.created_tenths = 7;
        entry.created_time = 0x1234;
        entry.created_date = 0x5678;
        entry.accessed_date = 0x9ABC;
        entry.modified_time = 0xDEF0;
        entry.modified_date = 0x1357;
        entry.size = 0x0102_0304;
        entry.set_first_cluster(FatKind::Fat32, 0x0ABC_DEF1);
        assert_eq!(Slot::parse(&entry.encode()), Slot::Short(entry));
        assert_eq!(entry.first_cluster(FatKind::Fat32), 0x0ABC_DEF1);
        entry.set_first_cluster(FatKind::Fat16, 9);
        assert_eq!(entry.first_cluster(FatKind::Fat16), 9);
        assert_eq!(entry.encode()[20..22], 0x0ABCu16.to_le_bytes());
    }

    #[test]
    fn long_entries_round_trip() {
        let units: [u16; LFN_UNITS_PER_ENTRY] = core::array::from_fn(|i| 0x100 + i as u16);
        let raw = LongEntry::new(0x42, 0x99, &units).encode();
        let Slot::Long(entry) = Slot::parse(&raw) else {
            panic!("long entry")
        };
        assert_eq!((entry.sequence(), entry.checksum()), (0x42, 0x99));
        assert_eq!(entry.units(), (units, LFN_UNITS_PER_ENTRY));
        assert_eq!(raw[12], 0);
        assert_eq!(raw[26..28], [0, 0]);
    }

    #[test]
    fn dot_label_and_directory() {
        let Slot::Short(dot) = Slot::parse(&short(b"..         ", ATTR_DIRECTORY)) else {
            panic!("short entry")
        };
        assert!(dot.is_dot() && dot.is_dot_dot() && dot.is_dir() && !dot.is_visible());
        let Slot::Short(label) = Slot::parse(&short(b"HADRIS     ", ATTR_VOLUME_ID)) else {
            panic!("short entry")
        };
        assert!(label.is_label() && !label.is_visible());
    }
}
