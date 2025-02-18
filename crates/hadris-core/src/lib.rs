#![cfg_attr(not(feature = "std"), no_std)]

use std::sync::Mutex;
#[cfg(feature = "alloc")]
extern crate alloc;

pub mod internal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    Read,
    Write,
    Append,
}

pub trait FileSystem {
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<File, ()>;
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

    pub fn read<T: internal::FileSystemRead + ?Sized>(&self, fs: &T, buffer: &mut [u8]) -> Result<usize, ()> {
        fs.read(self, buffer)
    }

    pub fn write<T: internal::FileSystemWrite + ?Sized>(&self, fs: &mut T, buffer: &[u8]) -> Result<usize, ()> {
        fs.write(self, buffer)
    }

    pub fn descriptor(&self) -> u32 {
        self.descriptor
    }

    pub fn seek(&self) -> u32 {
        *self.seek.lock().unwrap()
    }
}
