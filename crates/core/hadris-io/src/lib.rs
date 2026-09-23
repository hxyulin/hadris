//! # Hadris IO
//!
//! Portable I/O traits for the Hadris filesystem crates, built on
//! [`embedded-io`](embedded_io).
//!
//! The [`Read`], [`Write`] and [`Seek`] traits return the portable [`Error`]
//! and are implemented for `&mut T`. Wrap an `embedded-io` device in
//! [`FromEmbedded`] to use it with Hadris. With `std`, wrap a `std::io` type
//! in [`StdIo`], and wrap a Hadris reader in [`ToStd`] to use it with
//! `std::io`. Enabling features only adds items; no trait or type changes
//! shape.
//!
//! ## Feature Flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | [`StdIo`], [`ToStd`] and `std::io::Error` conversions (implies `alloc`) |
//! | `sync`  | yes     | Synchronous traits in [`sync`] |
//! | `async` | no      | Asynchronous traits in `r#async` |
//! | `alloc` | via `std` | Keep the device error as [`Error`]'s source |
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
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod error;
pub use error::{Error, ErrorKind, Result};

#[cfg(feature = "std")]
mod std_adapters;
#[cfg(feature = "std")]
pub use std_adapters::StdIo;
#[cfg(all(feature = "std", feature = "sync"))]
pub use std_adapters::ToStd;

/// Portable seek position, convertible to and from `std::io::SeekFrom`.
pub use embedded_io::SeekFrom;

/// Use an `embedded-io` (or, in async mode, `embedded-io-async`) device with
/// the Hadris traits.
///
/// The device error is kept as the [`Error`]'s source when `alloc` is enabled.
#[derive(Debug, Clone, Copy, Default)]
pub struct FromEmbedded<T>(T);

impl<T> FromEmbedded<T> {
    /// Wrap an `embedded-io` device.
    pub const fn new(inner: T) -> Self {
        Self(inner)
    }

    /// Recover the device.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// Borrow the device.
    pub const fn get_ref(&self) -> &T {
        &self.0
    }

    /// Mutably borrow the device.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Short-circuit an `Err` by returning `Some(Err(..))`.
///
/// Useful in iterator implementations where the return type is
/// `Option<Result<T>>`.
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
            Err(err) => return Some(Err(err.into())),
        }
    };
}

#[cfg(any(feature = "sync", feature = "async"))]
fn copy_from_slice_at(data: &[u8], offset: u64, buf: &mut [u8]) -> usize {
    let Ok(start) = usize::try_from(offset) else {
        return 0;
    };
    let Some(available) = data.get(start..) else {
        return 0;
    };
    let count = available.len().min(buf.len());
    buf[..count].copy_from_slice(&available[..count]);
    count
}

/// A no-std cursor for reading from a byte slice.
///
/// Implements [`Read`] and [`Seek`] in both modes.
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
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, cursor: 0 }
    }

    /// Returns the current byte offset within the underlying data.
    pub fn position(&self) -> usize {
        self.cursor
    }

    /// Sets the cursor position to the given byte offset.
    pub fn set_position(&mut self, pos: usize) {
        self.cursor = pos;
    }

    /// Returns a reference to the underlying byte slice.
    pub fn get_ref(&self) -> &'a [u8] {
        self.data
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    fn read_impl(&mut self, buf: &mut [u8]) -> core::result::Result<usize, ErrorKind> {
        let count = copy_slice(self.remaining_slice(), buf);
        self.cursor += count;
        Ok(count)
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    fn remaining_slice(&self) -> &'a [u8] {
        self.data.get(self.cursor..).unwrap_or(&[])
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    fn seek_impl(&mut self, pos: SeekFrom) -> core::result::Result<u64, ErrorKind> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(offset) => (self.data.len() as u64).checked_add_signed(offset),
            SeekFrom::Current(offset) => (self.cursor as u64).checked_add_signed(offset),
        }
        .ok_or(ErrorKind::InvalidInput)?;

        self.cursor = usize::try_from(new_pos).map_err(|_| ErrorKind::InvalidInput)?;
        Ok(self.cursor as u64)
    }
}

#[cfg(any(feature = "sync", feature = "async"))]
fn copy_slice(from: &[u8], to: &mut [u8]) -> usize {
    let count = from.len().min(to.len());
    to[..count].copy_from_slice(&from[..count]);
    count
}

#[cfg(feature = "sync")]
impl sync::Read for Cursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.read_impl(buf)?)
    }
}

#[cfg(feature = "sync")]
impl sync::Seek for Cursor<'_> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        Ok(self.seek_impl(pos)?)
    }
}

#[cfg(feature = "async")]
impl r#async::Read for Cursor<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        Ok(self.read_impl(buf)?)
    }
}

#[cfg(feature = "async")]
impl r#async::Seek for Cursor<'_> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        Ok(self.seek_impl(pos)?)
    }
}

/// Synchronous I/O traits.
#[cfg(feature = "sync")]
pub mod sync;

#[cfg(feature = "sync")]
pub use sync::*;

/// Asynchronous I/O traits.
#[cfg(feature = "async")]
pub mod r#async;

#[cfg(all(test, feature = "sync"))]
mod tests {
    extern crate std;
    use super::*;
    use std::format;
    #[cfg(feature = "alloc")]
    use std::vec::Vec;

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
    fn cursor_seek_end_large_offset_does_not_panic() {
        let data = [0u8; 5];
        let mut cursor = Cursor::new(&data);
        let result = cursor.seek_impl(SeekFrom::End(i64::MAX));
        match result {
            Ok(pos) => assert_eq!(pos, 5 + i64::MAX as u64),
            Err(err) => assert_eq!(err.kind(), ErrorKind::InvalidInput),
        }
    }

    #[test]
    fn cursor_seek_current_overflow_errors() {
        let data = [0u8; 5];
        let mut cursor = Cursor::new(&data);
        cursor.set_position(usize::MAX);
        let result = cursor.seek_impl(SeekFrom::Current(i64::MAX));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn cursor_seek_start_u64_max_accepted() {
        let data = [1u8, 2, 3];
        let mut cursor = Cursor::new(&data);
        match cursor.seek_impl(SeekFrom::Start(u64::MAX)) {
            Ok(pos) => {
                assert!(usize::try_from(u64::MAX).is_ok());
                assert_eq!(pos, u64::MAX);
                let mut buf = [0u8; 4];
                let n = cursor.read_impl(&mut buf).unwrap();
                assert_eq!(n, 0);
            }
            Err(err) => {
                assert!(usize::try_from(u64::MAX).is_err());
                assert_eq!(err.kind(), ErrorKind::InvalidInput);
            }
        }
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
        let debug = format!("{cursor:?}");
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

    // -----------------------------------------------------------------------
    // Blanket impls, &mut and ByteSource
    // -----------------------------------------------------------------------

    #[test]
    fn mutable_reference_is_a_reader() {
        fn read_two<R: Read>(mut reader: R) -> [u8; 2] {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).unwrap();
            buf
        }
        let data = [7, 8, 9, 10];
        let mut cursor = Cursor::new(&data);
        assert_eq!(read_two(&mut cursor), [7, 8]);
        assert_eq!(read_two(&mut cursor), [9, 10]);
    }

    #[test]
    fn byte_source_over_slice() {
        let mut slice: &[u8] = &[1, 2, 3, 4];
        let mut buf = [0u8; 3];
        assert_eq!(slice.read_at(2, &mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[3, 4]);
        assert_eq!(slice.read_at(9, &mut buf).unwrap(), 0);
        assert_eq!(slice.read_at(u64::MAX, &mut buf).unwrap(), 0);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn byte_source_over_vec() {
        let mut buf = [0u8; 3];
        let mut vec: Vec<u8> = (0..10).collect();
        vec.read_exact_at(5, &mut buf).unwrap();
        assert_eq!(buf, [5, 6, 7]);
        assert_eq!(
            vec.read_exact_at(8, &mut buf).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn seek_source_limits_reads_to_len() {
        let data = [0u8, 1, 2, 3, 4, 5];
        let mut source = SeekSource::new(Cursor::new(&data)).unwrap();
        assert_eq!(source.len(), 6);
        let mut buf = [0u8; 4];
        assert_eq!(source.read_at(4, &mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[4, 5]);

        let mut short = SeekSource::with_len(Cursor::new(&data), 3);
        assert_eq!(short.read_at(1, &mut buf).unwrap(), 2);
    }
}
