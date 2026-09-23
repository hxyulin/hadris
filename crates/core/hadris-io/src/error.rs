//! Error contract shared by every stream and device.

use core::convert::Infallible;
use core::fmt;

/// The error type of a stream or device.
///
/// Any `core::error::Error` that is `Send + Sync` works: a kernel uses its own
/// enum, std types use `std::io::Error`, and `embedded-io` errors pass through
/// unchanged. `Send + Sync` lets callers erase any device error into
/// `std::io::Error` or a boxed error without further bounds.
pub trait ErrorType {
    /// The error returned by every method.
    type Error: core::error::Error + Send + Sync + 'static;
}

impl<T: ErrorType + ?Sized> ErrorType for &mut T {
    type Error = T::Error;
}

#[cfg(feature = "alloc")]
impl<T: ErrorType + ?Sized> ErrorType for alloc::boxed::Box<T> {
    type Error = T::Error;
}

impl ErrorType for &[u8] {
    type Error = Infallible;
}

#[cfg(feature = "alloc")]
impl ErrorType for alloc::vec::Vec<u8> {
    type Error = Infallible;
}

#[cfg(feature = "std")]
impl ErrorType for std::fs::File {
    type Error = std::io::Error;
}

/// Failure of `read_exact`, `write_all` or `read_exact_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExactError<E> {
    /// The input ended before the buffer was filled.
    UnexpectedEof,
    /// The output accepted no bytes.
    WriteZero,
    /// The stream failed.
    Io(E),
}

impl<E: fmt::Display> fmt::Display for ExactError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => f.write_str("unexpected end of input"),
            Self::WriteZero => f.write_str("output accepted no bytes"),
            Self::Io(err) => err.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for ExactError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

/// A seek to a negative position, or one that does not fit in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSeek;

impl fmt::Display for InvalidSeek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("seek to a negative or out-of-range position")
    }
}

impl core::error::Error for InvalidSeek {}

/// Converts any device error to `std::io::Error`.
///
/// An `std::io::Error` is returned as itself, found by a downcast that does
/// not allocate, so `raw_os_error()` survives. Any other error becomes the
/// source of an error with kind `Other`.
///
/// ```rust
/// let os = std::io::Error::from_raw_os_error(5);
/// assert_eq!(hadris_io::into_std_error(os).raw_os_error(), Some(5));
/// ```
#[cfg(feature = "std")]
pub fn into_std_error<E: core::error::Error + Send + Sync + 'static>(err: E) -> std::io::Error {
    let mut slot = Some(err);
    let any: &mut dyn core::any::Any = &mut slot;
    if let Some(io) = any
        .downcast_mut::<Option<std::io::Error>>()
        .and_then(Option::take)
    {
        return io;
    }
    slot.map_or_else(|| std::io::ErrorKind::Other.into(), std::io::Error::other)
}

#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<ExactError<E>> for std::io::Error {
    fn from(err: ExactError<E>) -> Self {
        match err {
            ExactError::UnexpectedEof => std::io::ErrorKind::UnexpectedEof.into(),
            ExactError::WriteZero => std::io::ErrorKind::WriteZero.into(),
            ExactError::Io(err) => into_std_error(err),
        }
    }
}

#[cfg(feature = "std")]
impl From<InvalidSeek> for std::io::Error {
    fn from(err: InvalidSeek) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, err)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Fault;

    impl fmt::Display for Fault {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("fault")
        }
    }

    impl core::error::Error for Fault {}

    #[test]
    fn std_errors_come_back_unchanged() {
        let err: std::io::Error = ExactError::Io(std::io::Error::from_raw_os_error(13)).into();
        assert_eq!(err.raw_os_error(), Some(13));
    }

    #[test]
    fn other_errors_become_the_source() {
        let err: std::io::Error = ExactError::Io(Fault).into();
        assert_eq!(err.kind(), std::io::ErrorKind::Other);
        assert!(err.into_inner().unwrap().downcast::<Fault>().is_ok());
        let eof: std::io::Error = ExactError::<Fault>::UnexpectedEof.into();
        assert_eq!(eof.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
