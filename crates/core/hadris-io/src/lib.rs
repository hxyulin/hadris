//! # Hadris IO
//!
//! Portable I/O traits for the Hadris filesystem crates.
//!
//! [`Read`](sync::Read), [`Write`](sync::Write) and [`Seek`](sync::Seek)
//! report the implementor's own error through the [`ErrorType`]
//! supertrait, as `embedded-io` does. The error can
//! be any `core::error::Error + Send + Sync`: a kernel uses its own enum,
//! [`StdIo`] reports `std::io::Error`, and `FromEmbedded` (with the
//! `embedded-io` feature) passes an `embedded-io` error through unchanged. `&mut T` implements each trait
//! when `T` does. Enabling features only adds items; no trait or type changes
//! shape.
//!
//! [`Error<E>`](Error) is the error of every block device and filesystem
//! operation in Hadris: an [`ErrorKind`], a static message, an optional
//! [`Location`] and [`DetailCode`], and the device's own error `E` when the
//! device failed. It lives here, in the lowest crate, so block devices can
//! return it; `hadris-fs` and `hadris` re-export it.
//!
//! The traits live in one module per mode: [`sync`], `r#async`,
//! `async_send` and `local` (futures that need not be `Send`). The crate
//! root holds only the mode-independent items, so
//! `hadris_io::sync::Read` and `hadris_io::r#async::Read` are always named
//! explicitly.
//!
//! ## Feature Flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | [`StdIo`], [`ToStd`] and conversions to `std::io::Error` (implies `alloc`) |
//! | `sync`  | yes     | Synchronous traits in [`sync`] |
//! | `async` | no      | Asynchronous traits in `r#async` and `local` |
//! | `async-send` | no | Asynchronous traits with `Send` futures in `async_send` (implies `async`) |
//! | `alloc` | via `std` | `Box<T>` and `Vec<u8>` implement the traits |
//! | `embedded-io` | no | `FromEmbedded`, the `embedded-io` traits on [`StdIo`] and [`SeekFrom`] conversions |
//!
//! ## Quick Start
//!
//! ```rust
//! use hadris_io::sync::{Read, Seek};
//! use hadris_io::{Cursor, SeekFrom};
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
//! ## Implementing a device
//!
//! ```rust
//! use hadris_io::ErrorType;
//! use hadris_io::sync::Read;
//!
//! #[derive(Debug)]
//! enum UartError { Framing }
//!
//! impl core::fmt::Display for UartError {
//!     fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
//!         f.write_str("framing error")
//!     }
//! }
//!
//! impl core::error::Error for UartError {}
//!
//! struct Uart;
//!
//! impl ErrorType for Uart {
//!     type Error = UartError;
//! }
//!
//! impl Read for Uart {
//!     fn read(&mut self, _buf: &mut [u8]) -> Result<usize, UartError> {
//!         Err(UartError::Framing)
//!     }
//! }
//!
//! let err = Uart.read_exact(&mut [0; 4]).unwrap_err();
//! assert!(matches!(err, hadris_io::ExactError::Io(UartError::Framing)));
//! ```

#![no_std]
#![deny(missing_docs)]
#![allow(async_fn_in_trait)]

#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod error;
mod fs_error;
mod kind;
#[cfg(feature = "std")]
pub use error::into_std_error;
pub use error::{ErrorType, ExactError, InvalidSeek};
pub use fs_error::{DetailCode, Error, FsResult, Location};
pub use kind::{Errno, ErrorKind};

#[cfg(feature = "std")]
mod std_adapters;
#[cfg(feature = "std")]
pub use std_adapters::StdIo;
#[cfg(all(feature = "std", feature = "sync"))]
pub use std_adapters::ToStd;

/// A seek position, as in `std::io::SeekFrom`.
///
/// Converts to and from `std::io::SeekFrom` with `std`, and
/// `embedded_io::SeekFrom` with the `embedded-io` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SeekFrom {
    /// This many bytes from the start.
    Start(u64),
    /// This many bytes from the end, usually negative.
    End(i64),
    /// This many bytes from the current position.
    Current(i64),
}

impl SeekFrom {
    /// The position this names in a stream at `current` of `len` bytes, or
    /// `None` when it is negative or overflows.
    ///
    /// ```rust
    /// use hadris_io::SeekFrom;
    ///
    /// assert_eq!(SeekFrom::End(-2).resolve(0, 10), Some(8));
    /// assert_eq!(SeekFrom::Current(-5).resolve(3, 10), None);
    /// ```
    pub const fn resolve(self, current: u64, len: u64) -> Option<u64> {
        match self {
            Self::Start(offset) => Some(offset),
            Self::End(delta) => len.checked_add_signed(delta),
            Self::Current(delta) => current.checked_add_signed(delta),
        }
    }
}

#[cfg(feature = "std")]
impl From<SeekFrom> for std::io::SeekFrom {
    fn from(pos: SeekFrom) -> Self {
        match pos {
            SeekFrom::Start(offset) => Self::Start(offset),
            SeekFrom::End(delta) => Self::End(delta),
            SeekFrom::Current(delta) => Self::Current(delta),
        }
    }
}

#[cfg(feature = "std")]
impl From<std::io::SeekFrom> for SeekFrom {
    fn from(pos: std::io::SeekFrom) -> Self {
        match pos {
            std::io::SeekFrom::Start(offset) => Self::Start(offset),
            std::io::SeekFrom::End(delta) => Self::End(delta),
            std::io::SeekFrom::Current(delta) => Self::Current(delta),
        }
    }
}

#[cfg(feature = "embedded-io")]
impl From<SeekFrom> for embedded_io::SeekFrom {
    fn from(pos: SeekFrom) -> Self {
        match pos {
            SeekFrom::Start(offset) => Self::Start(offset),
            SeekFrom::End(delta) => Self::End(delta),
            SeekFrom::Current(delta) => Self::Current(delta),
        }
    }
}

#[cfg(feature = "embedded-io")]
impl From<embedded_io::SeekFrom> for SeekFrom {
    fn from(pos: embedded_io::SeekFrom) -> Self {
        match pos {
            embedded_io::SeekFrom::Start(offset) => Self::Start(offset),
            embedded_io::SeekFrom::End(delta) => Self::End(delta),
            embedded_io::SeekFrom::Current(delta) => Self::Current(delta),
        }
    }
}

/// Use an `embedded-io` (or, in async mode, `embedded-io-async`) device with
/// the Hadris traits. The device's error is used unchanged.
#[cfg(feature = "embedded-io")]
#[derive(Debug, Clone, Copy, Default)]
pub struct FromEmbedded<T>(T);

#[cfg(feature = "embedded-io")]
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

#[cfg(feature = "embedded-io")]
impl<T: embedded_io::ErrorType> ErrorType for FromEmbedded<T>
where
    T::Error: Send + Sync + 'static,
{
    type Error = T::Error;
}

/// Short-circuit an `Err` by returning `Some(Err(..))`.
///
/// Useful in iterator implementations where the return type is
/// `Option<Result<T, E>>`.
///
/// ```rust
/// use hadris_io::try_io_result_option;
///
/// fn next_item(ok: bool) -> Option<Result<u32, &'static str>> {
///     let result: Result<u32, &'static str> = if ok { Ok(42) } else { Err("missing") };
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
/// Implements [`sync::Read`] and [`sync::Seek`] and their async counterparts.
/// Reads never fail; seeking to a negative position fails with [`InvalidSeek`].
///
/// ```rust
/// use hadris_io::sync::{Read, Seek};
/// use hadris_io::{Cursor, SeekFrom};
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
    fn read_slice(&mut self, buf: &mut [u8]) -> usize {
        let remaining = self.data.get(self.cursor..).unwrap_or(&[]);
        let count = remaining.len().min(buf.len());
        buf[..count].copy_from_slice(&remaining[..count]);
        self.cursor += count;
        count
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    fn seek_to(&mut self, pos: SeekFrom) -> Result<u64, InvalidSeek> {
        let new_pos = pos
            .resolve(self.cursor as u64, self.data.len() as u64)
            .ok_or(InvalidSeek)?;
        self.cursor = usize::try_from(new_pos).map_err(|_| InvalidSeek)?;
        Ok(new_pos)
    }
}

impl ErrorType for Cursor<'_> {
    type Error = InvalidSeek;
}

/// Synchronous I/O traits.
#[cfg(feature = "sync")]
pub mod sync;

/// Asynchronous I/O traits.
#[cfg(feature = "async")]
pub mod r#async;

/// Asynchronous I/O traits whose futures need not be `Send`, for
/// single-threaded executors such as embassy.
///
/// Generated from the same source as `r#async`, with the same items.
#[cfg(feature = "async")]
pub mod local;

/// Asynchronous I/O traits whose futures are `Send`, for generic code on
/// multi-threaded executors.
///
/// Generated from the same source as `r#async`. Every trait has `Send` as a
/// supertrait, so `R: Read` alone proves that `R`'s futures are `Send`.
/// Implementations are written with `async fn` exactly as in `r#async`.
/// `FromEmbedded` has no impls here: `embedded-io-async` futures are not
/// `Send`.
#[cfg(feature = "async-send")]
pub mod async_send;

#[cfg(all(test, feature = "sync"))]
mod tests {
    extern crate std;
    use super::sync::*;
    use super::*;
    use core::convert::Infallible;
    use std::format;
    #[cfg(feature = "alloc")]
    use std::vec::Vec;

    #[test]
    fn cursor_new_starts_at_zero() {
        let data = [1, 2, 3, 4, 5];
        let cursor = Cursor::new(&data);
        assert_eq!(cursor.position(), 0);
        assert_eq!(cursor.get_ref(), &data);
    }

    #[test]
    fn cursor_read_past_end() {
        let data = [1, 2];
        let mut cursor = Cursor::new(&data);
        let mut buf = [0u8; 5];
        assert_eq!(cursor.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[1, 2]);
        assert_eq!(cursor.read(&mut buf).unwrap(), 0);
        assert_eq!(
            cursor.read_exact(&mut buf).unwrap_err(),
            ExactError::UnexpectedEof
        );
    }

    #[test]
    fn cursor_seeks() {
        let data = [0u8; 20];
        let mut cursor = Cursor::new(&data);
        assert_eq!(cursor.seek(SeekFrom::Start(10)).unwrap(), 10);
        assert_eq!(cursor.seek(SeekFrom::End(-5)).unwrap(), 15);
        assert_eq!(cursor.seek(SeekFrom::Current(-3)).unwrap(), 12);
        assert_eq!(cursor.stream_position().unwrap(), 12);
        cursor.rewind().unwrap();
        assert_eq!(cursor.position(), 0);
    }

    #[test]
    fn seek_positions_resolve_and_convert() {
        assert_eq!(SeekFrom::Start(7).resolve(3, 10), Some(7));
        assert_eq!(SeekFrom::Current(2).resolve(3, 10), Some(5));
        assert_eq!(SeekFrom::End(-11).resolve(3, 10), None);
        assert_eq!(SeekFrom::Current(i64::MAX).resolve(u64::MAX, 0), None);
        #[cfg(feature = "std")]
        for pos in [SeekFrom::Start(1), SeekFrom::End(-2), SeekFrom::Current(3)] {
            assert_eq!(SeekFrom::from(std::io::SeekFrom::from(pos)), pos);
        }
        #[cfg(feature = "embedded-io")]
        for pos in [SeekFrom::Start(1), SeekFrom::End(-2), SeekFrom::Current(3)] {
            assert_eq!(SeekFrom::from(embedded_io::SeekFrom::from(pos)), pos);
        }
    }

    #[test]
    fn cursor_rejects_invalid_seeks() {
        let data = [0u8; 10];
        let mut cursor = Cursor::new(&data);
        assert_eq!(cursor.seek(SeekFrom::End(-20)), Err(InvalidSeek));
        cursor.set_position(usize::MAX);
        assert_eq!(cursor.seek(SeekFrom::Current(i64::MAX)), Err(InvalidSeek));
    }

    #[test]
    fn cursor_seek_past_end_reads_nothing() {
        let data = [1u8, 2, 3];
        let mut cursor = Cursor::new(&data);
        if cursor.seek(SeekFrom::Start(u64::MAX)).is_ok() {
            assert_eq!(cursor.read(&mut [0u8; 4]).unwrap(), 0);
        }
    }

    #[test]
    fn cursor_debug_format() {
        let data = [1, 2, 3];
        assert!(format!("{:?}", Cursor::new(&data)).contains("Cursor"));
    }

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

    struct Sink {
        accepted: usize,
    }

    impl ErrorType for Sink {
        type Error = Infallible;
    }

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> Result<usize, Infallible> {
            let n = buf.len().min(self.accepted);
            self.accepted -= n;
            Ok(n)
        }

        fn flush(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
    }

    #[test]
    fn write_all_reports_write_zero() {
        let mut sink = Sink { accepted: 3 };
        assert_eq!(sink.write_all(&[1, 2]), Ok(()));
        assert_eq!(sink.write_all(&[1, 2]), Err(ExactError::WriteZero));
    }

    #[test]
    fn try_io_result_option_err() {
        fn test_fn() -> Option<Result<u32, InvalidSeek>> {
            let val: Result<u32, InvalidSeek> = Err(InvalidSeek);
            let _v = try_io_result_option!(val);
            Some(Ok(0))
        }
        assert!(matches!(test_fn(), Some(Err(InvalidSeek))));
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
            vec.read_exact_at(8, &mut buf).unwrap_err(),
            ExactError::UnexpectedEof
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
