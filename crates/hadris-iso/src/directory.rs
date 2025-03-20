use std::io::{SeekFrom, Write};

use crate::{types::{IsoStringFile, U16LsbMsb, U32LsbMsb}, ReadWriteSeek};

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

    pub fn new(name: &[u8], dir_ref: DirectoryRef, flags: FileFlags) -> Self {
        Self {
            header: DirectoryRecordHeader {
                len: ((size_of::<DirectoryRecordHeader>() + name.len() + 1) & !1) as u8,
                extended_attr_record: 0,
                extent: U32LsbMsb::new(dir_ref.offset as u32),
                data_len: U32LsbMsb::new(dir_ref.size as u32),
                date_time: DirDateTime::default(),
                flags: flags.bits(),
                file_unit_size: 0,
                interleave_gap_size: 0,
                volume_sequence_number: U16LsbMsb::new(1),
                file_identifier_len: name.len() as u8,
            },
            name: IsoStringFile::from_bytes(name),
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

pub struct IsoDirectory<'a, T: ReadWriteSeek> {
    pub(crate) reader: &'a mut T,
    pub(crate) directory: DirectoryRef,
}
impl<'a, T: ReadWriteSeek> IsoDirectory<'a, T> {
    // TODO: Make this private after testing
    /// Returns a list of all entries in the directory, along with their offset in the directory
    pub fn entries(&mut self) -> Result<Vec<(u64, DirectoryRecord)>, std::io::Error> {
        self.reader
            .seek(SeekFrom::Start(self.directory.offset * 2048))?;
        // This is the easiest implementation, but it's not the most efficient
        // because we are storing the entire directory in memory.
        let mut bytes = vec![0; self.directory.size as usize];
        self.reader.read_exact(&mut bytes)?;
        let mut entries = Vec::new();
        let mut idx = 0;
        while idx < bytes.len() {
            let entry = DirectoryRecordHeader::from_bytes(
                &bytes[idx..idx + size_of::<DirectoryRecordHeader>()],
            );
            if entry.len == 0 {
                break;
            }
            let name = IsoStringFile::from_bytes(
                &bytes[idx + size_of::<DirectoryRecordHeader>()
                    ..idx
                        + size_of::<DirectoryRecordHeader>()
                        + entry.file_identifier_len as usize],
            );
            entries.push((
                idx as u64,
                DirectoryRecord {
                    header: *entry,
                    name,
                },
            ));
            idx += entry.len as usize;
        }
        Ok(entries)
    }

    pub fn find_directory(
        &mut self,
        name: &str,
    ) -> Result<Option<IsoDirectory<T>>, std::io::Error> {
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
            Some(entry) => Ok(Some(IsoDirectory {
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

