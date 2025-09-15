use crate::directory::{DirectoryRecord, DirectoryRecordHeader, DirectoryRef};
use alloc::vec;
use bytemuck::Zeroable;
use hadris_io::{self as io, Read, Seek, SeekFrom};
use spin::Mutex;

pub struct IsoDir<'a, T: Seek> {
    pub(crate) reader: &'a Mutex<T>,
    pub(crate) directory: DirectoryRef,
}

pub struct IsoDirIter<'a, T: Read + Seek> {
    pub(crate) reader: &'a Mutex<T>,
    pub(crate) directory: DirectoryRef,
    pub(crate) offset: usize,
}

macro_rules! try_io_result_option {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(err) => return Some(Err(err)),
        }
    };
}

impl<T: Read + Seek> IsoDirIter<'_, T> {
    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl<T: Read + Seek> Iterator for IsoDirIter<'_, T> {
    type Item = io::Result<DirectoryRecord>;
    fn next(&mut self) -> Option<Self::Item> {
        const ENTRY_SIZE: usize = size_of::<DirectoryRecordHeader>();
        let mut reader = self.reader.lock();
        if self.offset >= self.directory.size {
            return None;
        }
        try_io_result_option!(reader.seek(SeekFrom::Start(
            (self.directory.extent.0 as u64) * 2048 + (self.offset as u64),
        )));
        let mut header = DirectoryRecordHeader::zeroed();
        try_io_result_option!(reader.read_exact(bytemuck::bytes_of_mut(&mut header)));
        // SPEC: We should check if the flag for the NOT_FINAL bit instaed of checking the length (but probably check it as well)
        if header.len == 0 {
            return None;
        }
        let mut bytes = vec![0; header.len as usize - ENTRY_SIZE];
        try_io_result_option!(reader.read_exact(&mut bytes));
        // Truncate to string length, since we don't need the padding
        _ = bytes.split_off(header.file_identifier_len as usize);
        self.offset += header.len as usize;

        Some(Ok(DirectoryRecord {
            header,
            name: bytes.into(),
        }))
    }
}

impl<'a, T: Read + Seek> IsoDir<'a, T> {
    // TODO: Refactor this, because we dont need the offset always
    /// Returns a list of all entries in the directory, along with their offset in the directory
    pub fn entries(&self) -> IsoDirIter<'_, T> {
        IsoDirIter {
            reader: self.reader,
            directory: self.directory,
            offset: 0,
        }
    }

    /*
    pub fn find_directory(&mut self, name: &str) -> Result<Option<IsoDir<T>>, Error> {
        let entry = self.entries()?.iter().find_map(|(_offset, entry)| {
            if entry.name.as_str() == name
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
                    offset: entry.header.extent.read() as usize,
                    size: entry.header.data_len.read() as usize,
                },
            })),
            None => Ok(None),
        }
    }

    pub fn read_file(&self, name: &str) -> Result<Vec<u8>, Error> {
        let entry = self.entries()?.iter().find_map(|(_offset, entry)| {
            if entry.name.as_str() == name {
                Some(entry.clone())
            } else {
                None
            }
        });
        match entry {
            Some(entry) => {
                let mut reader = self.reader.lock();
                let mut bytes = vec![0; entry.header.data_len.read() as usize];
                reader.seek(SeekFrom::Start(entry.header.extent.read() as u64))?;
                reader.read_exact(&mut bytes)?;
                Ok(bytes)
            }
            None => todo!("Custom not found error"),
        }
    }
    */
}
