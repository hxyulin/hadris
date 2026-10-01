use crate::{ErrorType, ExactError, SeekFrom};

io_transform! {

/// A byte source.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`. Wrap an
/// `embedded-io` device in `FromEmbedded` (with the `embedded-io` feature)
/// and a `std::io` type in `StdIo`.
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

local_only! {
#[cfg(feature = "embedded-io")]
impl<T: super::base::Read> Read for crate::FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        super::base::Read::read(&mut self.0, buf).await
    }
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

local_only! {
#[cfg(feature = "embedded-io")]
impl<T: super::base::Write> Write for crate::FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        super::base::Write::write(&mut self.0, buf).await
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        super::base::Write::flush(&mut self.0).await
    }
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

local_only! {
#[cfg(feature = "embedded-io")]
impl<T: super::base::Seek> Seek for crate::FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        super::base::Seek::seek(&mut self.0, pos.into()).await
    }
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

}
