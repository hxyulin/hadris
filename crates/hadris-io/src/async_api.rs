//! Asynchronous portable I/O traits and adapters.

use crate::{Error, ErrorKind, Result, SeekFrom};

/// Asynchronously read bytes from a source.
pub trait Read {
    /// Error returned by the source.
    type Error: embedded_io::Error;

    /// Read some bytes.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Fill `buf`, retrying interrupted operations.
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut read = 0;
        while read < buf.len() {
            match self.read(&mut buf[read..]).await {
                Ok(0) => return Err(Error::from_kind(ErrorKind::UnexpectedEof)),
                Ok(n) => read += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.erase()),
            }
        }
        Ok(())
    }
}

/// Asynchronously write bytes to a destination.
pub trait Write {
    /// Error returned by the destination.
    type Error: embedded_io::Error;
    /// Write some bytes.
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;
    /// Flush buffered output.
    async fn flush(&mut self) -> Result<(), Self::Error>;

    /// Write all bytes, retrying interrupted operations.
    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut written = 0;
        while written < buf.len() {
            match self.write(&buf[written..]).await {
                Ok(0) => return Err(Error::from_kind(ErrorKind::WriteZero)),
                Ok(n) => written += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.erase()),
            }
        }
        Ok(())
    }
}

/// Asynchronously move within a stream.
pub trait Seek {
    /// Error returned by the stream.
    type Error: embedded_io::Error;
    /// Seek to a new byte position.
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error>;
    /// Return the current position.
    async fn stream_position(&mut self) -> Result<u64, Self::Error> {
        self.seek(SeekFrom::Current(0)).await
    }
    /// Seek relative to the current position.
    async fn seek_relative(&mut self, offset: i64) -> Result<(), Self::Error> {
        self.seek(SeekFrom::Current(offset)).await?;
        Ok(())
    }
}

/// Async reader and seeker with one common source error.
pub trait ReadSeek: Read + Seek<Error = <Self as Read>::Error> {}
impl<T: Read + Seek<Error = <T as Read>::Error> + ?Sized> ReadSeek for T {}

/// Async reader, writer, and seeker with one common source error.
pub trait ReadWriteSeek:
    Read + Write<Error = <Self as Read>::Error> + Seek<Error = <Self as Read>::Error>
{
}
impl<T> ReadWriteSeek for T where
    T: Read + Write<Error = <T as Read>::Error> + Seek<Error = <T as Read>::Error> + ?Sized
{
}

impl<T: Read + ?Sized> Read for &mut T {
    type Error = T::Error;
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        T::read(self, buf).await
    }
}
impl<T: Write + ?Sized> Write for &mut T {
    type Error = T::Error;
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        T::write(self, buf).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        T::flush(self).await
    }
}
impl<T: Seek + ?Sized> Seek for &mut T {
    type Error = T::Error;
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        T::seek(self, pos).await
    }
}

/// Structured async-reading helpers.
pub trait ReadExt: Read {
    /// Read an arbitrary-bit-pattern value.
    async fn read_struct<T: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<T> {
        let mut temp = T::zeroed();
        self.read_exact(bytemuck::bytes_of_mut(&mut temp)).await?;
        Ok(temp)
    }
    /// Parse a value with its custom parser.
    async fn parse<T: Parsable>(&mut self) -> Result<T>
    where
        Self: Sized,
    {
        T::parse(self).await
    }
}
impl<T: Read + ?Sized> ReadExt for T {}

/// Parse a value from an async reader.
pub trait Parsable: Sized {
    /// Parse `Self` while preserving the source error.
    async fn parse<R: Read>(reader: &mut R) -> Result<Self>;
}

/// Write a value to an async writer.
pub trait Writable: Sized {
    /// Write `Self` while preserving the source error.
    async fn write<W: Write>(&self, writer: &mut W) -> Result<()>;
}

/// Adapt an `embedded-io-async` value to Hadris async traits.
#[derive(Debug, Clone, Copy, Default)]
pub struct FromEmbedded<T>(pub T);

impl<T> FromEmbedded<T> {
    /// Wrap a value.
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }
    /// Recover the wrapped value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

/// Reborrow a Hadris async I/O value.
#[derive(Debug)]
pub struct Borrowed<'a, T: ?Sized>(pub &'a mut T);

impl<'a, T: ?Sized> Borrowed<'a, T> {
    /// Borrow an I/O value.
    pub fn new(inner: &'a mut T) -> Self {
        Self(inner)
    }
}
impl<T: Read + ?Sized> Read for Borrowed<'_, T> {
    type Error = T::Error;
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}
impl<T: Write + ?Sized> Write for Borrowed<'_, T> {
    type Error = T::Error;
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush().await
    }
}
impl<T: Seek + ?Sized> Seek for Borrowed<'_, T> {
    type Error = T::Error;
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        self.0.seek(pos).await
    }
}
impl<T: embedded_io_async::Read> Read for FromEmbedded<T> {
    type Error = T::Error;
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        embedded_io_async::Read::read(&mut self.0, buf)
            .await
            .map_err(Error::from_source)
    }
}
impl<T: embedded_io_async::Write> Write for FromEmbedded<T> {
    type Error = T::Error;
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        embedded_io_async::Write::write(&mut self.0, buf)
            .await
            .map_err(Error::from_source)
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        embedded_io_async::Write::flush(&mut self.0)
            .await
            .map_err(Error::from_source)
    }
}
impl<T: embedded_io_async::Seek> Seek for FromEmbedded<T> {
    type Error = T::Error;
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        embedded_io_async::Seek::seek(&mut self.0, pos)
            .await
            .map_err(Error::from_source)
    }
}
