use super::base;
use crate::FromEmbedded;
use crate::{Error, ErrorKind, Result, SeekFrom};

io_transform! {

/// Read bytes from a source.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`. Wrap an
/// `embedded-io` device in [`FromEmbedded`] and a `std::io` type in
/// [`StdIo`](crate::StdIo).
pub trait Read {
    /// Read some bytes, returning zero at end of input.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Fill `buf`, retrying interrupted operations.
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut read = 0;
        while read < buf.len() {
            match self.read(&mut buf[read..]).await {
                Ok(0) => return Err(Error::from_kind(ErrorKind::UnexpectedEof)),
                Ok(n) => read += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl<T: Read + ?Sized> Read for &mut T {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        T::read(self, buf).await
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        T::read_exact(self, buf).await
    }
}

#[cfg(feature = "alloc")]
impl<T: Read + ?Sized> Read for alloc::boxed::Box<T> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        T::read(self, buf).await
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        T::read_exact(self, buf).await
    }
}

impl<T: base::Read> Read for FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        base::Read::read(&mut self.0, buf).await.map_err(Error::from_io)
    }
}

/// Write bytes to a destination.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`.
pub trait Write {
    /// Write some bytes.
    async fn write(&mut self, buf: &[u8]) -> Result<usize>;

    /// Flush buffered output.
    async fn flush(&mut self) -> Result<()>;

    /// Write all bytes, retrying interrupted operations.
    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut written = 0;
        while written < buf.len() {
            match self.write(&buf[written..]).await {
                Ok(0) => return Err(Error::from_kind(ErrorKind::WriteZero)),
                Ok(n) => written += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl<T: Write + ?Sized> Write for &mut T {
    async fn write(&mut self, buf: &[u8]) -> Result<usize> {
        T::write(self, buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        T::flush(self).await
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        T::write_all(self, buf).await
    }
}

#[cfg(feature = "alloc")]
impl<T: Write + ?Sized> Write for alloc::boxed::Box<T> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize> {
        T::write(self, buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        T::flush(self).await
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        T::write_all(self, buf).await
    }
}

impl<T: base::Write> Write for FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize> {
        base::Write::write(&mut self.0, buf).await.map_err(Error::from_io)
    }

    async fn flush(&mut self) -> Result<()> {
        base::Write::flush(&mut self.0).await.map_err(Error::from_io)
    }
}

/// Move within a stream.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`.
pub trait Seek {
    /// Seek to a new byte position.
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

    /// Return the current byte position.
    async fn stream_position(&mut self) -> Result<u64> {
        self.seek(SeekFrom::Current(0)).await
    }

    /// Seek relative to the current byte position.
    async fn seek_relative(&mut self, offset: i64) -> Result<()> {
        self.seek(SeekFrom::Current(offset)).await?;
        Ok(())
    }

    /// Seek to the start of the stream.
    async fn rewind(&mut self) -> Result<()> {
        self.seek(SeekFrom::Start(0)).await?;
        Ok(())
    }
}

impl<T: Seek + ?Sized> Seek for &mut T {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        T::seek(self, pos).await
    }

    async fn stream_position(&mut self) -> Result<u64> {
        T::stream_position(self).await
    }
}

#[cfg(feature = "alloc")]
impl<T: Seek + ?Sized> Seek for alloc::boxed::Box<T> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        T::seek(self, pos).await
    }

    async fn stream_position(&mut self) -> Result<u64> {
        T::stream_position(self).await
    }
}

impl<T: base::Seek> Seek for FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        base::Seek::seek(&mut self.0, pos).await.map_err(Error::from_io)
    }
}

/// A reader that can seek.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek + ?Sized> ReadSeek for T {}

/// A reader that can write.
pub trait ReadWrite: Read + Write {}
impl<T: Read + Write + ?Sized> ReadWrite for T {}

/// A reader that can write and seek.
pub trait ReadWriteSeek: Read + Write + Seek {}
impl<T: Read + Write + Seek + ?Sized> ReadWriteSeek for T {}

/// Structured-reading helpers.
pub trait ReadExt: Read {
    /// Read an arbitrary-bit-pattern value.
    async fn read_struct<T: bytemuck::AnyBitPattern + bytemuck::NoUninit>(&mut self) -> Result<T> {
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

/// Parse a value from a reader.
pub trait Parsable: Sized {
    /// Parse `Self` from `reader`.
    async fn parse<R: Read>(reader: &mut R) -> Result<Self>;
}

/// Write a value to a writer.
pub trait Writable: Sized {
    /// Write `Self` to `writer`.
    async fn write<W: Write>(&self, writer: &mut W) -> Result<()>;
}

/// A positional source of bytes with a known length.
///
/// Writers use it for file contents so they can read the same bytes more than
/// once without a seek contract.
pub trait ByteSource {
    /// Total length in bytes.
    fn len(&self) -> u64;

    /// Whether the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read bytes starting at `offset`. Returns zero at or past the end.
    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize>;

    /// Fill `buf` from `offset`, failing with `UnexpectedEof` if the source ends first.
    async fn read_exact_at(&mut self, mut offset: u64, buf: &mut [u8]) -> Result<()> {
        let mut read = 0;
        while read < buf.len() {
            match self.read_at(offset, &mut buf[read..]).await {
                Ok(0) => return Err(Error::from_kind(ErrorKind::UnexpectedEof)),
                Ok(n) => {
                    read += n;
                    offset += n as u64;
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl ByteSource for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        Ok(crate::copy_from_slice_at(self, offset, buf))
    }
}

#[cfg(feature = "alloc")]
impl ByteSource for alloc::vec::Vec<u8> {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        Ok(crate::copy_from_slice_at(self, offset, buf))
    }
}

impl<S: ByteSource + ?Sized> ByteSource for &mut S {
    fn len(&self) -> u64 {
        S::len(self)
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        S::read_at(self, offset, buf).await
    }
}

/// A [`ByteSource`] over a seekable reader.
#[derive(Debug)]
pub struct SeekSource<T> {
    inner: T,
    len: u64,
}

impl<T: Read + Seek> SeekSource<T> {
    /// Wrap `inner`, measuring its length by seeking to the end.
    pub async fn new(mut inner: T) -> Result<Self> {
        let len = inner.seek(SeekFrom::End(0)).await?;
        Ok(Self { inner, len })
    }

    /// Wrap `inner` with a known length.
    pub fn with_len(inner: T, len: u64) -> Self {
        Self { inner, len }
    }

    /// Recover the reader.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: Read + Seek> ByteSource for SeekSource<T> {
    fn len(&self) -> u64 {
        self.len
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let limit = usize::try_from(self.len - offset).unwrap_or(usize::MAX).min(buf.len());
        self.inner.seek(SeekFrom::Start(offset)).await?;
        self.inner.read(&mut buf[..limit]).await
    }
}

}
