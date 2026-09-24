//! 32-byte directory entry layouts and their constants.

use hadris_common::types::{
    endian::LittleEndian,
    number::{U16, U32},
};

/// Size of every directory entry.
pub const ENTRY_SIZE: usize = 32;
/// First name byte of the end marker: this slot and every later one are
/// unused.
pub const ENTRY_END: u8 = 0x00;
/// First name byte of a deleted entry.
pub const ENTRY_FREE: u8 = 0xE5;
/// First name byte stored for a name that starts with `0xE5`.
pub const ENTRY_KANJI_E5: u8 = 0x05;

/// `DIR_Attr` bit: read-only.
pub const ATTR_READ_ONLY: u8 = 0x01;
/// `DIR_Attr` bit: hidden.
pub const ATTR_HIDDEN: u8 = 0x02;
/// `DIR_Attr` bit: system.
pub const ATTR_SYSTEM: u8 = 0x04;
/// `DIR_Attr` bit: the volume label.
pub const ATTR_VOLUME_ID: u8 = 0x08;
/// `DIR_Attr` bit: a directory.
pub const ATTR_DIRECTORY: u8 = 0x10;
/// `DIR_Attr` bit: changed since the last backup.
pub const ATTR_ARCHIVE: u8 = 0x20;
/// `DIR_Attr` value of a long-name entry.
pub const ATTR_LONG_NAME: u8 = 0x0F;
/// The `DIR_Attr` bits compared with [`ATTR_LONG_NAME`].
pub const ATTR_LONG_NAME_MASK: u8 = 0x3F;

/// `DIR_NTRes` bit: the base name is shown in lower case.
pub const NT_LOWER_BASE: u8 = 0x08;
/// `DIR_NTRes` bit: the extension is shown in lower case.
pub const NT_LOWER_EXTENSION: u8 = 0x10;

/// `LDIR_Ord` bit of the long-name entry that holds the last part of the
/// name, which is the first one on disk.
pub const LFN_LAST_ENTRY: u8 = 0x40;
/// The `LDIR_Ord` bits that hold the 1-based entry number.
pub const LFN_SEQUENCE_MASK: u8 = 0x3F;
/// UTF-16 code units in each long-name entry.
pub const LFN_UNITS_PER_ENTRY: usize = 13;

/// A short directory entry: a file, a directory, `.`, `..` or the volume
/// label.
///
/// @hadris-spec FAT:DirEntry
/// @hadris-compliance partial
/// @hadris-note Name/attributes/timestamps/cluster/size and NT case flags (`DIR_NTRes`) are read and written; extended access-time granularity is not modeled.
/// @hadris-tests dirent::tests::decodes_short_fields, fatfs_write::short_names_and_case_bits, raw::dirent::tests::layouts_match_the_specification
/// @hadris-fuzz fat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RawDirEntry {
    /// `DIR_Name`: the 8.3 name, space padded, without the dot. A leading
    /// `0xE5` is stored as [`ENTRY_KANJI_E5`].
    pub name: [u8; 11],
    /// `DIR_Attr`
    pub attributes: u8,
    /// `DIR_NTRes`: the case bits [`NT_LOWER_BASE`] and
    /// [`NT_LOWER_EXTENSION`].
    pub nt_reserved: u8,
    /// `DIR_CrtTimeTenth`: creation time in 10 ms units, 0 to 199.
    pub created_tenths: u8,
    /// `DIR_CrtTime`
    pub created_time: U16<LittleEndian>,
    /// `DIR_CrtDate`
    pub created_date: U16<LittleEndian>,
    /// `DIR_LstAccDate`
    pub accessed_date: U16<LittleEndian>,
    /// `DIR_FstClusHI`: the high word of the first cluster, FAT32 only.
    pub first_cluster_high: U16<LittleEndian>,
    /// `DIR_WrtTime`
    pub modified_time: U16<LittleEndian>,
    /// `DIR_WrtDate`
    pub modified_date: U16<LittleEndian>,
    /// `DIR_FstClusLO`: the low word of the first cluster.
    pub first_cluster_low: U16<LittleEndian>,
    /// `DIR_FileSize`
    pub size: U32<LittleEndian>,
}

impl RawDirEntry {
    /// The checksum of [`name`](Self::name) that each long-name entry of
    /// this entry stores in `LDIR_Chksum`.
    pub const fn lfn_checksum(&self) -> u8 {
        crate::codec::lfn::checksum(&self.name)
    }
}

/// A long-name entry, holding 13 UTF-16 code units of a name.
///
/// @hadris-spec FAT:LFN
/// @hadris-compliance partial
/// @hadris-note Sequence, attributes, checksum, terminator and filler are read and written; names are UTF-16 only, with no legacy ANSI fallback.
/// @hadris-tests lfn::tests::checksum_matches_reference, lfn::tests::encoded_orders_entries_last_first, lfn::tests::assembler_rejects_broken_sequences, fatfs_write::long_names_up_to_255_units, raw::dirent::tests::layouts_match_the_specification
/// @hadris-fuzz fat_read
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RawLfnEntry {
    /// `LDIR_Ord`: the entry number, with [`LFN_LAST_ENTRY`] on the last.
    pub sequence: u8,
    /// `LDIR_Name1`: code units 1 to 5.
    pub name1: [u8; 10],
    /// `LDIR_Attr`, always [`ATTR_LONG_NAME`].
    pub attributes: u8,
    /// `LDIR_Type`, zero.
    pub kind: u8,
    /// `LDIR_Chksum`: [`RawDirEntry::lfn_checksum`] of the short entry.
    pub checksum: u8,
    /// `LDIR_Name2`: code units 6 to 11.
    pub name2: [u8; 12],
    /// `LDIR_FstClusLO`, zero.
    pub first_cluster_low: [u8; 2],
    /// `LDIR_Name3`: code units 12 and 13.
    pub name3: [u8; 4],
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{offset_of, size_of};

    #[test]
    fn layouts_match_the_specification() {
        assert_eq!(size_of::<RawDirEntry>(), ENTRY_SIZE);
        assert_eq!(size_of::<RawLfnEntry>(), ENTRY_SIZE);
        assert_eq!(offset_of!(RawDirEntry, attributes), 11);
        assert_eq!(offset_of!(RawDirEntry, first_cluster_high), 20);
        assert_eq!(offset_of!(RawDirEntry, first_cluster_low), 26);
        assert_eq!(offset_of!(RawDirEntry, size), 28);
        assert_eq!(offset_of!(RawLfnEntry, attributes), 11);
        assert_eq!(offset_of!(RawLfnEntry, checksum), 13);
        assert_eq!(offset_of!(RawLfnEntry, name2), 14);
        assert_eq!(offset_of!(RawLfnEntry, name3), 28);

        let mut entry: RawDirEntry = bytemuck::Zeroable::zeroed();
        entry.name = *b"README  TXT";
        assert_eq!(entry.lfn_checksum(), 0x73);
    }
}
