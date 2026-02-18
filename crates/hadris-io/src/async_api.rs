//! Asynchronous I/O traits using async fn in trait (AFIT).
//!
//! These mirror the synchronous traits in [`crate::sync`] but with `async`
//! method signatures. No blanket impls are provided — async consumers
//! implement these directly for their device types.

use crate::{ErrorKind, Result, SeekFrom};

// ---------------------------------------------------------------------------
// Core I/O traits (async)
// ---------------------------------------------------------------------------

/// Async version of [`crate::sync::Read`].
pub trait Read {
    /// Pull some bytes from this source into the specified buffer.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    /// Read the exact number of bytes required to fill `buf`.
    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut total_read = 0;
        while total_read < buf.len() {
            match self.read(&mut buf[total_read..]).await {
                Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
                Ok(n) => total_read += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Async version of [`crate::sync::Write`].
pub trait Write {
    /// Write a buffer into this writer.
    async fn write(&mut self, buf: &[u8]) -> Result<usize>;

    /// Flush this output stream.
    async fn flush(&mut self) -> Result<()>;

    /// Attempts to write an entire buffer into this writer.
    async fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut written = 0;
        while written < buf.len() {
            match self.write(&buf[written..]).await {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => written += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Async version of [`crate::sync::Seek`].
pub trait Seek {
    /// Seek to an offset, in bytes, in a stream.
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

    /// Returns the current seek position from the start of the stream.
    async fn stream_position(&mut self) -> Result<u64> {
        self.seek(SeekFrom::Current(0)).await
    }

    /// Seek relative to the current position.
    async fn seek_relative(&mut self, offset: i64) -> Result<()> {
        self.seek(SeekFrom::Current(offset)).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Blanket impls for &mut T (mirrors std::io behaviour)
// ---------------------------------------------------------------------------

impl<T: Read> Read for &mut T {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        T::read(self, buf).await
    }

    async fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        T::read_exact(self, buf).await
    }
}

impl<T: Write> Write for &mut T {
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

impl<T: Seek> Seek for &mut T {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        T::seek(self, pos).await
    }

    async fn stream_position(&mut self) -> Result<u64> {
        T::stream_position(self).await
    }

    async fn seek_relative(&mut self, offset: i64) -> Result<()> {
        T::seek_relative(self, offset).await
    }
}

// ---------------------------------------------------------------------------
// Extension traits (async)
// ---------------------------------------------------------------------------

/// Async read extension: structured reads and parsing.
pub trait ReadExt {
    async fn read_struct<T: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<T>;

    async fn parse<T: Parsable>(&mut self) -> Result<T>;
}

impl<T: Read> ReadExt for T {
    async fn read_struct<S: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<S> {
        let mut temp = S::zeroed();
        self.read_exact(bytemuck::bytes_of_mut(&mut temp)).await?;
        Ok(temp)
    }

    async fn parse<S: Parsable>(&mut self) -> Result<S> {
        S::parse(self).await
    }
}

/// Async parse a type from a reader.
pub trait Parsable: Sized {
    async fn parse<R: Read>(reader: &mut R) -> Result<Self>;
}

/// Async write a type to a writer.
pub trait Writable: Sized {
    async fn write<W: Write>(&self, writer: &mut W) -> Result<()>;
}
