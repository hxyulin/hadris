//! # Hadris IO
//!
//! Portable I/O trait abstractions for the Hadris filesystem crates.
//!
//! This crate provides [`Read`], [`Write`], and [`Seek`] traits that work in
//! both `std` and `no_std` environments. When the `std` feature is enabled,
//! the traits re-export directly from `std::io`. In `no_std` mode, minimal
//! custom trait definitions are provided with the same API surface.
//!
//! ## Feature Flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | Standard library support (implies `sync`) |
//! | `sync`  | yes     | Synchronous I/O traits |
//! | `async` | no      | Asynchronous I/O traits (uses async fn in trait) |
//!
//! ## Quick Start
//!
//! ```rust
//! use hadris_io::{Cursor, SeekFrom, Read, Seek};
//!
//! let data = [0x48, 0x44, 0x52, 0x53]; // "HDRS"
//! let mut cursor = Cursor::new(&data);
//!
//! let mut buf = [0u8; 2];
//! cursor.read_exact(&mut buf).unwrap();
//! assert_eq!(&buf, b"HD");
//!
//! cursor.seek(SeekFrom::Start(0)).unwrap();
//! cursor.read_exact(&mut buf).unwrap();
//! assert_eq!(&buf, b"HD");
//! ```
//!
//! ## Cursor
//!
//! The [`Cursor`] type wraps a byte slice and provides both [`Read`] and
//! [`Seek`] implementations, useful for in-memory parsing:
//!
//! ```rust
//! use hadris_io::Cursor;
//!
//! let data = b"Hello, Hadris!";
//! let mut cursor = Cursor::new(data);
//! assert_eq!(cursor.position(), 0);
//! cursor.set_position(7);
//! assert_eq!(cursor.position(), 7);
//! ```
//!
//! ## Extension Traits
//!
//! The [`ReadExt`] trait adds structured reading via [`bytemuck`]:
//!
//! ```rust
//! use hadris_io::{Cursor, ReadExt};
//!
//! let bytes = 0x1234u16.to_ne_bytes();
//! let mut cursor = Cursor::new(&bytes);
//! let value: u16 = cursor.read_struct().unwrap();
//! assert_eq!(value, 0x1234);
//! ```

#![no_std]
#![allow(async_fn_in_trait)]

#[cfg(feature = "std")]
extern crate std;

// ---------------------------------------------------------------------------
// Shared types (always available)
// ---------------------------------------------------------------------------

/// No-std compatible I/O error types.
#[cfg(not(feature = "std"))]
mod error;
#[cfg(not(feature = "std"))]
pub use error::{Error, ErrorKind, Result};

/// Re-export std error types when std is available.
#[cfg(feature = "std")]
pub use std::io::{Error, ErrorKind, Result};

/// Re-export std path types when std is available.
#[cfg(feature = "std")]
pub use std::path::{Path, PathBuf};

/// `SeekFrom` — re-exported from std or defined for no-std.
#[cfg(feature = "std")]
pub use std::io::SeekFrom;

#[cfg(not(feature = "std"))]
mod traits;
#[cfg(not(feature = "std"))]
pub use traits::SeekFrom;

/// Helper macro: short-circuit an `Err` by returning `Some(Err(..))`.
///
/// Useful in iterator implementations where the return type is
/// `Option<Result<T>>`. Extracts the `Ok` value, or returns
/// `Some(Err(..))` immediately on error.
///
/// # Example
///
/// ```rust
/// use hadris_io::{try_io_result_option, Result, Error, ErrorKind};
///
/// fn next_item(ok: bool) -> Option<Result<u32>> {
///     let result: Result<u32> = if ok {
///         Ok(42)
///     } else {
///         Err(Error::new(ErrorKind::NotFound, "missing"))
///     };
///     let value = try_io_result_option!(result);
///     Some(Ok(value * 2))
/// }
///
/// assert!(matches!(next_item(true), Some(Ok(84))));
/// assert!(matches!(next_item(false), Some(Err(_))));
/// ```
#[macro_export]
macro_rules! try_io_result_option {
    ($expr:expr) => {
        match $expr {
            Ok(val) => val,
            Err(err) => return Some(Err(err)),
        }
    };
}

// ---------------------------------------------------------------------------
// Cursor (shared, works with both sync and async)
// ---------------------------------------------------------------------------

/// A no-std compatible Cursor for reading from byte slices.
///
/// Wraps a `&[u8]` and tracks a read position, implementing both
/// [`sync::Read`] and [`sync::Seek`] (when the `sync` feature is enabled).
///
/// # Example
///
/// ```rust
/// use hadris_io::{Cursor, Read, Seek, SeekFrom};
///
/// let data = [1u8, 2, 3, 4, 5];
/// let mut cursor = Cursor::new(&data);
///
/// let mut buf = [0u8; 2];
/// cursor.read_exact(&mut buf).unwrap();
/// assert_eq!(buf, [1, 2]);
///
/// cursor.seek(SeekFrom::Start(0)).unwrap();
/// cursor.read_exact(&mut buf).unwrap();
/// assert_eq!(buf, [1, 2]);
/// ```
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> Cursor<'a> {
    /// Creates a new cursor wrapping the given byte slice, starting at position 0.
    ///
    /// ```rust
    /// use hadris_io::Cursor;
    ///
    /// let data = [1, 2, 3];
    /// let cursor = Cursor::new(&data);
    /// assert_eq!(cursor.position(), 0);
    /// assert_eq!(cursor.get_ref().len(), 3);
    /// ```
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }

    /// Returns the current byte offset within the underlying data.
    ///
    /// ```rust
    /// use hadris_io::Cursor;
    ///
    /// let mut cursor = Cursor::new(&[0u8; 10]);
    /// assert_eq!(cursor.position(), 0);
    /// cursor.set_position(5);
    /// assert_eq!(cursor.position(), 5);
    /// ```
    pub fn position(&self) -> usize {
        self.cursor
    }

    /// Sets the cursor position to the given byte offset.
    ///
    /// ```rust
    /// use hadris_io::Cursor;
    ///
    /// let mut cursor = Cursor::new(&[0u8; 10]);
    /// cursor.set_position(7);
    /// assert_eq!(cursor.position(), 7);
    /// ```
    pub fn set_position(&mut self, pos: usize) {
        self.cursor = pos;
    }

    /// Returns a reference to the underlying byte slice.
    ///
    /// ```rust
    /// use hadris_io::Cursor;
    ///
    /// let data = [1, 2, 3];
    /// let cursor = Cursor::new(&data);
    /// assert_eq!(cursor.get_ref(), &[1, 2, 3]);
    /// ```
    pub fn get_ref(&self) -> &'a [u8] {
        self.data
    }

    #[allow(dead_code)]
    fn read_impl(&mut self, buf: &mut [u8]) -> Result<usize> {
        let remaining = self.data.len().saturating_sub(self.cursor);
        let to_read = buf.len().min(remaining);
        if to_read > 0 {
            buf[..to_read].copy_from_slice(&self.data[self.cursor..self.cursor + to_read]);
            self.cursor += to_read;
        }
        Ok(to_read)
    }

    #[allow(dead_code)]
    fn seek_impl(&mut self, pos: SeekFrom) -> Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.data.len() as i64 + offset,
            SeekFrom::Current(offset) => self.cursor as i64 + offset,
        };

        if new_pos < 0 {
            #[cfg(feature = "std")]
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid seek to negative position",
            ));
            #[cfg(not(feature = "std"))]
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "invalid seek to negative position",
            ));
        }

        self.cursor = new_pos as usize;
        Ok(self.cursor as u64)
    }
}

// ---------------------------------------------------------------------------
// Sync module
// ---------------------------------------------------------------------------

#[cfg(feature = "sync")]
mod sync_api;

/// Synchronous I/O traits.
///
/// Contains [`Read`], [`Write`], [`Seek`],
/// plus extension traits [`ReadExt`], [`Parsable`],
/// [`Writable`].
#[cfg(feature = "sync")]
pub mod sync {
    pub use super::sync_api::*;
}

// Cursor: sync trait impls
#[cfg(feature = "sync")]
impl sync::Read for Cursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.read_impl(buf)
    }
}

#[cfg(all(feature = "sync", not(feature = "std")))]
impl sync::Seek for Cursor<'_> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.seek_impl(pos)
    }
}

#[cfg(all(feature = "sync", feature = "std"))]
impl std::io::Seek for Cursor<'_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        // Convert std SeekFrom to our result
        let our_pos = match pos {
            std::io::SeekFrom::Start(n) => SeekFrom::Start(n),
            std::io::SeekFrom::End(n) => SeekFrom::End(n),
            std::io::SeekFrom::Current(n) => SeekFrom::Current(n),
        };
        self.seek_impl(our_pos)
    }
}

// Default re-export for backwards compatibility
#[cfg(feature = "sync")]
pub use sync::*;

// ---------------------------------------------------------------------------
// Async module
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
mod async_api;

/// Asynchronous I/O traits (using async fn in trait).
///
/// Contains async versions of [`Read`](r#async::Read),
/// [`Write`](r#async::Write), [`Seek`](r#async::Seek),
/// plus async extension traits.
#[cfg(feature = "async")]
pub mod r#async {
    pub use super::async_api::*;
}

// Cursor: async trait impls
#[cfg(feature = "async")]
impl r#async::Read for Cursor<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.read_impl(buf)
    }
}

#[cfg(feature = "async")]
impl r#async::Seek for Cursor<'_> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.seek_impl(pos)
    }
}

#[cfg(all(test, feature = "sync"))]
mod tests {
    extern crate std;
    use super::*;
    use std::format;

    // -----------------------------------------------------------------------
    // Cursor tests
    // -----------------------------------------------------------------------

    #[test]
    fn cursor_new_starts_at_zero() {
        let data = [1, 2, 3, 4, 5];
        let cursor = Cursor::new(&data);
        assert_eq!(cursor.position(), 0);
        assert_eq!(cursor.get_ref(), &data);
    }

    #[test]
    fn cursor_set_position() {
        let data = [0u8; 10];
        let mut cursor = Cursor::new(&data);
        cursor.set_position(5);
        assert_eq!(cursor.position(), 5);
        cursor.set_position(0);
        assert_eq!(cursor.position(), 0);
    }

    #[test]
    fn cursor_read_basic() {
        let data = [10, 20, 30, 40, 50];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 3];
        let n = cursor.read_impl(&mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(buf, [10, 20, 30]);
        assert_eq!(cursor.position(), 3);
    }

    #[test]
    fn cursor_read_past_end() {
        let data = [1, 2];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 5];
        let n = cursor.read_impl(&mut buf).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&buf[..2], &[1, 2]);
        assert_eq!(cursor.position(), 2);

        // Reading again at end returns 0
        let n = cursor.read_impl(&mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn cursor_read_empty_buffer() {
        let data = [1, 2, 3];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 0];
        let n = cursor.read_impl(&mut buf).unwrap();
        assert_eq!(n, 0);
        assert_eq!(cursor.position(), 0);
    }

    #[test]
    fn cursor_seek_start() {
        let data = [0u8; 20];
        let mut cursor = Cursor::new(&data);
        let pos = cursor.seek_impl(SeekFrom::Start(10)).unwrap();
        assert_eq!(pos, 10);
        assert_eq!(cursor.position(), 10);
    }

    #[test]
    fn cursor_seek_end() {
        let data = [0u8; 20];
        let mut cursor = Cursor::new(&data);
        let pos = cursor.seek_impl(SeekFrom::End(-5)).unwrap();
        assert_eq!(pos, 15);
        assert_eq!(cursor.position(), 15);
    }

    #[test]
    fn cursor_seek_current() {
        let data = [0u8; 20];
        let mut cursor = Cursor::new(&data);
        cursor.set_position(10);
        let pos = cursor.seek_impl(SeekFrom::Current(3)).unwrap();
        assert_eq!(pos, 13);
        let pos = cursor.seek_impl(SeekFrom::Current(-5)).unwrap();
        assert_eq!(pos, 8);
    }

    #[test]
    fn cursor_seek_negative_position_errors() {
        let data = [0u8; 10];
        let mut cursor = Cursor::new(&data);
        let result = cursor.seek_impl(SeekFrom::End(-20));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn cursor_seek_to_start_of_stream() {
        let data = [0u8; 10];
        let mut cursor = Cursor::new(&data);
        cursor.set_position(5);
        let pos = cursor.seek_impl(SeekFrom::Start(0)).unwrap();
        assert_eq!(pos, 0);
    }

    #[test]
    fn cursor_clone() {
        let data = [1, 2, 3, 4, 5];
        let mut cursor = Cursor::new(&data);
        cursor.set_position(3);
        let clone = cursor.clone();
        assert_eq!(clone.position(), 3);
        assert_eq!(clone.get_ref(), cursor.get_ref());
    }

    #[test]
    fn cursor_debug_format() {
        let data = [1, 2, 3];
        let cursor = Cursor::new(&data);
        let debug = format!("{:?}", cursor);
        assert!(debug.contains("Cursor"));
    }

    // -----------------------------------------------------------------------
    // Sync Read/Seek trait tests via Cursor
    // -----------------------------------------------------------------------

    #[test]
    fn sync_read_trait() {
        use sync::Read;
        let data = [10, 20, 30, 40, 50];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 3];
        let n = cursor.read(&mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(buf, [10, 20, 30]);
    }

    #[test]
    fn sync_read_exact_success() {
        use sync::Read;
        let data = [1, 2, 3, 4, 5];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 5];
        cursor.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn sync_read_exact_eof() {
        use sync::Read;
        let data = [1, 2];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 5];
        let result = cursor.read_exact(&mut buf);
        assert!(result.is_err());
    }

    #[test]
    fn sync_seek_trait() {
        use sync::Seek;
        let data = [0u8; 20];
        let mut cursor = Cursor::new(&data);
        let pos = cursor.seek(SeekFrom::Start(10)).unwrap();
        assert_eq!(pos, 10);
        let pos = cursor.stream_position().unwrap();
        assert_eq!(pos, 10);
    }

    #[test]
    fn sync_seek_relative() {
        use sync::Seek;
        let data = [0u8; 20];
        let mut cursor = Cursor::new(&data);
        cursor.seek(SeekFrom::Start(5)).unwrap();
        cursor.seek_relative(3).unwrap();
        assert_eq!(cursor.stream_position().unwrap(), 8);
        cursor.seek_relative(-2).unwrap();
        assert_eq!(cursor.stream_position().unwrap(), 6);
    }

    // -----------------------------------------------------------------------
    // ReadExt tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_ext_read_struct() {
        use sync::ReadExt;
        let data = [0x78, 0x56, 0x34, 0x12]; // LE u32 = 0x12345678
        let mut cursor = Cursor::new(&data);
        let val: u32 = cursor.read_struct().unwrap();
        assert_eq!(val, u32::from_ne_bytes([0x78, 0x56, 0x34, 0x12]));
    }

    #[test]
    fn read_ext_read_struct_eof() {
        use sync::ReadExt;
        let data = [0x78, 0x56]; // Only 2 bytes, not enough for u32
        let mut cursor = Cursor::new(&data);
        let result: Result<u32> = cursor.read_struct();
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // try_io_result_option! macro tests
    // -----------------------------------------------------------------------

    #[test]
    fn try_io_result_option_ok() {
        fn test_fn() -> Option<Result<u32>> {
            let val: Result<u32> = Ok(42);
            let v = try_io_result_option!(val);
            Some(Ok(v))
        }
        let result = test_fn();
        assert!(matches!(result, Some(Ok(42))));
    }

    #[test]
    fn try_io_result_option_err() {
        fn test_fn() -> Option<Result<u32>> {
            let val: Result<u32> = Err(Error::new(ErrorKind::NotFound, "not found"));
            let _v = try_io_result_option!(val);
            Some(Ok(0)) // Should not reach here
        }
        let result = test_fn();
        assert!(matches!(result, Some(Err(_))));
    }
}
