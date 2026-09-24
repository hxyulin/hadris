//! 32-byte directory entries: classification and short-entry fields.

use super::entry::FatKind;
use super::lfn::{self, UNITS_PER_ENTRY};
use hadris_common::types::endian::Endian;
use hadris_common::types::number::{U16, U32};

/// Size of one directory entry.
pub(crate) const ENTRY_SIZE: u64 = 32;
/// The most entries one directory may hold.
pub(crate) const MAX_ENTRIES: u32 = 65_536;

pub(crate) use crate::raw::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_HIDDEN, ATTR_LONG_NAME, ATTR_READ_ONLY, ATTR_SYSTEM,
    ATTR_VOLUME_ID, ENTRY_FREE as FREE,
};
use crate::raw::{ATTR_LONG_NAME_MASK, ENTRY_END as END, RawDirEntry, RawLfnEntry};

/// What a directory slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
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
pub(crate) struct LongEntry {
    pub(crate) sequence: u8,
    pub(crate) checksum: u8,
    pub(crate) name1: [u8; 10],
    pub(crate) name2: [u8; 12],
    pub(crate) name3: [u8; 4],
}

/// The fields of a short entry, decoded from a [`RawDirEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShortEntry {
    /// The 11 name bytes as stored, so a leading `0xE5` reads `0x05`.
    pub(crate) name: [u8; 11],
    pub(crate) attr: u8,
    pub(crate) nt_case: u8,
    pub(crate) created_tenths: u8,
    pub(crate) created_time: u16,
    pub(crate) created_date: u16,
    pub(crate) accessed_date: u16,
    cluster_high: u16,
    pub(crate) modified_time: u16,
    pub(crate) modified_date: u16,
    cluster_low: u16,
    pub(crate) size: u32,
}

impl Slot {
    pub(crate) fn parse(raw: &[u8; 32]) -> Self {
        match raw[0] {
            END => return Self::End,
            FREE => return Self::Free,
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

/// Encodes one long-name fragment as a directory slot.
pub(crate) fn encode_long(sequence: u8, checksum: u8, units: &[u16; UNITS_PER_ENTRY]) -> [u8; 32] {
    let (name1, name2, name3) = lfn::pack(units);
    bytemuck::cast(RawLfnEntry {
        sequence,
        name1,
        attributes: ATTR_LONG_NAME,
        kind: 0,
        checksum,
        name2,
        first_cluster_low: [0; 2],
        name3,
    })
}

impl ShortEntry {
    /// An entry with the given stored name and attributes, no times, no
    /// clusters and size 0.
    pub(crate) const fn new(name: [u8; 11], attr: u8) -> Self {
        Self {
            name,
            attr,
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
    pub(crate) fn encode(&self) -> [u8; 32] {
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

    /// Sets the first cluster. FAT12/16 keep whatever the high word held.
    pub(crate) const fn set_first_cluster(&mut self, kind: FatKind, cluster: u32) {
        self.cluster_low = cluster as u16;
        if let FatKind::Fat32 = kind {
            self.cluster_high = (cluster >> 16) as u16;
        }
    }

    pub(crate) const fn is_dir(&self) -> bool {
        self.attr & ATTR_DIRECTORY != 0
    }

    /// Whether this is the volume label rather than a file or directory.
    pub(crate) const fn is_label(&self) -> bool {
        self.attr & ATTR_VOLUME_ID != 0
    }

    /// Whether this is the `.` or `..` entry of a subdirectory.
    pub(crate) const fn is_dot(&self) -> bool {
        self.name[0] == b'.'
    }

    /// Whether this is the `..` entry.
    pub(crate) fn is_dot_dot(&self) -> bool {
        self.name == *b"..         "
    }

    /// Whether a listing shows this entry.
    pub(crate) const fn is_visible(&self) -> bool {
        !self.is_label() && !self.is_dot()
    }

    /// The first cluster. FAT12/16 have no high word; some tools keep other
    /// data there, so it is ignored.
    pub(crate) const fn first_cluster(&self, kind: FatKind) -> u32 {
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
        assert_eq!((entry.sequence, entry.checksum), (0x41, 0x5A));
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
        let units: [u16; UNITS_PER_ENTRY] = core::array::from_fn(|i| 0x100 + i as u16);
        let raw = encode_long(0x42, 0x99, &units);
        let Slot::Long(entry) = Slot::parse(&raw) else {
            panic!("long entry")
        };
        assert_eq!((entry.sequence, entry.checksum), (0x42, 0x99));
        assert_eq!(
            lfn::unpack(&entry.name1, &entry.name2, &entry.name3),
            (units, UNITS_PER_ENTRY)
        );
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
