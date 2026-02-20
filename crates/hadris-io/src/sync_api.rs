//! Synchronous I/O traits.
//!
//! When the `std` feature is enabled, [`Read`], [`Write`], and [`Seek`] are
//! re-exported from `std::io` (so any `std::io` implementor works).
//! In no-std mode, minimal custom trait definitions are provided.

use crate::Result;
#[cfg(not(feature = "std"))]
use crate::{Error, ErrorKind, SeekFrom};

// ---------------------------------------------------------------------------
// Traits
// ---------------------------------------------------------------------------

cfg_if::cfg_if! {
    if #[cfg(feature = "std")] {
        pub use std::io::Read;
        pub use std::io::Write;
        pub use std::io::Seek;
    } else {
        /// The `Read` trait allows for reading bytes from a source.
        pub trait Read {
            /// Pull some bytes from this source into the specified buffer,
            /// returning how many bytes were read.
            fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

            /// Read the exact number of bytes required to fill `buf`.
            fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
                let mut total_read = 0;
                while total_read < buf.len() {
                    match self.read(&mut buf[total_read..]) {
                        Ok(0) => return Err(Error::from_kind(ErrorKind::UnexpectedEof)),
                        Ok(n) => total_read += n,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(())
            }
        }

        /// The `Write` trait allows for writing bytes to a destination.
        pub trait Write {
            /// Write a buffer into this writer, returning how many bytes were
            /// written.
            fn write(&mut self, buf: &[u8]) -> Result<usize>;

            /// Flush this output stream, ensuring that all intermediately
            /// buffered contents reach their destination.
            fn flush(&mut self) -> Result<()>;

            /// Attempts to write an entire buffer into this writer.
            fn write_all(&mut self, buf: &[u8]) -> Result<()> {
                let mut written = 0;
                while written < buf.len() {
                    match self.write(&buf[written..]) {
                        Ok(0) => return Err(Error::from_kind(ErrorKind::WriteZero)),
                        Ok(n) => written += n,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(())
            }
        }

        /// The `Seek` trait provides a cursor which can be moved within a
        /// stream of bytes.
        pub trait Seek {
            /// Seek to an offset, in bytes, in a stream.
            fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

            /// Returns the current seek position from the start of the stream.
            fn stream_position(&mut self) -> Result<u64> {
                self.seek(SeekFrom::Current(0))
            }

            /// Seek relative to the current position.
            fn seek_relative(&mut self, offset: i64) -> Result<()> {
                self.seek(SeekFrom::Current(offset))?;
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Extension traits
// ---------------------------------------------------------------------------

/// Read extension: structured reads and parsing.
///
/// Provides [`read_struct`](ReadExt::read_struct) for zero-copy deserialization
/// via [`bytemuck`], and [`parse`](ReadExt::parse) for types implementing
/// [`Parsable`].
///
/// This trait is automatically implemented for all types implementing [`Read`].
///
/// # Example
///
/// ```rust
/// use hadris_io::{Cursor, ReadExt};
///
/// let bytes = 42u32.to_ne_bytes();
/// let mut cursor = Cursor::new(&bytes);
/// let value: u32 = cursor.read_struct().unwrap();
/// assert_eq!(value, 42);
/// ```
pub trait ReadExt {
    /// Reads a `T` from the stream using zero-copy deserialization.
    ///
    /// The type `T` must implement [`bytemuck::AnyBitPattern`] so any
    /// byte pattern is a valid value.
    fn read_struct<T: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<T>;

    /// Parses a `T` from the stream using the [`Parsable`] trait.
    fn parse<T: Parsable>(&mut self) -> Result<T>;
}

impl<T: Read> ReadExt for T {
    fn read_struct<S: bytemuck::Zeroable + bytemuck::NoUninit + bytemuck::AnyBitPattern>(
        &mut self,
    ) -> Result<S> {
        let mut temp = S::zeroed();
        self.read_exact(bytemuck::bytes_of_mut(&mut temp))?;
        Ok(temp)
    }

    fn parse<S: Parsable>(&mut self) -> Result<S> {
        S::parse(self)
    }
}

/// Parse a type from a reader.
///
/// Implement this trait for types that have a custom binary format
/// that cannot be expressed as a simple `bytemuck` cast.
///
/// # Example
///
/// ```rust
/// use hadris_io::{Cursor, Result, sync::{Read, Parsable, ReadExt}};
///
/// struct Magic(u16);
///
/// impl Parsable for Magic {
///     fn parse<R: Read>(reader: &mut R) -> Result<Self> {
///         let mut buf = [0u8; 2];
///         reader.read_exact(&mut buf)?;
///         Ok(Magic(u16::from_le_bytes(buf)))
///     }
/// }
///
/// let data = [0x34, 0x12];
/// let mut cursor = Cursor::new(&data);
/// let magic: Magic = cursor.parse().unwrap();
/// assert_eq!(magic.0, 0x1234);
/// ```
pub trait Parsable: Sized {
    /// Parse this type from the given reader.
    fn parse<R: Read>(reader: &mut R) -> Result<Self>;
}

/// Write a type to a writer.
///
/// Implement this trait for types that need custom binary serialization.
pub trait Writable: Sized {
    /// Write this type to the given writer.
    fn write<W: Write>(&self, writer: &mut W) -> Result<()>;
}
