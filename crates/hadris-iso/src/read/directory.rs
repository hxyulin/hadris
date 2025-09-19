use core::ops::DerefMut;

use crate::{
    io::{self, Read, Seek, SeekFrom, IsoCursor},
    directory::{DirectoryRecord, DirectoryRecordHeader, DirectoryRef},
    file::FixedFilename,
};
use alloc::vec;
use bytemuck::Zeroable;
use spin::Mutex;

pub struct IsoDir<'a, T: Seek> {
    pub(crate) reader: &'a Mutex<IsoCursor<T>>,
    pub(crate) directory: DirectoryRef,
}

pub struct IsoDirIter<'a, T: Read + Seek> {
    pub(crate) reader: &'a Mutex<IsoCursor<T>>,
    pub(crate) directory: DirectoryRef,
    pub(crate) offset: usize,
}

impl<T: Read + Seek> IsoDirIter<'_, T> {
    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl<T: Read + Seek> Iterator for IsoDirIter<'_, T> {
    type Item = io::Result<DirectoryRecord>;
    fn next(&mut self) -> Option<Self::Item> {
        use hadris_io::try_io_result_option;
        let mut reader = self.reader.lock();
        if self.offset >= self.directory.size {
            return None;
        }

        try_io_result_option!(reader.seek(SeekFrom::Start(
            (self.directory.extent.0 as u64) * 2048 + (self.offset as u64),
        )));
        let record = try_io_result_option!(DirectoryRecord::parse(reader.deref_mut()));
        // SPEC: We should check if the flag for the NOT_FINAL bit instaed of checking the length (but probably check it as well)
        if record.size() == 0 {
            return None;
        }
        self.offset += record.size();

        Some(Ok(record))
    }
}

impl<'a, T: Read + Seek> IsoDir<'a, T> {
    pub fn entries(&self) -> IsoDirIter<'_, T> {
        IsoDirIter {
            reader: self.reader,
            directory: self.directory,
            offset: 0,
        }
    }
}
