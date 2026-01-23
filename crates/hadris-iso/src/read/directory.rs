use core::ops::DerefMut;

use crate::{
    io::{self, Read, Seek, SeekFrom, IsoCursor},
    directory::{DirectoryRecord, DirectoryRef},
};
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

        // ISO 9660 directories are padded with zeros at sector boundaries.
        // When we encounter a zero-length record, we need to skip to the next sector
        // and continue reading, rather than stopping iteration.
        const SECTOR_SIZE: usize = 2048;

        loop {
            if self.offset >= self.directory.size {
                return None;
            }

            try_io_result_option!(reader.seek(SeekFrom::Start(
                (self.directory.extent.0 as u64) * SECTOR_SIZE as u64 + (self.offset as u64),
            )));

            // Peek at the record length byte to check for padding
            let mut len_byte = [0u8; 1];
            try_io_result_option!(reader.read_exact(&mut len_byte));

            if len_byte[0] == 0 {
                // Zero-length record indicates sector padding.
                // Skip to the start of the next sector.
                let current_sector_offset = self.offset % SECTOR_SIZE;
                if current_sector_offset == 0 {
                    // Already at sector boundary but got zero - we're done
                    return None;
                }
                let bytes_to_skip = SECTOR_SIZE - current_sector_offset;
                self.offset += bytes_to_skip;
                continue;
            }

            // Seek back to re-read the full record
            try_io_result_option!(reader.seek(SeekFrom::Start(
                (self.directory.extent.0 as u64) * SECTOR_SIZE as u64 + (self.offset as u64),
            )));

            let record = try_io_result_option!(DirectoryRecord::parse(reader.deref_mut()));
            self.offset += record.size();

            return Some(Ok(record));
        }
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
