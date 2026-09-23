use crate::SeekFrom;

/// Use a `std::io` type as a Hadris device.
///
/// Implements the Hadris sync traits, and the `embedded-io` traits, for
/// whichever of `std::io::Read`, `BufRead`, `Write` and `Seek` the inner type
/// implements. Errors are the `std::io::Error` itself.
///
/// ```rust
/// use hadris_io::{Read, StdIo};
///
/// let mut io = StdIo::new(std::io::Cursor::new(b"abc".to_vec()));
/// let mut buf = [0u8; 3];
/// io.read_exact(&mut buf).unwrap();
/// assert_eq!(&buf, b"abc");
/// ```
#[derive(Debug, Clone, Default)]
pub struct StdIo<T: ?Sized>(T);

impl<T> StdIo<T> {
    /// Wrap a `std::io` value.
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }

    /// Recover the wrapped value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: ?Sized> StdIo<T> {
    /// Borrow the wrapped value.
    pub const fn get_ref(&self) -> &T {
        &self.0
    }

    /// Mutably borrow the wrapped value.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: ?Sized> embedded_io::ErrorType for StdIo<T> {
    type Error = std::io::Error;
}

impl<T: ?Sized> crate::ErrorType for StdIo<T> {
    type Error = std::io::Error;
}

impl<T: std::io::Read + ?Sized> embedded_io::Read for StdIo<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<T: std::io::BufRead + ?Sized> embedded_io::BufRead for StdIo<T> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.0.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.0.consume(amt);
    }
}

impl<T: std::io::Write + ?Sized> embedded_io::Write for StdIo<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl<T: std::io::Seek + ?Sized> embedded_io::Seek for StdIo<T> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos.into())
    }
}

/// Use a Hadris reader, writer or seeker as a `std::io` type.
///
/// ```rust
/// use hadris_io::{Cursor, ToStd};
/// use std::io::Read;
///
/// let mut reader = ToStd::new(Cursor::new(b"abc"));
/// let mut out = String::new();
/// reader.read_to_string(&mut out).unwrap();
/// assert_eq!(out, "abc");
/// ```
#[cfg(feature = "sync")]
#[derive(Debug, Clone, Default)]
pub struct ToStd<T: ?Sized>(T);

#[cfg(feature = "sync")]
impl<T> ToStd<T> {
    /// Wrap a Hadris I/O value.
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }

    /// Recover the wrapped value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

#[cfg(feature = "sync")]
impl<T: ?Sized> ToStd<T> {
    /// Borrow the wrapped value.
    pub const fn get_ref(&self) -> &T {
        &self.0
    }

    /// Mutably borrow the wrapped value.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[cfg(feature = "sync")]
impl<T: crate::sync::Read + ?Sized> std::io::Read for ToStd<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf).map_err(crate::into_std_error)
    }
}

#[cfg(feature = "sync")]
impl<T: crate::sync::Write + ?Sized> std::io::Write for ToStd<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf).map_err(crate::into_std_error)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush().map_err(crate::into_std_error)
    }
}

#[cfg(feature = "sync")]
impl<T: crate::sync::Seek + ?Sized> std::io::Seek for ToStd<T> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos.into()).map_err(crate::into_std_error)
    }
}

#[cfg(feature = "sync")]
impl<T: std::io::Read + ?Sized> crate::sync::Read for StdIo<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

#[cfg(feature = "sync")]
impl<T: std::io::Write + ?Sized> crate::sync::Write for StdIo<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

#[cfg(feature = "sync")]
impl<T: std::io::Seek + ?Sized> crate::sync::Seek for StdIo<T> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos.into())
    }
}
