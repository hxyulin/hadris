//! Synchronous portable I/O traits and interoperability adapters.

use crate::{Error, ErrorKind, Result, SeekFrom};

/// Read bytes from a source.
pub trait Read {
    /// Error returned by the underlying source.
    type Error: embedded_io::Error;

    /// Read some bytes, returning zero at end of input.
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error>;

    /// Fill `buf`, retrying interrupted operations.
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut read = 0;
        while read < buf.len() {
            match self.read(&mut buf[read..]) {
                Ok(0) => return Err(Error::from_kind(ErrorKind::UnexpectedEof)),
                Ok(n) => read += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.erase()),
            }
        }
        Ok(())
    }
}

/// Write bytes to a destination.
pub trait Write {
    /// Error returned by the underlying destination.
    type Error: embedded_io::Error;

    /// Write some bytes.
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error>;

    /// Flush buffered output.
    fn flush(&mut self) -> Result<(), Self::Error>;

    /// Write all bytes, retrying interrupted operations.
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut written = 0;
        while written < buf.len() {
            match self.write(&buf[written..]) {
                Ok(0) => return Err(Error::from_kind(ErrorKind::WriteZero)),
                Ok(n) => written += n,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.erase()),
            }
        }
        Ok(())
    }
}

/// Move within a stream.
pub trait Seek {
    /// Error returned by the underlying stream.
    type Error: embedded_io::Error;

    /// Seek to a new byte position.
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error>;

    /// Return the current byte position.
    fn stream_position(&mut self) -> Result<u64, Self::Error> {
        self.seek(SeekFrom::Current(0))
    }

    /// Seek relative to the current byte position.
    fn seek_relative(&mut self, offset: i64) -> Result<(), Self::Error> {
        self.seek(SeekFrom::Current(offset))?;
        Ok(())
    }
}

/// Reader and seeker that use one common source error.
pub trait ReadSeek: Read + Seek<Error = <Self as Read>::Error> {}
impl<T: Read + Seek<Error = <T as Read>::Error> + ?Sized> ReadSeek for T {}

/// Reader and writer that use one common source error.
pub trait ReadWrite: Read + Write<Error = <Self as Read>::Error> {}
impl<T: Read + Write<Error = <T as Read>::Error> + ?Sized> ReadWrite for T {}

/// Reader, writer, and seeker that use one common source error.
pub trait ReadWriteSeek:
    Read + Write<Error = <Self as Read>::Error> + Seek<Error = <Self as Read>::Error>
{
}
impl<T> ReadWriteSeek for T where
    T: Read + Write<Error = <T as Read>::Error> + Seek<Error = <T as Read>::Error> + ?Sized
{
}

#[cfg(feature = "std")]
impl<T: std::io::Read + ?Sized> Read for T {
    type Error = std::io::Error;

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        std::io::Read::read(self, buf).map_err(Error::from_source)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Write + ?Sized> Write for T {
    type Error = std::io::Error;

    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        std::io::Write::write(self, buf).map_err(Error::from_source)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        std::io::Write::flush(self).map_err(Error::from_source)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Seek + ?Sized> Seek for T {
    type Error = std::io::Error;

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        std::io::Seek::seek(self, pos.into()).map_err(Error::from_source)
    }
}

#[cfg(not(feature = "std"))]
impl<T: embedded_io::Read + ?Sized> Read for T {
    type Error = T::Error;

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        embedded_io::Read::read(self, buf).map_err(Error::from_source)
    }
}

#[cfg(not(feature = "std"))]
impl<T: embedded_io::Write + ?Sized> Write for T {
    type Error = T::Error;

    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        embedded_io::Write::write(self, buf).map_err(Error::from_source)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        embedded_io::Write::flush(self).map_err(Error::from_source)
    }
}

#[cfg(not(feature = "std"))]
impl<T: embedded_io::Seek + ?Sized> Seek for T {
    type Error = T::Error;

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        embedded_io::Seek::seek(self, pos).map_err(Error::from_source)
    }
}

/// Explicitly expose an `embedded-io` value through Hadris traits.
#[derive(Debug, Clone, Copy, Default)]
pub struct FromEmbedded<T>(pub T);

impl<T> FromEmbedded<T> {
    /// Wrap an embedded I/O value.
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }
    /// Recover the wrapped value.
    pub fn into_inner(self) -> T {
        self.0
    }
    /// Borrow the wrapped value.
    pub const fn get_ref(&self) -> &T {
        &self.0
    }
    /// Mutably borrow the wrapped value.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Reborrow a Hadris I/O value without requiring a blanket implementation for `&mut T`.
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
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf)
    }
}
impl<T: Write + ?Sized> Write for Borrowed<'_, T> {
    type Error = T::Error;
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.0.flush()
    }
}
impl<T: Seek + ?Sized> Seek for Borrowed<'_, T> {
    type Error = T::Error;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        self.0.seek(pos)
    }
}

impl<T: embedded_io::Read> Read for FromEmbedded<T> {
    type Error = T::Error;
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        embedded_io::Read::read(&mut self.0, buf).map_err(Error::from_source)
    }
}

impl<T: embedded_io::Write> Write for FromEmbedded<T> {
    type Error = T::Error;
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        embedded_io::Write::write(&mut self.0, buf).map_err(Error::from_source)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        embedded_io::Write::flush(&mut self.0).map_err(Error::from_source)
    }
}

impl<T: embedded_io::Seek> Seek for FromEmbedded<T> {
    type Error = T::Error;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        embedded_io::Seek::seek(&mut self.0, pos).map_err(Error::from_source)
    }
}

/// Explicitly expose a Hadris value through `embedded-io` traits.
#[derive(Debug, Clone, Copy, Default)]
pub struct ToEmbedded<T>(pub T);

impl<T> ToEmbedded<T> {
    /// Wrap a Hadris I/O value.
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }
    /// Recover the wrapped value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Read> embedded_io::ErrorType for ToEmbedded<T> {
    type Error = Error<T::Error>;
}
impl<T: Read> embedded_io::Read for ToEmbedded<T> {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        Read::read(&mut self.0, buf)
    }
}
impl<T> embedded_io::Write for ToEmbedded<T>
where
    T: Read + Write<Error = <T as Read>::Error>,
{
    fn write(&mut self, buf: &[u8]) -> core::result::Result<usize, Self::Error> {
        Write::write(&mut self.0, buf)
    }
    fn flush(&mut self) -> core::result::Result<(), Self::Error> {
        Write::flush(&mut self.0)
    }
}
impl<T> embedded_io::Seek for ToEmbedded<T>
where
    T: Read + Seek<Error = <T as Read>::Error>,
{
    fn seek(&mut self, pos: SeekFrom) -> core::result::Result<u64, Self::Error> {
        Seek::seek(&mut self.0, pos)
    }
}

/// Structured-reading helpers.
pub trait ReadExt: Read {
    /// Read an arbitrary-bit-pattern value.
    fn read_struct<T: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<T> {
        let mut temp = T::zeroed();
        self.read_exact(bytemuck::bytes_of_mut(&mut temp))?;
        Ok(temp)
    }

    /// Parse a value with its custom parser.
    fn parse<T: Parsable>(&mut self) -> Result<T>
    where
        Self: Sized,
    {
        T::parse(self)
    }
}
impl<T: Read + ?Sized> ReadExt for T {}

/// Parse a value from a reader.
pub trait Parsable: Sized {
    /// Parse `Self` while preserving the reader's source error.
    fn parse<R: Read>(reader: &mut R) -> Result<Self>;
}

/// Write a value to a writer.
pub trait Writable: Sized {
    /// Write `Self` while preserving the writer's source error.
    fn write<W: Write>(&self, writer: &mut W) -> Result<()>;
}
