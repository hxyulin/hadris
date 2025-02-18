use crate::{FileSystem, File};

pub trait FileSystemRead: FileSystem {
    fn read(&self, file: &File, buffer: &mut [u8]) -> Result<usize, ()>;
}

pub trait FileSystemWrite: FileSystem {
    fn write(&mut self, file: &File, buffer: &[u8]) -> Result<usize, ()>;
}

pub trait FileSystemFull: FileSystem + FileSystemRead + FileSystemWrite {}

impl<T: FileSystem + FileSystemRead + FileSystemWrite> FileSystemFull for T {}
