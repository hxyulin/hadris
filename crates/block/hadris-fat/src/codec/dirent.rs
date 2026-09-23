//! 32-byte directory entries: classification and short-entry fields.

use super::entry::FatKind;
use super::lfn::{self, UNITS_PER_ENTRY};

/// Size of one directory entry.
pub(crate) const ENTRY_SIZE: u64 = 32;
/// The most entries one directory may hold.
pub(crate) const MAX_ENTRIES: u32 = 65_536;

pub(crate) const ATTR_READ_ONLY: u8 = 0x01;
pub(crate) const ATTR_HIDDEN: u8 = 0x02;
pub(crate) const ATTR_SYSTEM: u8 = 0x04;
pub(crate) const ATTR_VOLUME_ID: u8 = 0x08;
pub(crate) const ATTR_DIRECTORY: u8 = 0x10;
pub(crate) const ATTR_ARCHIVE: u8 = 0x20;
pub(crate) const ATTR_LONG_NAME: u8 = 0x0F;

const END: u8 = 0x00;
/// First name byte of a deleted entry.
pub(crate) const FREE: u8 = 0xE5;

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

/// One long-name fragment.
///
/// @hadris-spec FAT:LFN
/// @hadris-compliance partial
/// @hadris-note Sequence, attributes, checksum, terminator and filler are read and written; names are UTF-16 only, with no legacy ANSI fallback.
/// @hadris-tests lfn::tests::checksum_matches_reference, lfn::tests::encoded_orders_entries_last_first, lfn::tests::assembler_rejects_broken_sequences, fatfs_write::long_names_up_to_255_units
/// @hadris-fuzz fat_read
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LongEntry {
    pub(crate) sequence: u8,
    pub(crate) checksum: u8,
    pub(crate) name1: [u8; 10],
    pub(crate) name2: [u8; 12],
    pub(crate) name3: [u8; 4],
}

/// The fields of a short entry.
///
/// @hadris-spec FAT:DirEntry
/// @hadris-compliance partial
/// @hadris-note Name/attributes/timestamps/cluster/size and NT case flags (`DIR_NTRes`) are read and written; extended access-time granularity is not modeled.
/// @hadris-tests dirent::tests::decodes_short_fields, fatfs_write::short_names_and_case_bits
/// @hadris-fuzz fat_read
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

fn u16_at(raw: &[u8; 32], at: usize) -> u16 {
    u16::from_le_bytes([raw[at], raw[at + 1]])
}

impl Slot {
    pub(crate) fn parse(raw: &[u8; 32]) -> Self {
        match raw[0] {
            END => return Self::End,
            FREE => return Self::Free,
            _ => {}
        }
        if raw[11] & 0x3F == ATTR_LONG_NAME {
            let mut long = LongEntry {
                sequence: raw[0],
                checksum: raw[13],
                name1: [0; 10],
                name2: [0; 12],
                name3: [0; 4],
            };
            long.name1.copy_from_slice(&raw[1..11]);
            long.name2.copy_from_slice(&raw[14..26]);
            long.name3.copy_from_slice(&raw[28..32]);
            return Self::Long(long);
        }
        let mut name = [0; 11];
        name.copy_from_slice(&raw[..11]);
        Self::Short(ShortEntry {
            name,
            attr: raw[11],
            nt_case: raw[12],
            created_tenths: raw[13],
            created_time: u16_at(raw, 14),
            created_date: u16_at(raw, 16),
            accessed_date: u16_at(raw, 18),
            cluster_high: u16_at(raw, 20),
            modified_time: u16_at(raw, 22),
            modified_date: u16_at(raw, 24),
            cluster_low: u16_at(raw, 26),
            size: u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]),
        })
    }
}

/// Encodes one long-name fragment as a directory slot.
pub(crate) fn encode_long(sequence: u8, checksum: u8, units: &[u16; UNITS_PER_ENTRY]) -> [u8; 32] {
    let (name1, name2, name3) = lfn::pack(units);
    let mut raw = [0u8; 32];
    raw[0] = sequence;
    raw[1..11].copy_from_slice(&name1);
    raw[11] = ATTR_LONG_NAME;
    raw[13] = checksum;
    raw[14..26].copy_from_slice(&name2);
    raw[28..32].copy_from_slice(&name3);
    raw
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
        let mut raw = [0u8; 32];
        raw[..11].copy_from_slice(&self.name);
        raw[11] = self.attr;
        raw[12] = self.nt_case;
        raw[13] = self.created_tenths;
        for (at, value) in [
            (14, self.created_time),
            (16, self.created_date),
            (18, self.accessed_date),
            (20, self.cluster_high),
            (22, self.modified_time),
            (24, self.modified_date),
            (26, self.cluster_low),
        ] {
            raw[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        raw[28..32].copy_from_slice(&self.size.to_le_bytes());
        raw
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
