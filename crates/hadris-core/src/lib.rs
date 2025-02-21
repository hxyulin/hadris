#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub type UtcTime = chrono::DateTime<chrono::Utc>;

pub mod file;
pub mod internal;
pub mod str;
use file::FileAttributes;
pub use file::{File, OpenOptions};

pub trait FileSystem {
    fn create(&mut self, path: &str, attributes: FileAttributes) -> Result<File, ()>;
    fn open(&mut self, path: &str, options: OpenOptions) -> Result<File, ()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWriteError {
    OutOfBounds,
    InvalidSector,
}

impl core::fmt::Display for ReadWriteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfBounds => f.write_str("Index out of bounds"),
            Self::InvalidSector => f.write_str("Invalid sector"),
        }
    }
}

impl core::error::Error for ReadWriteError {}

/// A trait for reading data from a media
/// This trait is used to read data from a media, for a fully functional filesystem,
/// a reader and writer should be implemented
///
/// See `Writer` for more information
pub trait Reader {
    /// Reads a sector from the file system, this can be called multiple times on the same sector
    /// to read the entire sector, so the implementation should be able to handle this.
    fn read_sector(&mut self, sector: u32, buffer: &mut [u8; 512]) -> Result<(), ReadWriteError>;
    /// Reads bytes from the file system, this can be called multiple times on the same sector
    /// This is ganranteed to be less than a sector
    fn read_bytes(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), ReadWriteError> {
        if offset + buffer.len() > 512 {
            return Err(ReadWriteError::OutOfBounds);
        }
        let mut sector_buf: [u8; 512] = [0; 512];
        let sector = offset / 512;
        let offset = offset % 512;
        self.read_sector(sector as u32, &mut sector_buf)?;
        buffer.copy_from_slice(&sector_buf[offset..buffer.len() + offset]);
        Ok(())
    }
}

/// A trait for writing data to a media
pub trait Writer {
    /// Writes a sector to the file system, this can be called multiple times on the same sector
    /// to write the entire sector, so the implementation should be able to handle this.
    fn write_sector(&mut self, sector: u32, buffer: &[u8; 512]) -> Result<(), ReadWriteError>;

    /// Writes bytes to the file system, this can be called multiple times on the same sector
    /// This is ganranteed to be less than a sector
    fn write_bytes(&mut self, offset: usize, buffer: &[u8]) -> Result<(), ReadWriteError> {
        let mut sector_buf: [u8; 512] = [0; 512];
        if offset + buffer.len() > 512 {
            return Err(ReadWriteError::OutOfBounds);
        }
        let sector = offset / 512;
        let offset = offset % 512;
        sector_buf[offset..buffer.len() + offset].copy_from_slice(buffer);
        self.write_sector(sector as u32, &sector_buf)
    }
}

pub trait WriterExt: Writer {
    fn write_stream<T: core::iter::Iterator<Item = u8>>(&mut self, offset: usize, bytes: T) -> Result<(), ReadWriteError> {
        for byte in bytes {
            self.write_bytes(offset, &[byte])?;
        }
        Ok(())
    }
}

impl Reader for &[u8] {
    fn read_sector(&mut self, sector: u32, buffer: &mut [u8; 512]) -> Result<(), ReadWriteError> {
        let offset = sector as usize * 512;
        if offset + buffer.len() > self.len() {
            return Err(ReadWriteError::OutOfBounds);
        }
        let len = buffer.len();
        buffer.copy_from_slice(&self[offset..offset + len]);
        Ok(())
    }

    fn read_bytes(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), ReadWriteError> {
        if offset + buffer.len() > self.len() {
            return Err(ReadWriteError::OutOfBounds);
        }
        buffer.copy_from_slice(&self[offset..offset + buffer.len()]);
        Ok(())
    }
}

impl Writer for &mut [u8] {
    fn write_sector(&mut self, sector: u32, buffer: &[u8; 512]) -> Result<(), ReadWriteError> {
        let offset = sector as usize * 512;
        if offset + buffer.len() > self.len() {
            return Err(ReadWriteError::OutOfBounds);
        }
        self[offset..offset + buffer.len()].copy_from_slice(buffer);
        Ok(())
    }

    fn write_bytes(&mut self, offset: usize, buffer: &[u8]) -> Result<(), ReadWriteError> {
        if offset + buffer.len() > self.len() {
            return Err(ReadWriteError::OutOfBounds);
        }
        self[offset..offset + buffer.len()].copy_from_slice(buffer);
        Ok(())
    }
}
