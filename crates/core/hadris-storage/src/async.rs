use hadris_io::r#async::{Read, Seek, Write};
use hadris_io::{ErrorKind, SeekFrom};

use crate::{BlockCount, BlockError, BlockGeometry, BlockIndex, BlockRange, Result};

/// Asynchronous read capability for logical block devices.
pub trait BlockDevice {
    /// Error type produced by the underlying device.
    type Error: embedded_io::Error;

    /// Returns the device geometry.
    fn geometry(&self) -> BlockGeometry;

    /// Reads whole logical blocks starting at `start`.
    async fn read_blocks(
        &mut self,
        start: BlockIndex,
        buffer: &mut [u8],
    ) -> Result<(), Self::Error>;
}

/// Asynchronous write capability for logical block devices.
pub trait BlockDeviceMut: BlockDevice {
    /// Writes whole logical blocks starting at `start`.
    async fn write_blocks(&mut self, start: BlockIndex, buffer: &[u8]) -> Result<(), Self::Error>;

    /// Flushes pending device writes.
    async fn flush(&mut self) -> Result<(), Self::Error>;
}

/// Adapts an asynchronous seekable byte stream to a logical block device.
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
        if !self
            .geometry
            .contains(BlockRange::new(start, BlockCount(count)))
        {
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

    async fn read_blocks(
        &mut self,
        start: BlockIndex,
        buffer: &mut [u8],
    ) -> Result<(), Self::Error> {
        let offset = self.checked_request(start, buffer.len())?;
        self.inner
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(BlockError::Io)?;
        let mut read = 0;
        while read < buffer.len() {
            match self.inner.read(&mut buffer[read..]).await {
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
    async fn write_blocks(&mut self, start: BlockIndex, buffer: &[u8]) -> Result<(), Self::Error> {
        let offset = self.checked_request(start, buffer.len())?;
        self.inner
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(BlockError::Io)?;
        let mut written = 0;
        while written < buffer.len() {
            match self.inner.write(&buffer[written..]).await {
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

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().await.map_err(BlockError::Io)
    }
}
