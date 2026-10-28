use crate::{ErrorType, ExactError, SeekFrom};

io_transform! {

/// A byte source.
///
/// Implemented for `&mut T`, for `&[u8]`, which reads from its front as
/// `std` and `embedded-io` do, and, with `alloc`, `Box<T>`. Wrap an
/// `embedded-io` device in `FromEmbedded` (with the `embedded-io` feature)
/// and a `std::io` type in `StdIo`.
pub trait Read: ErrorType {
    /// Reads up to `buf.len()` bytes. Returns 0 only at the end of input or
    /// for an empty `buf`.
    /// The returned count never exceeds `buf.len()`.
    ///
    /// # Errors
    ///
    /// Returns the source's error if reading fails. An error does not report
    /// how much input was consumed or how much of `buf` was changed.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Fills `buf`.
    ///
    /// On failure, the source may have advanced and `buf` may be partly filled.
    ///
    /// # Errors
    ///
    /// Returns [`ExactError::UnexpectedEof`] if input ends before filling
    /// `buf`, or [`ExactError::Io`] if the source fails.
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

impl Read for &[u8] {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min(self.len());
        let (head, tail) = self.split_at(n);
        buf[..n].copy_from_slice(head);
        *self = tail;
        Ok(n)
    }
}

#[cfg(feature = "embedded-io")]
impl<T: super::base::Read> Read for crate::FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        super::base::Read::read(&mut self.0, buf).await
    }
}

/// A byte sink.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>` and `Vec<u8>`,
/// which appends as `std` and `embedded-io` do.
pub trait Write: ErrorType {
    /// Writes up to `buf.len()` bytes.
    ///
    /// The returned count never exceeds `buf.len()`. A zero count means the
    /// buffer is empty or the sink cannot accept more bytes.
    ///
    /// # Errors
    ///
    /// Returns the sink's error if writing fails. Some bytes may already have
    /// been written; an error does not report progress or promise rollback.
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;

    /// Flushes buffered output.
    ///
    /// # Errors
    ///
    /// Returns the sink's error if buffered output cannot be flushed.
    async fn flush(&mut self) -> Result<(), Self::Error>;

    /// Writes all of `buf`.
    ///
    /// Does not flush the sink. Failure may leave a prefix written; retrying
    /// the entire buffer can repeat bytes already accepted by the sink.
    ///
    /// # Errors
    ///
    /// Returns [`ExactError::WriteZero`] if the sink accepts no bytes before
    /// completion, or [`ExactError::Io`] if the sink fails.
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

#[cfg(feature = "alloc")]
impl Write for alloc::vec::Vec<u8> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

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

/// A seekable stream.
///
/// Implemented for `&mut T` and, with `alloc`, `Box<T>`.
pub trait Seek: ErrorType {
    /// Moves to `pos` and returns the new position from the start.
    ///
    /// # Errors
    ///
    /// Returns the stream's error if the position is invalid or seeking fails.
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error>;

    /// Returns the current position.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`seek`](Self::seek).
    async fn stream_position(&mut self) -> Result<u64, Self::Error> {
        self.seek(SeekFrom::Current(0)).await
    }

    /// Moves to the start.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`seek`](Self::seek).
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

#[cfg(feature = "embedded-io")]
impl<T: super::base::Seek> Seek for crate::FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        super::base::Seek::seek(&mut self.0, pos.into()).await
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
