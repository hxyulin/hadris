use hadris_io::sync::{Read, Seek, Write};
use hadris_io::{ErrorKind, SeekFrom};

use crate::{BlockCount, BlockError, BlockGeometry, BlockIndex, BlockRange, Result};

/// Synchronous read capability for logical block devices.
pub trait BlockDevice {
    /// Error type produced by the underlying device.
    type Error: embedded_io::Error;

    /// Returns the device geometry.
    fn geometry(&self) -> BlockGeometry;

    /// Reads whole logical blocks starting at `start`.
    fn read_blocks(&mut self, start: BlockIndex, buffer: &mut [u8]) -> Result<(), Self::Error>;
}

/// Synchronous write capability for logical block devices.
pub trait BlockDeviceMut: BlockDevice {
    /// Writes whole logical blocks starting at `start`.
    fn write_blocks(&mut self, start: BlockIndex, buffer: &[u8]) -> Result<(), Self::Error>;

    /// Flushes pending device writes.
    fn flush(&mut self) -> Result<(), Self::Error>;
}

/// Adapts a seekable byte stream to a logical block device.
#[derive(Debug)]
pub struct SeekBlockDevice<T> {
    inner: T,
    geometry: BlockGeometry,
}

impl<T> SeekBlockDevice<T> {
    /// Creates an adapter with explicit geometry.
    pub const fn new(inner: T, geometry: BlockGeometry) -> Self {
        Self { inner, geometry }
    }

    /// Returns a shared reference to the byte stream.
    pub const fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Returns a mutable reference to the byte stream.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consumes the adapter and returns the byte stream.
    pub fn into_inner(self) -> T {
        self.inner
    }

    fn checked_request<E>(&self, start: BlockIndex, length: usize) -> Result<u64, E> {
        let block_size = self.geometry.logical_block_size.get() as usize;
        if length == 0 || !length.is_multiple_of(block_size) {
            return Err(BlockError::InvalidBufferLength {
                length,
                block_size: block_size as u32,
            });
        }

        let count = u64::try_from(length / block_size).map_err(|_| BlockError::AddressOverflow)?;
        let range = BlockRange::new(start, BlockCount(count));
        if !self.geometry.contains(range) {
            return Err(BlockError::OutOfBounds {
                start: start.0,
                count,
                device_blocks: self.geometry.block_count.0,
            });
        }

        start
            .0
            .checked_mul(block_size as u64)
            .ok_or(BlockError::AddressOverflow)
    }
}

impl<T> BlockDevice for SeekBlockDevice<T>
where
    T: Read + Seek<Error = <T as Read>::Error>,
{
    type Error = <T as Read>::Error;

    fn geometry(&self) -> BlockGeometry {
        self.geometry
    }

    fn read_blocks(&mut self, start: BlockIndex, buffer: &mut [u8]) -> Result<(), Self::Error> {
        let offset = self.checked_request(start, buffer.len())?;
        self.inner
            .seek(SeekFrom::Start(offset))
            .map_err(BlockError::Io)?;

        let mut read = 0;
        while read < buffer.len() {
            match self.inner.read(&mut buffer[read..]) {
                Ok(0) => {
                    return Err(BlockError::Io(hadris_io::Error::from_kind(
                        ErrorKind::UnexpectedEof,
                    )));
                }
                Ok(count) => read += count,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(BlockError::Io(error)),
            }
        }
        Ok(())
    }
}

impl<T> BlockDeviceMut for SeekBlockDevice<T>
where
    T: Read + Write<Error = <T as Read>::Error> + Seek<Error = <T as Read>::Error>,
{
    fn write_blocks(&mut self, start: BlockIndex, buffer: &[u8]) -> Result<(), Self::Error> {
        let offset = self.checked_request(start, buffer.len())?;
        self.inner
            .seek(SeekFrom::Start(offset))
            .map_err(BlockError::Io)?;

        let mut written = 0;
        while written < buffer.len() {
            match self.inner.write(&buffer[written..]) {
                Ok(0) => {
                    return Err(BlockError::Io(hadris_io::Error::from_kind(
                        ErrorKind::WriteZero,
                    )));
                }
                Ok(count) => written += count,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(BlockError::Io(error)),
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().map_err(BlockError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn reads_and_writes_aligned_blocks() {
        let geometry = BlockGeometry::new(crate::BlockSize::new(4).unwrap(), BlockCount(2));
        let cursor = std::io::Cursor::new(vec![0_u8; 8]);
        let mut device = SeekBlockDevice::new(cursor, geometry);

        device.write_blocks(BlockIndex(1), &[1, 2, 3, 4]).unwrap();
        let mut block = [0_u8; 4];
        device.read_blocks(BlockIndex(1), &mut block).unwrap();
        assert_eq!(block, [1, 2, 3, 4]);
    }

    #[test]
    fn rejects_partial_and_out_of_bounds_requests() {
        let geometry = BlockGeometry::new(crate::BlockSize::new(4).unwrap(), BlockCount(2));
        let cursor = std::io::Cursor::new(vec![0_u8; 8]);
        let mut device = SeekBlockDevice::new(cursor, geometry);

        assert!(matches!(
            device.read_blocks(BlockIndex(0), &mut [0_u8; 3]),
            Err(BlockError::InvalidBufferLength { .. })
        ));
        assert!(matches!(
            device.read_blocks(BlockIndex(2), &mut [0_u8; 4]),
            Err(BlockError::OutOfBounds { .. })
        ));
    }
}
