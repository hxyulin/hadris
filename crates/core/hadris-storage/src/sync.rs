use hadris_io::sync::{Read, Seek, Write};
use hadris_io::{ErrorKind, SeekFrom};

use crate::PartitionView;
use crate::{BlockCount, BlockGeometry, BlockIndex, BlockRange, Error, Result};

impl<S> Read for PartitionView<'_, S>
where
    S: Read + Seek<Error = <S as Read>::Error>,
{
    type Error = hadris_io::ErrorKind;

    fn read(&mut self, buffer: &mut [u8]) -> hadris_io::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source
            .seek(SeekFrom::Start(absolute))
            .map_err(hadris_io::Error::erase)?;
        let read = self
            .source
            .read(&mut buffer[..length])
            .map_err(hadris_io::Error::erase)?;
        self.position += read as u64;
        Ok(read)
    }
}

impl<S> Seek for PartitionView<'_, S>
where
    S: Read + Seek<Error = <S as Read>::Error>,
{
    type Error = hadris_io::ErrorKind;

    fn seek(&mut self, from: SeekFrom) -> hadris_io::Result<u64> {
        self.position = self.seek_position(from)?;
        Ok(self.position)
    }
}

impl<S> Write for PartitionView<'_, S>
where
    S: Read + Write<Error = <S as Read>::Error> + Seek<Error = <S as Read>::Error>,
{
    type Error = hadris_io::ErrorKind;

    fn write(&mut self, buffer: &[u8]) -> hadris_io::Result<usize> {
        let length = buffer.len().min(self.remaining());
        if length == 0 {
            return Ok(0);
        }
        let absolute = self.absolute_position()?;
        self.source
            .seek(SeekFrom::Start(absolute))
            .map_err(hadris_io::Error::erase)?;
        let written = self
            .source
            .write(&buffer[..length])
            .map_err(hadris_io::Error::erase)?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> hadris_io::Result<()> {
        self.source.flush().map_err(hadris_io::Error::erase)
    }
}

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
            return Err(Error::InvalidBufferLength {
                length,
                block_size: block_size as u32,
            });
        }

        let count = u64::try_from(length / block_size).map_err(|_| Error::AddressOverflow)?;
        let range = BlockRange::new(start, BlockCount(count));
        if !self.geometry.contains(range) {
            return Err(Error::OutOfBounds {
                start: start.0,
                count,
                device_blocks: self.geometry.block_count.0,
            });
        }

        start
            .0
            .checked_mul(block_size as u64)
            .ok_or(Error::AddressOverflow)
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
            .map_err(Error::Io)?;

        let mut read = 0;
        while read < buffer.len() {
            match self.inner.read(&mut buffer[read..]) {
                Ok(0) => {
                    return Err(Error::Io(hadris_io::Error::from_kind(
                        ErrorKind::UnexpectedEof,
                    )));
                }
                Ok(count) => read += count,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(Error::Io(error)),
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
            .map_err(Error::Io)?;

        let mut written = 0;
        while written < buffer.len() {
            match self.inner.write(&buffer[written..]) {
                Ok(0) => {
                    return Err(Error::Io(hadris_io::Error::from_kind(ErrorKind::WriteZero)));
                }
                Ok(count) => written += count,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(Error::Io(error)),
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().map_err(Error::Io)
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
            Err(Error::InvalidBufferLength { .. })
        ));
        assert!(matches!(
            device.read_blocks(BlockIndex(2), &mut [0_u8; 4]),
            Err(Error::OutOfBounds { .. })
        ));
    }

    #[test]
    fn partition_view_translates_and_bounds_io() {
        let mut source = std::io::Cursor::new(vec![0_u8, 1, 2, 3, 4, 5, 6, 7]);
        let mut view = PartitionView::new(&mut source, 2, 4).unwrap();

        let mut buffer = [0_u8; 8];
        assert_eq!(view.read(&mut buffer).unwrap(), 4);
        assert_eq!(&buffer[..4], &[2, 3, 4, 5]);
        assert_eq!(view.read(&mut buffer).unwrap(), 0);

        view.seek(SeekFrom::Start(1)).unwrap();
        view.write_all(&[9, 8]).unwrap();
        let source = view.into_inner();
        assert_eq!(&source.get_ref()[..], &[0, 1, 2, 9, 8, 5, 6, 7]);
    }

    #[test]
    fn partition_view_rejects_invalid_ranges_and_seeks() {
        let mut source = std::io::Cursor::new(vec![0_u8; 8]);
        assert!(PartitionView::new(&mut source, u64::MAX, 2).is_err());
        assert!(PartitionView::new(&mut source, 0, 0).is_err());

        let mut view = PartitionView::new(&mut source, 2, 4).unwrap();
        assert!(view.seek(SeekFrom::Start(5)).is_err());
        assert!(view.seek(SeekFrom::End(-5)).is_err());
    }
}
