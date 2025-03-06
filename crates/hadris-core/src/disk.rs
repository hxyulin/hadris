//! This module contains structures and functions for working with disks.
//!
//! Disks are represented by the [`DiskReader`] and [`DiskWriter`] traits, which are implemented
//! for byte slices and vectors by default. The errors returned by these traits are [`DiskError`].

/// Errors that can occur when reading or writing to a disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DiskError {
    /// The undex that was requested is out of bounds, e.g. the disk is smaller than the requested
    /// This should never happen. If this does happen, there is a bug in the code.
    #[error("Index out of bounds")]
    OutOfBounds,
    /// An error occurred while reading or writing to the disk. This can happen randomly at any
    /// time, especially for hard drives, and should be handled by the caller.
    #[error("Disk error")]
    DiskError,
}

/// A trait for reading to a disk.
///
/// Implementations of this trait can be used to read from a disk. Reads are always done in 512-byte sectors,
/// but this is due to change in the future, if more performance is needed.
/// The struct implementing this trait should hold a reference to the data, or other means of
/// ensuring that the data is not modified while being read, as this can lead to undefined behavior, as well
/// as a non functional file system. In the future, this may be changed in favor for a more
/// flexible appraoch, using some sort of notification system, to notify the file system when the data
/// is modified.
/// See [`DiskWriter`] for writing to a disk.
///
/// # Examples
/// ```
/// use hadris_core::disk::{DiskReader, DiskError};
///
/// // This would be a real disk
/// let mut disk = [0; 1024];
/// let mut reader = &mut disk[..];
/// let mut buffer = [0; 512];
///
/// // Read the first sector
/// reader.read_sector(0, &mut buffer)?;
///
/// // Read the second sector
/// reader.read_sector(1, &mut buffer)?;
/// # Ok::<(), DiskError>(())
/// ```
pub trait DiskReader {
    /// Reads a sector from the disk into the given buffer.
    ///
    /// # Errors
    /// This function will return an error if the requested sector is out of bounds, or if there is
    /// an error while reading from the disk.
    fn read_sector(&mut self, sector: u32, buffer: &mut [u8; 512]) -> Result<(), DiskError>;

    fn read_bytes(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), DiskError> {
        let mut temp_buffer = [0u8; 512];
        let sector = offset / 512;
        let offset = offset % 512;
        self.read_sector(sector as u32, &mut temp_buffer)?;
        buffer.copy_from_slice(&temp_buffer[offset..offset + buffer.len()]);
        Ok(())
    }
}

/// A trait for writing to a disk.
///
/// Implementations of this trait can be used to write to a disk. Writes are always done in 512-byte sectors,
/// but this is due to change in the future, if more performance is needed.
/// See [`DiskReader`] for reading from a disk.
///
/// # Examples
/// ```
/// use hadris_core::disk::{DiskWriter, DiskError};
///
/// // This would be a real disk
/// let mut disk = [0; 1024];
/// let mut writer = &mut disk[..];
/// let mut buffer = [0; 512];
///
/// // Write the first sector
/// writer.write_sector(0, &buffer)?;
///
/// // Write the second sector
/// writer.write_sector(1, &buffer)?;
/// # Ok::<(), DiskError>(())
/// ```
pub trait DiskWriter {
    /// Writes a sector to the disk from the given buffer.
    ///
    /// # Errors
    /// This function will return an error if the requested sector is out of bounds, or if there is
    /// an error while writing to the disk.
    fn write_sector(&mut self, sector: u32, buffer: &[u8; 512]) -> Result<(), DiskError>;

    fn write_bytes(&mut self, offset: usize, buffer: &[u8]) -> Result<(), DiskError> {
        let mut temp_buffer = [0u8; 512];
        let sector = offset / 512;
        let offset = offset % 512;
        self.write_sector(sector as u32, &temp_buffer)?;
        temp_buffer[offset..offset + buffer.len()].copy_from_slice(buffer);
        self.write_sector(sector as u32, &temp_buffer)?;
        Ok(())
    }
}

/// A unified trait for [`DiskReader`] and [`DiskWriter`].
pub trait Disk: DiskReader + DiskWriter {}

/// Implementations of [`DiskReader`] and [`DiskWriter`] for byte slices.
#[doc(hidden)]
mod impls {
    use super::*;

    impl DiskReader for &[u8] {
        fn read_sector(&mut self, sector: u32, buffer: &mut [u8; 512]) -> Result<(), DiskError> {
            let offset = sector as usize * 512;
            if offset + buffer.len() > self.len() {
                return Err(DiskError::OutOfBounds);
            }
            let len = buffer.len();
            buffer.copy_from_slice(&self[offset..offset + len]);
            Ok(())
        }

        fn read_bytes(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), DiskError> {
            let len = buffer.len();
            if offset + len > self.len() {
                return Err(DiskError::OutOfBounds);
            }
            buffer.copy_from_slice(&self[offset..offset + len]);
            Ok(())
        }
    }

    impl DiskReader for &mut [u8] {
        fn read_sector(&mut self, sector: u32, buffer: &mut [u8; 512]) -> Result<(), DiskError> {
            let offset = sector as usize * 512;
            if offset + buffer.len() > self.len() {
                return Err(DiskError::OutOfBounds);
            }
            let len = buffer.len();
            buffer.copy_from_slice(&self[offset..offset + len]);
            Ok(())
        }

        fn read_bytes(&mut self, offset: usize, buffer: &mut [u8]) -> Result<(), DiskError> {
            let len = buffer.len();
            if offset + len > self.len() {
                return Err(DiskError::OutOfBounds);
            }
            buffer.copy_from_slice(&self[offset..offset + len]);
            Ok(())
        }
    }

    impl DiskWriter for &mut [u8] {
        fn write_sector(&mut self, sector: u32, buffer: &[u8; 512]) -> Result<(), DiskError> {
            let offset = sector as usize * 512;
            if offset + buffer.len() > self.len() {
                return Err(DiskError::OutOfBounds);
            }
            self[offset..offset + buffer.len()].copy_from_slice(buffer);
            Ok(())
        }

        fn write_bytes(&mut self, offset: usize, buffer: &[u8]) -> Result<(), DiskError> {
            let len = buffer.len();
            if offset + len > self.len() {
                return Err(DiskError::OutOfBounds);
            }
            self[offset..offset + len].copy_from_slice(buffer);
            Ok(())
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_disk_writer() {
        let mut disk = [0u8; 1024];
        let mut writer = &mut disk[..];
        writer.write_sector(0, &[0xFF; 512]).unwrap();
        writer.write_sector(1, &[0xFF; 512]).unwrap();
        assert_eq!(disk[0..512], [0xFF; 512]);
        assert_eq!(disk[512..1024], [0xFF; 512]);

        let mut writer = &mut disk[..];
        writer.write_bytes(0, &[0xEE; 16]).unwrap();
        writer.write_bytes(16, &[0xFF; 16]).unwrap();
        assert_eq!(disk[0..16], [0xEE; 16]);
        assert_eq!(disk[16..32], [0xFF; 16]);
    }

    #[test]
    fn test_disk_reader() {
        let mut disk = [0u8; 1024];
        let mut reader = &mut disk[..];
        reader.write_sector(0, &[0xFF; 512]).unwrap();
        reader.write_sector(1, &[0xFF; 512]).unwrap();
        let mut buffer = [0u8; 512];
        reader.read_sector(0, &mut buffer).unwrap();
        assert_eq!(buffer, [0xFF; 512]);
        reader.read_sector(1, &mut buffer).unwrap();
        assert_eq!(buffer, [0xFF; 512]);

        let mut reader = &mut disk[..];
        reader.write_bytes(0, &[0xEE; 16]).unwrap();
        reader.write_bytes(16, &[0xFF; 16]).unwrap();
        let mut buffer = [0u8; 16];
        reader.read_bytes(0, &mut buffer).unwrap();
        assert_eq!(buffer, [0xEE; 16]);
        reader.read_bytes(16, &mut buffer).unwrap();
        assert_eq!(buffer, [0xFF; 16]);
    }
}
