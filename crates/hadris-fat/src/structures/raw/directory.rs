/// A file entry in a directory,
/// regardless of the file system type
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
    /// Reserved for use by Windows NT
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
    pub first_cluster_high: [u8; 2],
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
    pub first_cluster_low: [u8; 2],
    /// DIR_FileSize
    ///
    /// The size of the file, in bytes
    pub size: [u8; 4],
}

/// A long file name entry
/// The maximum length of a long file name is 255 characters, not including the null terminator
/// The characters allowed extend these characters:
///  . + , ; = [ ]
/// Embedded paces are also allowed
/// The name is stored in UTF-16 encoding (UNICODE)
/// When the unicode character cannot be translated to ANSI, an underscore is used
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

#[cfg(feature = "lfn")]
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub union RawDirectoryEntry {
    pub file: RawFileEntry,
    pub lfn: RawLfnEntry,
}
#[cfg(not(feature = "lfn"))]
pub type RawDirectoryEntry = RawFileEntry;

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};
    use static_assertions::const_assert_eq;

    const_assert_eq!(size_of::<RawFileEntry>(), 32);
    const_assert_eq!(align_of::<RawFileEntry>(), 1);

    const_assert_eq!(offset_of!(RawFileEntry, name), 0);
    const_assert_eq!(offset_of!(RawFileEntry, attributes), 11);
    const_assert_eq!(offset_of!(RawFileEntry, reserved), 12);
    const_assert_eq!(offset_of!(RawFileEntry, creation_time_tenth), 13);
    const_assert_eq!(offset_of!(RawFileEntry, creation_time), 14);
    const_assert_eq!(offset_of!(RawFileEntry, creation_date), 16);
    const_assert_eq!(offset_of!(RawFileEntry, last_access_date), 18);
    const_assert_eq!(offset_of!(RawFileEntry, first_cluster_high), 20);
    const_assert_eq!(offset_of!(RawFileEntry, last_write_time), 22);
    const_assert_eq!(offset_of!(RawFileEntry, last_write_date), 24);
    const_assert_eq!(offset_of!(RawFileEntry, first_cluster_low), 26);
    const_assert_eq!(offset_of!(RawFileEntry, size), 28);

    #[cfg(feature = "lfn")]
    mod lfn {
        use super::*;

        const_assert_eq!(size_of::<RawLfnEntry>(), 32);
        const_assert_eq!(align_of::<RawLfnEntry>(), 1);

        const_assert_eq!(offset_of!(RawLfnEntry, sequence_number), 0);
        const_assert_eq!(offset_of!(RawLfnEntry, name1), 1);
        const_assert_eq!(offset_of!(RawLfnEntry, attributes), 11);
        const_assert_eq!(offset_of!(RawLfnEntry, ty), 12);
        const_assert_eq!(offset_of!(RawLfnEntry, checksum), 13);
        const_assert_eq!(offset_of!(RawLfnEntry, name2), 14);
        const_assert_eq!(offset_of!(RawLfnEntry, first_cluster_low), 26);
        const_assert_eq!(offset_of!(RawLfnEntry, name3), 28);
    }
}
