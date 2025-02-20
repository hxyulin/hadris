use crate::{File, FileSystem, UtcTime};

pub trait FileSystemRead: FileSystem {
    /// This needs to be a mutable reference, because the filesystem might need to update metadata
    /// of the file
    fn read(&mut self, file: &File, buffer: &mut [u8], time: UtcTime) -> Result<usize, ()>;
}

pub trait FileSystemWrite: FileSystem {
    fn write(&mut self, file: &File, buffer: &[u8], time: UtcTime) -> Result<usize, ()>;
}

pub trait FileSystemFull: FileSystem + FileSystemRead + FileSystemWrite {}

impl<T: FileSystem + FileSystemRead + FileSystemWrite> FileSystemFull for T {}
