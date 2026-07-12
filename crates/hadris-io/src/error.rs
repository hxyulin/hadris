//! Portable, allocation-free I/O errors.

use core::fmt::{self, Display};

/// Portable error classification. This is a superset of `embedded_io::ErrorKind`
/// and retains the `std::io` conditions Hadris needs for helper operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    BrokenPipe,
    AlreadyExists,
    WouldBlock,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    Interrupted,
    UnexpectedEof,
    Unsupported,
    OutOfMemory,
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

/// An error produced either by an underlying I/O object or by a Hadris helper.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error<E = ErrorKind> {
    /// Error returned by the underlying reader, writer, or seeker.
    Source(E),
    /// Error synthesized by Hadris, with optional allocation-free context.
    Context {
        /// Portable classification of the error.
        kind: ErrorKind,
        /// Static diagnostic context.
        message: Option<&'static str>,
    },
}

impl<E> Error<E> {
    /// Wrap an error returned by the underlying I/O object.
    pub const fn from_source(source: E) -> Self {
        Self::Source(source)
    }

    /// Construct a Hadris-generated error without additional context.
    pub const fn from_kind(kind: ErrorKind) -> Self {
        Self::Context {
            kind,
            message: None,
        }
    }

    /// Construct a Hadris-generated error with static context.
    pub const fn new(kind: ErrorKind, message: &'static str) -> Self {
        Self::Context {
            kind,
            message: Some(message),
        }
    }

    /// Construct an `Other` error with static context.
    pub const fn other(message: &'static str) -> Self {
        Self::new(ErrorKind::Other, message)
    }

    /// Borrow the underlying source, if present.
    pub const fn source_ref(&self) -> Option<&E> {
        match self {
            Self::Source(source) => Some(source),
            Self::Context { .. } => None,
        }
    }

    /// Consume the error and return its underlying source, if present.
    pub fn into_source(self) -> Option<E> {
        match self {
            Self::Source(source) => Some(source),
            Self::Context { .. } => None,
        }
    }
}

impl<E: embedded_io::Error> Error<E> {
    /// Return the portable error kind.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Source(source) => source.kind().into(),
            Self::Context { kind, .. } => *kind,
        }
    }

    /// Erase the concrete source while retaining its normalized kind.
    pub fn erase(self) -> Error<ErrorKind> {
        match self {
            Self::Source(source) => Error::Source(source.kind().into()),
            Self::Context { kind, message } => Error::Context { kind, message },
        }
    }
}

impl<E: embedded_io::Error + 'static> embedded_io::Error for Error<E> {
    fn kind(&self) -> embedded_io::ErrorKind {
        Error::<E>::kind(self).into()
    }
}

impl<E: Display> Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(source) => Display::fmt(source, f),
            Self::Context {
                kind,
                message: Some(message),
            } => {
                write!(f, "{kind:?}: {message}")
            }
            Self::Context {
                kind,
                message: None,
            } => write!(f, "{kind:?}"),
        }
    }
}

impl<E> core::error::Error for Error<E>
where
    E: core::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.source_ref()
            .map(|source| source as &(dyn core::error::Error + 'static))
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error<std::io::Error> {
    fn from(error: std::io::Error) -> Self {
        Self::Source(error)
    }
}

#[cfg(feature = "std")]
impl From<Error<std::io::Error>> for std::io::Error {
    fn from(error: Error<std::io::Error>) -> Self {
        match error {
            Error::Source(source) => source,
            Error::Context { kind, message } => match message {
                Some(message) => std::io::Error::new(std::io::ErrorKind::from(kind), message),
                None => std::io::Error::from(std::io::ErrorKind::from(kind)),
            },
        }
    }
}

/// Result returned by Hadris helpers and filesystem operations.
pub type Result<T, E = ErrorKind> = core::result::Result<T, Error<E>>;

#[cfg(feature = "std")]
impl From<ErrorKind> for std::io::ErrorKind {
    fn from(kind: ErrorKind) -> Self {
        match embedded_io::ErrorKind::from(kind) {
            embedded => embedded.into(),
        }
    }
}
