use hadris_common::types::{
    endian::LittleEndian,
    number::{U16, U32},
};

mod boot;

pub use boot::{BpbExt32Flags, RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};

/// Raw 32-byte FAT short-name file or directory entry.
///
/// @hadris-spec FAT:DirEntry
/// @hadris-compliance partial
/// @hadris-note Name/attributes/timestamps/cluster/size and NT case flags (`DIR_NTRes`) are read and written; extended access-time granularity is not modeled.
/// @hadris-tests test_write::test_lowercase_short_name_uses_nt_case_flags
/// @hadris-fuzz fat_read
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawFileEntry {
    /// DIR_Name
    ///
    /// The name of the file, padded with spaces, and in the 8.3 format
    /// A value of 0xE5 indicates that the directory is free. For kanji, 0x05 is used instead of 0xE5
    /// The special value 0x00 also indicates that the directory is free, but also all the entries
    /// following it are free
    /// The name cannot start with a space
    /// Only upper case letters, digits, and the following characters are allowed:
    /// $ % ' - _ @ ~ ` ! ( ) { } ^ # &
    pub name: [u8; 11],
    /// DIR_Attr
    ///
    /// The file attributes
    pub attributes: u8,
    /// DIR_NTRes
    ///
    /// Reserved for use by Windows NT. In practice Windows stores 8.3 name
    /// case information here: bit 3 (`0x08`) marks the base name as originally
    /// lowercase and bit 4 (`0x10`) marks the extension as originally
    /// lowercase. See [`NtCaseFlags`].
    pub reserved: u8,
    /// DIR_CrtTimeTenth
    ///
    /// The creation time, in tenths of a second
    pub creation_time_tenth: u8,
    /// DIR_CrtTime
    ///
    /// The creation time, granularity is 2 seconds
    pub creation_time: [u8; 2],
    /// DIR_CrtDate
    ///
    /// The creation date
    pub creation_date: [u8; 2],
    /// DIR_LstAccDate
    ///
    /// The last access date
    pub last_access_date: [u8; 2],
    /// DIR_FstClusHI
    ///
    /// The high word of the first cluster number
    pub first_cluster_high: U16<LittleEndian>,
    /// DIR_WrtTime
    ///
    /// The last write time, granularity is 2 seconds
    pub last_write_time: [u8; 2],
    /// DIR_WrtDate
    ///
    /// The last write date
    pub last_write_date: [u8; 2],
    /// DIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: U16<LittleEndian>,
    /// DIR_FileSize
    ///
    /// The size of the file, in bytes
    pub size: U32<LittleEndian>,
}

unsafe impl bytemuck::NoUninit for RawFileEntry {}
unsafe impl bytemuck::Zeroable for RawFileEntry {}
unsafe impl bytemuck::AnyBitPattern for RawFileEntry {}

/// A long file name entry
/// The maximum length of a long file name is 255 characters, not including the null terminator
/// The characters allowed extend these characters:
///  . + , ; = [ ]
/// Embedded paces are also allowed
/// The name is stored in UTF-16 encoding (UNICODE)
/// When the unicode character cannot be translated to ANSI, an underscore is used
///
/// @hadris-spec FAT:LFN
/// @hadris-compliance partial
/// @hadris-note This raw on-disk structure is complete, while semantic validation and legacy ANSI fallback behavior are implemented by higher-level LFN readers and writers.
/// @hadris-tests test_write::lfn_checksum_matches_short_name, test_write::lfn_padding_uses_terminator_then_filler
/// @hadris-fuzz fat_read
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct RawLfnEntry {
    /// LFN_Ord
    ///
    /// The order of the LFN entry, the contents must be masked with 0x40 for the last entry
    pub sequence_number: u8,
    /// LFN_Name1
    ///
    /// The first part of the long file name
    pub name1: [u8; 10],
    /// LDIR_Attr
    ///
    /// Attributes, must be set to: ATTR_LONG_NAME, which is:
    /// ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID
    pub attributes: u8,
    /// LFN_Type
    ///
    /// The type of the LFN entry, must be set to 0
    pub ty: u8,
    /// LFN_Chksum
    ///
    /// Checksum of name in the associated short name directory entry at the end of the LFN sequence
    /// THe algorithm described in the FAT spec is:
    /// unsigned char ChkSum (unsigned char \*pFcbName)
    /// {
    ///     short FcbNameLen;
    ///     unsigned char Sum;
    ///     Sum = 0;
    ///     for (FcbNameLen=11; FcbNameLen!=0; FcbNameLen--) {
    ///         // NOTE: The operation is an unsigned char rotate right
    ///         Sum = ((Sum & 1) ? 0x80 : 0) + (Sum >> 1) + *pFcbName++;
    ///     }
    ///     return (Sum);
    /// }
    pub checksum: u8,
    /// LFN_Name2
    ///
    /// The second part of the long file name
    pub name2: [u8; 12],
    /// LDIR_FstClusLO
    ///
    /// The low word of the first cluster number
    pub first_cluster_low: [u8; 2],
    /// LFN_Name3
    ///
    /// The third part of the long file name
    pub name3: [u8; 4],
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
/// Raw directory entry interpreted as a short entry, LFN entry, or bytes.
pub union RawDirectoryEntry {
    /// Short-name file or directory representation.
    pub file: RawFileEntry,
    #[cfg(feature = "lfn")]
    /// Long-file-name component representation.
    pub lfn: RawLfnEntry,
    /// Uninterpreted 32-byte representation.
    pub bytes: [u8; 32],
}

impl RawDirectoryEntry {
    /// Returns the entry's raw attribute byte.
    pub fn attributes(&self) -> u8 {
        unsafe { self.file }.attributes
    }
}

// Bytemuck implementations for RawDirectoryEntry union
// These are needed for read_struct to work
unsafe impl bytemuck::NoUninit for RawDirectoryEntry {}
unsafe impl bytemuck::Zeroable for RawDirectoryEntry {}
unsafe impl bytemuck::AnyBitPattern for RawDirectoryEntry {}

bitflags::bitflags! {
    /// Attribute bits stored in a FAT directory entry.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DirEntryAttrFlags: u8 {
        /// Entry is read-only.
        const READ_ONLY = 1 << 0;
        /// Entry is hidden.
        const HIDDEN = 1 << 1;
        /// Entry is a system file.
        const SYSTEM = 1 << 2;
        /// Entry contains a volume label.
        const VOLUME_ID = 1 << 3;
        /// Entry is a directory.
        const DIRECTORY = 1 << 4;
        /// Entry has the archive bit set.
        const ARCHIVE = 1 << 5;
    }
}

bitflags::bitflags! {
    /// Windows NT 8.3 name case flags stored in the `DIR_NTRes` byte.
    ///
    /// FAT stores 8.3 short names uppercase on disk. Windows records in this
    /// byte whether the base name and/or extension were originally lowercase
    /// so an all-lowercase (or lowercase-base / lowercase-ext) 8.3 name can be
    /// presented in its original case without spending a long-file-name entry.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NtCaseFlags: u8 {
        /// The base name (characters before the dot) was originally lowercase.
        const LOWER_BASE = 0x08;
        /// The extension (characters after the dot) was originally lowercase.
        const LOWER_EXT = 0x10;
    }
}

impl DirEntryAttrFlags {
    /// Attribute combination identifying a long-file-name component.
    pub const LONG_NAME: Self = Self::from_bits_truncate(
        Self::READ_ONLY.bits() | Self::HIDDEN.bits() | Self::SYSTEM.bits() | Self::VOLUME_ID.bits(),
    );

    /// True for a root-directory volume label entry (`VOLUME_ID` set, not a
    /// directory, not an LFN component).
    pub fn is_volume_label_entry(self) -> bool {
        self.contains(Self::VOLUME_ID) && !self.contains(Self::DIRECTORY) && self != Self::LONG_NAME
    }
}
