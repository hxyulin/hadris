use bytemuck::Zeroable;
use hadris_io::{self as io, Read, Write};

use crate::{
    io::LogicalSector,
    types::{U16LsbMsb, U32LsbMsb},
};

/// The header of a directory record, because the identifier is variable length,
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DirectoryRecordHeader {
    pub len: u8,
    pub extended_attr_record: u8,
    /// The LBA of the record
    pub extent: U32LsbMsb,
    /// The length of the data in bytes
    pub data_len: U32LsbMsb,
    pub date_time: DirDateTime,
    pub flags: u8,
    pub file_unit_size: u8,
    pub interleave_gap_size: u8,
    pub volume_sequence_number: U16LsbMsb,
    pub file_identifier_len: u8,
}

impl Default for DirectoryRecordHeader {
    fn default() -> Self {
        Self {
            len: 0,
            extended_attr_record: 0,
            extent: U32LsbMsb::new(0),
            data_len: U32LsbMsb::new(0),
            date_time: DirDateTime::now(),
            flags: 0,
            file_unit_size: 0,
            interleave_gap_size: 0,
            volume_sequence_number: U16LsbMsb::new(0),
            file_identifier_len: 0,
        }
    }
}

impl DirectoryRecordHeader {
    pub fn from_bytes(bytes: &[u8]) -> &Self {
        bytemuck::from_bytes(bytes)
    }

    pub fn to_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    pub fn is_directory(&self) -> bool {
        FileFlags::from_bits_retain(self.flags).contains(FileFlags::DIRECTORY)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
pub struct DirectoryRecord {
    data: [u8; 256],
}

impl Default for DirectoryRecord {
    fn default() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

/// Error returned when trying to access a file as a directory
#[derive(Debug, Clone, Copy)]
pub struct NotADirectoryError;

impl core::fmt::Display for NotADirectoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "not a directory")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for NotADirectoryError {}

impl DirectoryRecord {
    const DATA_START: usize = size_of::<DirectoryRecordHeader>();

    #[inline]
    pub fn header(&self) -> &DirectoryRecordHeader {
        bytemuck::from_bytes(&self.data[0..Self::DATA_START])
    }

    #[inline]
    pub fn header_mut(&mut self) -> &mut DirectoryRecordHeader {
        bytemuck::from_bytes_mut(&mut self.data[0..size_of::<DirectoryRecordHeader>()])
    }

    #[inline]
    pub fn name(&self) -> &[u8] {
        let len = self.header().file_identifier_len as usize;
        &self.data[Self::DATA_START..Self::DATA_START + len]
    }

    /// Get the filename decoded from Joliet (UTF-16 BE) encoding
    ///
    /// This is useful when reading from a Joliet supplementary volume descriptor.
    /// Returns the decoded Unicode string.
    #[cfg(feature = "alloc")]
    pub fn joliet_name(&self) -> alloc::string::String {
        crate::joliet::decode_joliet_name(self.name())
    }

    /// Check if this entry's name appears to be Joliet-encoded (UTF-16 BE)
    #[cfg(feature = "alloc")]
    pub fn is_joliet_name(&self) -> bool {
        crate::joliet::is_likely_joliet_name(self.name())
    }

    #[inline]
    pub fn system_use(&self) -> &[u8] {
        let header = self.header();
        &self.data[Self::DATA_START + header.file_identifier_len as usize..header.len as usize]
    }

    #[inline]
    pub fn is_special(&self) -> bool {
        self.name() == b"\x00" || self.name() == b"\x01"
    }

    #[inline]
    pub fn is_directory(&self) -> bool {
        self.header().is_directory()
    }

    pub fn is_file(&self) -> bool {
        !self.header().is_directory()
    }

    pub fn as_dir_ref(&self) -> Result<DirectoryRef, NotADirectoryError> {
        if !self.is_directory() {
            return Err(NotADirectoryError);
        }

        let header = self.header();
        Ok(DirectoryRef {
            extent: LogicalSector(header.extent.read() as usize),
            size: header.data_len.read() as usize,
        })
    }

    pub fn size(&self) -> usize {
        self.header().len as usize
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.data[0..self.size()]
    }

    pub fn new(name: &[u8], system_use: &[u8], directory: DirectoryRef, flags: FileFlags) -> Self {
        let mut sel = Self::zeroed();
        let len = Self::DATA_START + name.len() + system_use.len();
        *sel.header_mut() = DirectoryRecordHeader {
            len: (len as u8 + 1) & !1,
            extended_attr_record: 0,
            extent: U32LsbMsb::new(directory.extent.0 as u32),
            data_len: U32LsbMsb::new(directory.size as u32),
            date_time: DirDateTime::now(),
            flags: flags.bits(),
            file_unit_size: 0,
            interleave_gap_size: 0,
            volume_sequence_number: U16LsbMsb::new(1),
            file_identifier_len: name.len() as u8,
        };
        sel.data[Self::DATA_START..Self::DATA_START + name.len()].copy_from_slice(name);
        sel.data[Self::DATA_START + name.len()..len].copy_from_slice(system_use);
        sel
    }

    pub fn with_len(name_len: usize, su_len: usize) -> Self {
        let mut sel = Self::zeroed();
        let len = Self::DATA_START + name_len + su_len;
        sel.header_mut().len = len as u8;
        sel
    }

    pub fn parse<R: Read>(reader: &mut R) -> io::Result<Self> {
        let mut sel = Self::zeroed();
        reader.read_exact(&mut sel.data[0..Self::DATA_START])?;
        let size = sel.size();
        if size > Self::DATA_START {
            reader.read_exact(&mut sel.data[Self::DATA_START..size])?;
        }
        Ok(sel)
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        let size = self.size();
        writer.write_all(&self.data[0..size])?;
        Ok(size)
    }
}

/// The root directory entry
#[repr(C)]
#[derive(Default, Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RootDirectoryEntry {
    pub header: DirectoryRecordHeader,
    /// There is no name on the root directory, so this is always empty
    pub padding: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DirDateTime {
    /// Number of years since 1900
    year: u8,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    offset: u8,
}

impl Default for DirDateTime {
    fn default() -> Self {
        Self {
            year: 0,
            month: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            offset: 0,
        }
    }
}

impl DirDateTime {
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        use chrono::{Datelike, Timelike, Utc};
        let now = Utc::now();
        Self {
            year: (now.year() - 1900) as u8,
            month: now.month() as u8,
            day: now.day() as u8,
            hour: now.hour() as u8,
            minute: now.minute() as u8,
            second: now.second() as u8,
            // UTC offset is always 0
            offset: 0,
        }
    }

    /// Creates a zeroed datetime for no-std environments
    #[cfg(not(feature = "std"))]
    pub fn now() -> Self {
        Self::default()
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct DirectoryRef {
    pub extent: LogicalSector,
    pub size: usize,
}

bitflags::bitflags! {
    pub struct FileFlags: u8 {
        const HIDDEN = 0b0000_0001;
        const DIRECTORY = 0b0000_0010;
        const ASSOCIATED_FILE = 0b0000_0100;
        const EXTENDED_ATTRIBUTES = 0b0000_1000;
        const EXTENDED_PERMISSIONS = 0b0001_0000;
        const NOT_FINAL = 0b1000_0000;
    }
}
