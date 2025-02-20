use crate::{
    internal::{FileSystemRead, FileSystemWrite},
    UtcTime,
};
use spin::Mutex;

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct OpenOptions: u8 {
        /// Open the file in read only mode
        const READ = 0x01;
        /// Open the file in write mode
        const WRITE = 0x02;
        /// Open the file in append mode
        const APPEND = 0x04;
        /// Create the file when opening if it doesn't exist
        const CREATE = 0x08;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileAttributes: u16 {
        /// A read only file
        const READ_ONLY = 0x01;
        /// A file that should be hidden, should only display with a certain flag
        const HIDDEN = 0x02;
    }
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

    pub fn read_with_timestamp<T: FileSystemRead + ?Sized>(
        &self,
        fs: &mut T,
        buffer: &mut [u8],
        time: UtcTime,
    ) -> Result<usize, ()> {
        fs.read(self, buffer, time)
    }

    pub fn write_with_timestamp<T: FileSystemWrite + ?Sized>(
        &self,
        fs: &mut T,
        buffer: &[u8],
        time: UtcTime,
    ) -> Result<usize, ()> {
        fs.write(self, buffer, time)
    }

    #[cfg(feature = "std")]
    pub fn read<T: FileSystemRead + ?Sized>(
        &self,
        fs: &mut T,
        buffer: &mut [u8],
    ) -> Result<usize, ()> {
        fs.read(self, buffer, std::time::SystemTime::now().into())
    }

    #[cfg(feature = "std")]
    pub fn write<T: FileSystemWrite + ?Sized>(
        &self,
        fs: &mut T,
        buffer: &[u8],
    ) -> Result<usize, ()> {
        fs.write(self, buffer, std::time::SystemTime::now().into())
    }

    pub fn descriptor(&self) -> u32 {
        self.descriptor
    }

    pub fn seek(&self) -> u32 {
        *self.seek.lock()
    }

    pub fn set_seek(&self, seek: u32) {
        *self.seek.lock() = seek;
    }
}
