use super::base;
use crate::{ErrorType, ExactError, FromEmbedded, SeekFrom};

io_transform! {

/// A byte source.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`. Wrap an
/// `embedded-io` device in [`FromEmbedded`] and a `std::io` type in
/// [`StdIo`](crate::StdIo).
pub trait Read: ErrorType {
    /// Reads up to `buf.len()` bytes. Returns 0 only at the end of input or
    /// for an empty `buf`.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Fills `buf`.
    async fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), ExactError<Self::Error>> {
        while !buf.is_empty() {
            match self.read(buf).await.map_err(ExactError::Io)? {
                0 => return Err(ExactError::UnexpectedEof),
                n => buf = &mut buf[n..],
            }
        }
        Ok(())
    }
}

impl<T: Read + ?Sized> Read for &mut T {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        T::read(self, buf).await
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ExactError<Self::Error>> {
        T::read_exact(self, buf).await
    }
}

#[cfg(feature = "alloc")]
impl<T: Read + ?Sized> Read for alloc::boxed::Box<T> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        T::read(self, buf).await
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ExactError<Self::Error>> {
        T::read_exact(self, buf).await
    }
}

impl<T: base::Read> Read for FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        base::Read::read(&mut self.0, buf).await
    }
}

/// A byte sink.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`.
pub trait Write: ErrorType {
    /// Writes up to `buf.len()` bytes. Returns 0 only for an empty `buf`.
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;

    /// Flushes buffered output.
    async fn flush(&mut self) -> Result<(), Self::Error>;

    /// Writes all of `buf`.
    async fn write_all(&mut self, mut buf: &[u8]) -> Result<(), ExactError<Self::Error>> {
        while !buf.is_empty() {
            match self.write(buf).await.map_err(ExactError::Io)? {
                0 => return Err(ExactError::WriteZero),
                n => buf = &buf[n..],
            }
        }
        Ok(())
    }
}

impl<T: Write + ?Sized> Write for &mut T {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        T::write(self, buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        T::flush(self).await
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), ExactError<Self::Error>> {
        T::write_all(self, buf).await
    }
}

#[cfg(feature = "alloc")]
impl<T: Write + ?Sized> Write for alloc::boxed::Box<T> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        T::write(self, buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        T::flush(self).await
    }

    async fn write_all(&mut self, buf: &[u8]) -> Result<(), ExactError<Self::Error>> {
        T::write_all(self, buf).await
    }
}

impl<T: base::Write> Write for FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        base::Write::write(&mut self.0, buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        base::Write::flush(&mut self.0).await
    }
}

/// A seekable stream.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`.
pub trait Seek: ErrorType {
    /// Moves to `pos` and returns the new position from the start.
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error>;

    /// Returns the current position.
    async fn stream_position(&mut self) -> Result<u64, Self::Error> {
        self.seek(SeekFrom::Current(0)).await
    }

    /// Moves to the start.
    async fn rewind(&mut self) -> Result<(), Self::Error> {
        self.seek(SeekFrom::Start(0)).await?;
        Ok(())
    }
}

impl<T: Seek + ?Sized> Seek for &mut T {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        T::seek(self, pos).await
    }

    async fn stream_position(&mut self) -> Result<u64, Self::Error> {
        T::stream_position(self).await
    }
}

#[cfg(feature = "alloc")]
impl<T: Seek + ?Sized> Seek for alloc::boxed::Box<T> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        T::seek(self, pos).await
    }

    async fn stream_position(&mut self) -> Result<u64, Self::Error> {
        T::stream_position(self).await
    }
}

impl<T: base::Seek> Seek for FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        base::Seek::seek(&mut self.0, pos).await
    }
}

impl Read for crate::Cursor<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(self.read_slice(buf))
    }
}

impl Seek for crate::Cursor<'_> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        self.seek_to(pos)
    }
}

/// A positional source of bytes with a known length.
///
/// Writers read file contents through it, so they can read the same bytes
/// more than once without a seek contract.
pub trait ByteSource: ErrorType {
    /// Total length in bytes.
    fn len(&self) -> u64;

    /// Whether the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reads bytes starting at `offset`. Returns 0 at or past the end.
    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Fills `buf` from `offset`.
    async fn read_exact_at(
        &mut self,
        mut offset: u64,
        mut buf: &mut [u8],
    ) -> Result<(), ExactError<Self::Error>> {
        while !buf.is_empty() {
            match self.read_at(offset, buf).await.map_err(ExactError::Io)? {
                0 => return Err(ExactError::UnexpectedEof),
                n => {
                    buf = &mut buf[n..];
                    offset += n as u64;
                }
            }
        }
        Ok(())
    }
}

impl ByteSource for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(crate::copy_from_slice_at(self, offset, buf))
    }
}

#[cfg(feature = "alloc")]
impl ByteSource for alloc::vec::Vec<u8> {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(crate::copy_from_slice_at(self, offset, buf))
    }
}

impl<S: ByteSource + ?Sized> ByteSource for &mut S {
    fn len(&self) -> u64 {
        S::len(self)
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error> {
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
    /// Wraps `inner`, measuring its length by seeking to the end.
    pub async fn new(mut inner: T) -> Result<Self, T::Error> {
        let len = inner.seek(SeekFrom::End(0)).await?;
        Ok(Self { inner, len })
    }

    /// Wraps `inner` with a known length.
    pub fn with_len(inner: T, len: u64) -> Self {
        Self { inner, len }
    }

    /// Returns the reader.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: ErrorType> ErrorType for SeekSource<T> {
    type Error = T::Error;
}

impl<T: Read + Seek> ByteSource for SeekSource<T> {
    fn len(&self) -> u64 {
        self.len
    }

    async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if offset >= self.len {
            return Ok(0);
        }
        let limit = usize::try_from(self.len - offset).unwrap_or(usize::MAX).min(buf.len());
        self.inner.seek(SeekFrom::Start(offset)).await?;
        self.inner.read(&mut buf[..limit]).await
    }
}

}
