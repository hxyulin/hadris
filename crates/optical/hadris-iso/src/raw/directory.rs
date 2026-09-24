use core::fmt;

use super::{DirDateTime, U16Both, U32Both};

bitflags::bitflags! {
    /// The file flags of a directory record (ECMA-119 9.1.6).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct FileFlags: u8 {
        /// Hidden from ordinary listings.
        const HIDDEN = 0b0000_0001;
        /// A directory.
        const DIRECTORY = 0b0000_0010;
        /// An associated file.
        const ASSOCIATED_FILE = 0b0000_0100;
        /// The extended attribute record describes the format.
        const RECORD = 0b0000_1000;
        /// The extended attribute record holds permissions.
        const PROTECTION = 0b0001_0000;
        /// Reserved, zero.
        const RESERVED = 0b0110_0000;
        /// More records of the same file follow.
        const NOT_FINAL = 0b1000_0000;
    }
}

/// The fixed part of a directory record (ECMA-119 9.1).
///
/// @hadris-spec ECMA-119:9.1
/// @hadris-compliance partial
/// @hadris-note Fixed fields round-trip, but all identifier, flag, and semantic constraints are not yet validated.
/// @hadris-tests raw::directory::tests::directory_record_parse_roundtrip
/// @hadris-fuzz iso_read
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DirectoryRecordHeader {
    /// The length of the whole record.
    pub len: u8,
    /// The length of the extended attribute record, in logical blocks.
    pub extended_attr_record: u8,
    /// The first logical block of the extent.
    pub extent: U32Both,
    /// The length of the extent in bytes.
    pub data_len: U32Both,
    /// When the file was recorded.
    pub date_time: DirDateTime,
    /// The file flags; see [`FileFlags`].
    pub flags: u8,
    /// The file unit size of an interleaved file.
    pub file_unit_size: u8,
    /// The interleave gap size of an interleaved file.
    pub interleave_gap_size: u8,
    /// The volume in the set holding the extent.
    pub volume_sequence_number: U16Both,
    /// The length of the file identifier.
    pub file_identifier_len: u8,
}

impl DirectoryRecordHeader {
    /// The file flags.
    pub fn file_flags(&self) -> FileFlags {
        FileFlags::from_bits_retain(self.flags)
    }

    /// Whether the record names a directory.
    pub fn is_directory(&self) -> bool {
        self.file_flags().contains(FileFlags::DIRECTORY)
    }
}

/// A whole directory record (ECMA-119 9.1): the header, the file identifier
/// and the system use area, in a 256-byte buffer.
///
/// @hadris-spec ECMA-119:9.1
/// @hadris-compliance partial
/// @hadris-tests raw::directory::tests::directory_record_parse_roundtrip, raw::directory::tests::directory_record_rejects_invalid_bounds_and_endian_copy
/// @hadris-fuzz iso_read
/// @hadris-note Records are validated for length, padding and redundant fields on read; identifier character sets are not.
#[repr(transparent)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DirectoryRecord {
    data: [u8; 256],
}

impl Default for DirectoryRecord {
    fn default() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

impl DirectoryRecord {
    /// The size of the fixed part.
    pub const HEADER_LEN: usize = size_of::<DirectoryRecordHeader>();

    /// Parses the record at the start of `bytes`, which must hold it whole.
    ///
    /// Returns `None` for a zero length byte (padding at the end of a
    /// sector) and `Err(())` for a record that breaks ECMA-119: too short,
    /// odd, longer than `bytes`, an identifier past the end or without its
    /// padding byte, reserved flags set, or redundant fields that disagree.
    #[allow(clippy::result_unit_err)]
    pub fn parse(bytes: &[u8]) -> Result<Option<Self>, ()> {
        let Some(&len) = bytes.first() else {
            return Err(());
        };
        if len == 0 {
            return Ok(None);
        }
        let len = usize::from(len);
        if len < Self::HEADER_LEN + 1 || len % 2 != 0 || len > bytes.len() {
            return Err(());
        }
        let mut record = Self::default();
        record.data[..len].copy_from_slice(&bytes[..len]);
        let header = *record.header();
        let name_len = usize::from(header.file_identifier_len);
        if name_len == 0
            || Self::HEADER_LEN + name_len > len
            || header.flags & FileFlags::RESERVED.bits() != 0
            || !header.extent.is_consistent()
            || !header.data_len.is_consistent()
            || !header.volume_sequence_number.is_consistent()
        {
            return Err(());
        }
        if name_len % 2 == 0 {
            let padding = Self::HEADER_LEN + name_len;
            if padding >= len || record.data[padding] != 0 {
                return Err(());
            }
        }
        Ok(Some(record))
    }

    /// A record for `name` with `system_use`, or `None` when they do not fit
    /// 255 bytes or the name is empty. The header fields other than the
    /// lengths are zero.
    pub fn new(name: &[u8], system_use: &[u8]) -> Option<Self> {
        if name.is_empty() || name.len() > 255 {
            return None;
        }
        let su_start = (Self::HEADER_LEN + name.len() + 1) & !1;
        let len = (su_start + system_use.len() + 1) & !1;
        if len > 255 {
            return None;
        }
        let mut record = Self::default();
        record.data[0] = len as u8;
        record.data[32] = name.len() as u8;
        record.data[Self::HEADER_LEN..Self::HEADER_LEN + name.len()].copy_from_slice(name);
        record.data[su_start..su_start + system_use.len()].copy_from_slice(system_use);
        Some(record)
    }

    /// The fixed part.
    pub fn header(&self) -> &DirectoryRecordHeader {
        bytemuck::from_bytes(&self.data[..Self::HEADER_LEN])
    }

    /// The fixed part, for editing. Do not change the lengths.
    pub fn header_mut(&mut self) -> &mut DirectoryRecordHeader {
        bytemuck::from_bytes_mut(&mut self.data[..Self::HEADER_LEN])
    }

    /// The record length in bytes.
    pub fn len(&self) -> usize {
        usize::from(self.data[0])
    }

    /// Whether this is an empty record, which a valid parse never returns.
    pub fn is_empty(&self) -> bool {
        self.data[0] == 0
    }

    /// The file identifier, as stored.
    pub fn name(&self) -> &[u8] {
        let len = usize::from(self.header().file_identifier_len);
        &self.data[Self::HEADER_LEN..(Self::HEADER_LEN + len).min(256)]
    }

    /// Whether this is the `.` or `..` record of a directory.
    pub fn is_dot(&self) -> bool {
        matches!(self.name(), [0] | [1])
    }

    /// The system use area.
    pub fn system_use(&self) -> &[u8] {
        let start = (Self::HEADER_LEN + usize::from(self.header().file_identifier_len) + 1) & !1;
        let end = self.len();
        if start >= end {
            return &[];
        }
        &self.data[start..end]
    }

    /// The system use area, for editing in place.
    pub fn system_use_mut(&mut self) -> &mut [u8] {
        let start = (Self::HEADER_LEN + usize::from(self.header().file_identifier_len) + 1) & !1;
        let end = self.len();
        if start >= end {
            return &mut [];
        }
        &mut self.data[start..end]
    }

    /// The record's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len()]
    }
}

impl fmt::Debug for DirectoryRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectoryRecord")
            .field("header", self.header())
            .field("name", &self.name())
            .field("system_use_len", &self.system_use().len())
            .finish()
    }
}

/// The 34-byte root directory record inside a volume descriptor.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RootDirectoryRecord {
    /// The fixed part; the identifier is one zero byte.
    pub header: DirectoryRecordHeader,
    /// The identifier, `0`.
    pub identifier: u8,
}

impl RootDirectoryRecord {
    /// A root record for the directory at `extent` of `size` bytes.
    pub fn new(extent: u32, size: u32, date_time: DirDateTime) -> Self {
        Self {
            header: DirectoryRecordHeader {
                len: 34,
                extended_attr_record: 0,
                extent: U32Both::new(extent),
                data_len: U32Both::new(size),
                date_time,
                flags: FileFlags::DIRECTORY.bits(),
                file_unit_size: 0,
                interleave_gap_size: 0,
                volume_sequence_number: U16Both::new(1),
                file_identifier_len: 1,
            },
            identifier: 0,
        }
    }
}

const _: () = {
    assert!(size_of::<DirectoryRecordHeader>() == 33);
    assert!(size_of::<RootDirectoryRecord>() == 34);
};

#[cfg(test)]
mod tests {
    use super::*;

    fn dot(id: u8) -> [u8; 34] {
        let mut record = [0u8; 34];
        record[0] = 34;
        record[2..10].copy_from_slice(bytemuck::bytes_of(&U32Both::new(20)));
        record[10..18].copy_from_slice(bytemuck::bytes_of(&U32Both::new(2048)));
        record[25] = FileFlags::DIRECTORY.bits();
        record[28..32].copy_from_slice(bytemuck::bytes_of(&U16Both::new(1)));
        record[32] = 1;
        record[33] = id;
        record
    }

    #[test]
    fn directory_record_parse_roundtrip() {
        let record = DirectoryRecord::parse(&dot(0)).unwrap().unwrap();
        assert_eq!((record.len(), record.name()), (34, &[0][..]));
        assert!(record.header().is_directory() && record.is_dot());
        assert_eq!(record.header().extent.get(), 20);
        assert_eq!(record.header().data_len.get(), 2048);

        let mut made = DirectoryRecord::new(b"README.;1", b"NM\x05\x01\x00").unwrap();
        made.header_mut().extent.set(42);
        made.header_mut().data_len.set(100);
        made.header_mut().volume_sequence_number.set(1);
        let parsed = DirectoryRecord::parse(made.as_bytes()).unwrap().unwrap();
        assert_eq!(parsed.name(), b"README.;1");
        assert_eq!(parsed.system_use(), b"NM\x05\x01\x00\x00");
        assert_eq!(parsed.header().extent.get(), 42);
        assert!(DirectoryRecord::parse(&[0u8; 4]).unwrap().is_none());
    }

    #[test]
    fn directory_record_rejects_invalid_bounds_and_endian_copy() {
        let mut short = dot(0);
        short[0] = 32;
        assert!(DirectoryRecord::parse(&short).is_err());
        let mut mismatched = dot(0);
        mismatched[9] = 1;
        assert!(DirectoryRecord::parse(&mismatched).is_err());
        let mut flagged = dot(0);
        flagged[25] |= 0x40;
        assert!(DirectoryRecord::parse(&flagged).is_err());
        assert!(DirectoryRecord::parse(&dot(0)[..20]).is_err());
        let mut unpadded = DirectoryRecord::new(b"AB", &[]).unwrap();
        unpadded.header_mut().volume_sequence_number.set(1);
        let mut bytes = [0u8; 36];
        bytes.copy_from_slice(unpadded.as_bytes());
        bytes[35] = 7;
        assert!(DirectoryRecord::parse(&bytes).is_err());
        assert!(DirectoryRecord::new(&[b'A'; 250], &[0; 10]).is_none());
    }
}
