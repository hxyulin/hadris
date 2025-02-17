#![cfg_attr(not(feature = "std"), no_std)]

use std::sync::Mutex;
#[cfg(feature = "alloc")]
extern crate alloc;

#[derive(Debug, Clone, Copy)]
pub enum OpenMode {
    Read,
    Write,
    Append,
}

pub trait FileSystem {
    fn open(&self, path: &str, mode: OpenMode) -> Result<File, ()>;
}

pub trait FileSystemRead: FileSystem {
    fn read(&self, file: &File, buffer: &mut [u8]) -> Result<usize, ()>;
}

pub trait FileSystemWrite: FileSystem {
    fn write(&self, file: &File, buffer: &[u8]) -> Result<usize, ()>;
}

#[derive(Debug)]
pub struct File {
    descriptor: u32,
    seek: Mutex<u32>,
}

impl File {
    /// # Safety
    ///
    /// The descriptor must be valid
    pub unsafe fn with_descriptor(descriptor: u32) -> Self {
        Self {
            descriptor,
            seek: Mutex::new(0),
        }
    }

    pub fn read(&self, fs: &dyn FileSystemRead, buffer: &mut [u8]) -> Result<usize, ()> {
        fs.read(self, buffer)
    }

    pub fn write(&self, fs: &dyn FileSystemWrite, buffer: &[u8]) -> Result<usize, ()> {
        fs.write(self, buffer)
    }

    pub fn descriptor(&self) -> u32 {
        self.descriptor
    }
}
