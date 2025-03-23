use std::io::{SeekFrom, Write};

use bytemuck::Zeroable;

use crate::{
    ReadWriteSeek,
    types::{IsoStringFile, U16LsbMsb, U32LsbMsb},
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
            date_time: DirDateTime::default(),
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

#[derive(Debug, Clone)]
pub struct DirectoryRecord {
    pub header: DirectoryRecordHeader,
    pub name: IsoStringFile,
}

impl Default for DirectoryRecord {
    fn default() -> Self {
        Self {
            header: DirectoryRecordHeader::default(),
            name: IsoStringFile::empty(),
        }
    }
}

impl DirectoryRecord {
    pub fn size(&self) -> usize {
        size_of::<DirectoryRecordHeader>() + self.name.len()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(bytemuck::bytes_of(&self.header));
        bytes.extend_from_slice(self.name.bytes());
        bytes
    }

    pub fn new(name: IsoStringFile, dir_ref: DirectoryRef, flags: FileFlags) -> Self {
        Self {
            header: DirectoryRecordHeader {
                len: ((size_of::<DirectoryRecordHeader>() + name.len() + 1) & !1) as u8,
                extended_attr_record: 0,
                extent: U32LsbMsb::new(dir_ref.offset as u32),
                data_len: U32LsbMsb::new(dir_ref.size as u32),
                date_time: DirDateTime::now(),
                flags: flags.bits(),
                file_unit_size: 0,
                interleave_gap_size: 0,
                volume_sequence_number: U16LsbMsb::new(1),
                file_identifier_len: name.len() as u8,
            },
            name,
        }
    }

    /// Creates a new directory record with a given name length
    pub fn with_len(len: u8) -> Self {
        Self {
            header: DirectoryRecordHeader {
                file_identifier_len: len,
                ..Default::default()
            },
            name: IsoStringFile::with_size(len as usize),
        }
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<usize, std::io::Error> {
        let mut written = 0;
        writer.write_all(&self.header.to_bytes())?;
        written += size_of::<DirectoryRecordHeader>();
        writer.write_all(&self.name.bytes())?;
        written += self.name.len();
        if written < self.header.len as usize {
            for _ in 0..(self.header.len as usize - written) {
                writer.write_all(&[0])?;
            }
        }
        Ok(written)
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
}

#[derive(Default, Debug, Clone, Copy)]
pub struct DirectoryRef {
    pub offset: u64,
    pub size: u64,
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

pub struct IsoDir<'a, T: ReadWriteSeek> {
    pub(crate) reader: &'a mut T,
    pub(crate) directory: DirectoryRef,
}

impl<'a, T: ReadWriteSeek> IsoDir<'a, T> {
    // TODO: Refactor this, because we dont need the offset always
    /// Returns a list of all entries in the directory, along with their offset in the directory
    pub fn entries(&mut self) -> Result<Vec<(u64, DirectoryRecord)>, std::io::Error> {
        const ENTRY_SIZE: usize = size_of::<DirectoryRecordHeader>();
        self.reader
            .seek(SeekFrom::Start(self.directory.offset * 2048))?;
        let mut offset = 0;
        let mut entries = Vec::new();
        while offset < self.directory.size as usize {
            let mut header = DirectoryRecordHeader::zeroed();
            self.reader
                .read_exact(bytemuck::bytes_of_mut(&mut header))?;
            // SPEC: We should check if the flag for the NOT_FINAL bit instaed of checking the length (but probably check it as well)
            if header.len == 0 {
                break;
            }
            let mut bytes = vec![0; header.len as usize - ENTRY_SIZE];
            self.reader.read_exact(&mut bytes)?;
            // Truncate to string length, since we don't need the padding
            _ = bytes.split_off(header.file_identifier_len as usize);
            offset += header.len as usize;

            entries.push((
                offset as u64,
                DirectoryRecord {
                    header,
                    name: bytes.into(),
                },
            ));
        }
        Ok(entries.into_iter().collect())
    }

    pub fn find_directory(&mut self, name: &str) -> Result<Option<IsoDir<T>>, std::io::Error> {
        let entry = self.entries()?.iter().find_map(|(_offset, entry)| {
            if entry.name.to_str() == name
                && FileFlags::from_bits_retain(entry.header.flags).contains(FileFlags::DIRECTORY)
            {
                Some(entry.clone())
            } else {
                None
            }
        });
        match entry {
            Some(entry) => Ok(Some(IsoDir {
                reader: self.reader,
                directory: DirectoryRef {
                    offset: entry.header.extent.read() as u64,
                    size: entry.header.data_len.read() as u64,
                },
            })),
            None => Ok(None),
        }
    }

    pub fn read_file(&mut self, name: &str) -> Result<Vec<u8>, std::io::Error> {
        let entry = self.entries()?.iter().find_map(|(_offset, entry)| {
            if entry.name.to_str() == name {
                Some(entry.clone())
            } else {
                None
            }
        });
        match entry {
            Some(entry) => {
                let mut bytes = vec![0; entry.header.data_len.read() as usize];
                self.reader
                    .seek(SeekFrom::Start(entry.header.extent.read() as u64))?;
                self.reader.read_exact(&mut bytes)?;
                Ok(bytes)
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "File not found",
            )),
        }
    }
}
