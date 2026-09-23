//! 32-byte directory entries: classification and short-entry fields.

use super::entry::FatKind;

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
const FREE: u8 = 0xE5;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LongEntry {
    pub(crate) sequence: u8,
    pub(crate) checksum: u8,
    pub(crate) name1: [u8; 10],
    pub(crate) name2: [u8; 12],
    pub(crate) name3: [u8; 4],
}

/// The fields of a short entry.
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

impl ShortEntry {
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
