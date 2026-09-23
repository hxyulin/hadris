//! Portable I/O errors.

use core::fmt::{self, Display};

/// Portable error classification. This is a superset of `embedded_io::ErrorKind`
/// and retains the `std::io` conditions Hadris needs for helper operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// An entity was not found.
    NotFound,
    /// An operation lacked sufficient permissions.
    PermissionDenied,
    /// A connection attempt was refused.
    ConnectionRefused,
    /// A connection was reset by its peer.
    ConnectionReset,
    /// A connection was aborted locally.
    ConnectionAborted,
    /// The endpoint is not connected.
    NotConnected,
    /// The requested address is already in use.
    AddrInUse,
    /// The requested address is unavailable.
    AddrNotAvailable,
    /// A write targeted a closed pipe or connection.
    BrokenPipe,
    /// An entity already exists.
    AlreadyExists,
    /// The operation would block.
    WouldBlock,
    /// An input parameter was invalid.
    InvalidInput,
    /// Input data was malformed.
    InvalidData,
    /// The operation timed out.
    TimedOut,
    /// A write produced no progress.
    WriteZero,
    /// The operation was interrupted and may be retried.
    Interrupted,
    /// Input ended before the requested data was read.
    UnexpectedEof,
    /// The requested operation is unsupported.
    Unsupported,
    /// The operation could not allocate required memory.
    OutOfMemory,
    /// An error without a more specific portable classification.
    Other,
}

impl ErrorKind {
    /// Return this already-normalized error kind.
    pub const fn kind(&self) -> Self {
        *self
    }
}

impl From<embedded_io::ErrorKind> for ErrorKind {
    fn from(kind: embedded_io::ErrorKind) -> Self {
        use embedded_io::ErrorKind as E;
        match kind {
            E::NotFound => Self::NotFound,
            E::PermissionDenied => Self::PermissionDenied,
            E::ConnectionRefused => Self::ConnectionRefused,
            E::ConnectionReset => Self::ConnectionReset,
            E::ConnectionAborted => Self::ConnectionAborted,
            E::NotConnected => Self::NotConnected,
            E::AddrInUse => Self::AddrInUse,
            E::AddrNotAvailable => Self::AddrNotAvailable,
            E::BrokenPipe => Self::BrokenPipe,
            E::AlreadyExists => Self::AlreadyExists,
            E::InvalidInput => Self::InvalidInput,
            E::InvalidData => Self::InvalidData,
            E::TimedOut => Self::TimedOut,
            E::WriteZero => Self::WriteZero,
            E::Interrupted => Self::Interrupted,
            E::Unsupported => Self::Unsupported,
            E::OutOfMemory => Self::OutOfMemory,
            _ => Self::Other,
        }
    }
}

impl From<ErrorKind> for embedded_io::ErrorKind {
    fn from(kind: ErrorKind) -> Self {
        use embedded_io::ErrorKind as E;
        match kind {
            ErrorKind::NotFound => E::NotFound,
            ErrorKind::PermissionDenied => E::PermissionDenied,
            ErrorKind::ConnectionRefused => E::ConnectionRefused,
            ErrorKind::ConnectionReset => E::ConnectionReset,
            ErrorKind::ConnectionAborted => E::ConnectionAborted,
            ErrorKind::NotConnected => E::NotConnected,
            ErrorKind::AddrInUse => E::AddrInUse,
            ErrorKind::AddrNotAvailable => E::AddrNotAvailable,
            ErrorKind::BrokenPipe => E::BrokenPipe,
            ErrorKind::AlreadyExists => E::AlreadyExists,
            ErrorKind::InvalidInput => E::InvalidInput,
            ErrorKind::InvalidData => E::InvalidData,
            ErrorKind::TimedOut => E::TimedOut,
            ErrorKind::WriteZero => E::WriteZero,
            ErrorKind::Interrupted => E::Interrupted,
            ErrorKind::Unsupported => E::Unsupported,
            ErrorKind::OutOfMemory => E::OutOfMemory,
            ErrorKind::WouldBlock | ErrorKind::UnexpectedEof | ErrorKind::Other => E::Other,
        }
    }
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl core::error::Error for ErrorKind {}

impl embedded_io::Error for ErrorKind {
    fn kind(&self) -> embedded_io::ErrorKind {
        (*self).into()
    }
}

/// An I/O error: a portable [`ErrorKind`], optional static context and, with
/// `alloc`, the original error from the underlying device.
///
/// The public shape is identical with and without `alloc`. Without `alloc`
/// the source is dropped and only the kind survives.
#[non_exhaustive]
pub struct Error {
    kind: ErrorKind,
    message: Option<&'static str>,
    #[cfg(feature = "alloc")]
    source: Option<alloc::boxed::Box<dyn core::error::Error + Send + Sync + 'static>>,
}

impl Error {
    /// Construct an error without additional context.
    pub const fn from_kind(kind: ErrorKind) -> Self {
        Self {
            kind,
            message: None,
            #[cfg(feature = "alloc")]
            source: None,
        }
    }

    /// Construct an error with static context.
    pub const fn new(kind: ErrorKind, message: &'static str) -> Self {
        Self {
            kind,
            message: Some(message),
            #[cfg(feature = "alloc")]
            source: None,
        }
    }

    /// Construct an `Other` error with static context.
    pub const fn other(message: &'static str) -> Self {
        Self::new(ErrorKind::Other, message)
    }

    /// Convert an error from an `embedded-io` device.
    ///
    /// A Hadris [`Error`] or [`ErrorKind`] passes through unchanged. Any other
    /// error keeps its kind and, with `alloc`, is kept as the [`source`].
    ///
    /// [`source`]: core::error::Error::source
    pub fn from_io<E>(error: E) -> Self
    where
        E: embedded_io::Error + Send + Sync + 'static,
    {
        let mut slot = Some(error);
        let any: &mut dyn core::any::Any = &mut slot;
        if let Some(inner) = any.downcast_mut::<Option<Error>>() {
            if let Some(inner) = inner.take() {
                return inner;
            }
        }
        if let Some(kind) = any.downcast_mut::<Option<ErrorKind>>() {
            if let Some(kind) = kind.take() {
                return Self::from_kind(kind);
            }
        }
        match slot {
            Some(error) => Self::with_source(error.kind().into(), error),
            None => Self::from_kind(ErrorKind::Other),
        }
    }

    /// Construct an error that wraps `source`. Without `alloc` the source is
    /// dropped and only `kind` is kept.
    pub fn with_source<E>(kind: ErrorKind, source: E) -> Self
    where
        E: core::error::Error + Send + Sync + 'static,
    {
        #[cfg(feature = "alloc")]
        {
            Self {
                kind,
                message: None,
                source: Some(alloc::boxed::Box::new(source)),
            }
        }
        #[cfg(not(feature = "alloc"))]
        {
            let _ = source;
            Self::from_kind(kind)
        }
    }

    /// Return the portable error kind.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Return the static context message, if any.
    pub const fn message(&self) -> Option<&'static str> {
        self.message
    }

    /// Replace the static context message.
    pub const fn with_message(mut self, message: &'static str) -> Self {
        self.message = Some(message);
        self
    }

    /// Borrow the underlying source error as `E`, if it is one.
    pub fn downcast_source<E: core::error::Error + 'static>(&self) -> Option<&E> {
        core::error::Error::source(self)?.downcast_ref::<E>()
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self::from_kind(kind)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Error");
        s.field("kind", &self.kind);
        if let Some(message) = self.message {
            s.field("message", &message);
        }
        #[cfg(feature = "alloc")]
        if let Some(source) = &self.source {
            s.field("source", source);
        }
        s.finish()
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.message {
            Some(message) => write!(f, "{:?}: {message}", self.kind),
            None => {
                #[cfg(feature = "alloc")]
                if let Some(source) = &self.source {
                    return Display::fmt(source, f);
                }
                write!(f, "{:?}", self.kind)
            }
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        #[cfg(feature = "alloc")]
        if let Some(source) = &self.source {
            return Some(source.as_ref());
        }
        None
    }
}

impl embedded_io::Error for Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        self.kind.into()
    }
}

/// Result returned by Hadris I/O operations.
pub type Result<T> = core::result::Result<T, Error>;

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::with_source(ErrorKind::from_std(error.kind()), error)
    }
}

#[cfg(feature = "std")]
impl From<Error> for std::io::Error {
    fn from(error: Error) -> Self {
        let kind = std::io::ErrorKind::from(error.kind);
        if error.message.is_none() {
            if let Some(source) = error.source {
                return match source.downcast::<std::io::Error>() {
                    Ok(inner) => *inner,
                    Err(other) => std::io::Error::new(kind, other),
                };
            }
            return std::io::Error::from(kind);
        }
        std::io::Error::new(kind, error)
    }
}

#[cfg(feature = "std")]
impl From<ErrorKind> for std::io::ErrorKind {
    fn from(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::WouldBlock => std::io::ErrorKind::WouldBlock,
            ErrorKind::UnexpectedEof => std::io::ErrorKind::UnexpectedEof,
            _ => embedded_io::ErrorKind::from(kind).into(),
        }
    }
}

#[cfg(feature = "std")]
impl ErrorKind {
    /// Convert a `std::io::ErrorKind`.
    pub fn from_std(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::WouldBlock => Self::WouldBlock,
            std::io::ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            other => embedded_io::ErrorKind::from(other).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct DeviceError;

    impl Display for DeviceError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("device fault")
        }
    }

    impl core::error::Error for DeviceError {}

    impl embedded_io::Error for DeviceError {
        fn kind(&self) -> embedded_io::ErrorKind {
            embedded_io::ErrorKind::TimedOut
        }
    }

    #[test]
    fn from_io_passes_hadris_errors_through() {
        let original = Error::new(ErrorKind::InvalidData, "bad");
        let converted = Error::from_io(original);
        assert_eq!(converted.kind(), ErrorKind::InvalidData);
        assert_eq!(converted.message(), Some("bad"));
        assert!(core::error::Error::source(&converted).is_none());
        assert_eq!(
            Error::from_io(ErrorKind::NotFound).kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn from_io_keeps_kind_of_foreign_errors() {
        let converted = Error::from_io(DeviceError);
        assert_eq!(converted.kind(), ErrorKind::TimedOut);
        #[cfg(feature = "alloc")]
        assert!(converted.downcast_source::<DeviceError>().is_some());
        #[cfg(not(feature = "alloc"))]
        assert!(converted.downcast_source::<DeviceError>().is_none());
    }
}

#[cfg(all(test, feature = "std"))]
mod std_tests {
    extern crate std;
    use super::*;

    #[test]
    fn error_kind_to_std_preserves_unexpected_eof() {
        assert_eq!(
            std::io::ErrorKind::from(ErrorKind::UnexpectedEof),
            std::io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn error_kind_to_std_preserves_would_block() {
        assert_eq!(
            std::io::ErrorKind::from(ErrorKind::WouldBlock),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn std_error_round_trips_through_hadris_error() {
        let original = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "locked");
        let hadris = Error::from(original);
        assert_eq!(hadris.kind(), ErrorKind::PermissionDenied);
        let back = std::io::Error::from(hadris);
        assert_eq!(back.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::format!("{back}"), "locked");
    }

    #[test]
    fn context_error_converts_to_std() {
        let error = Error::new(ErrorKind::UnexpectedEof, "short read");
        let std_error = std::io::Error::from(error);
        assert_eq!(std_error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
