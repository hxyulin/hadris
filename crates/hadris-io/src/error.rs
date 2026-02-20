//! No-std compatible I/O error types

use core::fmt::{self, Display};

/// Error kind for I/O operations (no-std compatible)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// An entity was not found
    NotFound,
    /// Permission denied
    PermissionDenied,
    /// Connection refused
    ConnectionRefused,
    /// Connection reset
    ConnectionReset,
    /// Connection aborted
    ConnectionAborted,
    /// Not connected
    NotConnected,
    /// Address in use
    AddrInUse,
    /// Address not available
    AddrNotAvailable,
    /// Broken pipe
    BrokenPipe,
    /// Entity already exists
    AlreadyExists,
    /// Operation would block
    WouldBlock,
    /// Invalid input
    InvalidInput,
    /// Invalid data
    InvalidData,
    /// Timed out
    TimedOut,
    /// Write zero
    WriteZero,
    /// Interrupted
    Interrupted,
    /// Unexpected end of file
    UnexpectedEof,
    /// Out of memory
    OutOfMemory,
    /// Other error
    Other,
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::NotFound => write!(f, "entity not found"),
            ErrorKind::PermissionDenied => write!(f, "permission denied"),
            ErrorKind::ConnectionRefused => write!(f, "connection refused"),
            ErrorKind::ConnectionReset => write!(f, "connection reset"),
            ErrorKind::ConnectionAborted => write!(f, "connection aborted"),
            ErrorKind::NotConnected => write!(f, "not connected"),
            ErrorKind::AddrInUse => write!(f, "address in use"),
            ErrorKind::AddrNotAvailable => write!(f, "address not available"),
            ErrorKind::BrokenPipe => write!(f, "broken pipe"),
            ErrorKind::AlreadyExists => write!(f, "entity already exists"),
            ErrorKind::WouldBlock => write!(f, "operation would block"),
            ErrorKind::InvalidInput => write!(f, "invalid input parameter"),
            ErrorKind::InvalidData => write!(f, "invalid data"),
            ErrorKind::TimedOut => write!(f, "operation timed out"),
            ErrorKind::WriteZero => write!(f, "write zero"),
            ErrorKind::Interrupted => write!(f, "operation interrupted"),
            ErrorKind::UnexpectedEof => write!(f, "unexpected end of file"),
            ErrorKind::OutOfMemory => write!(f, "out of memory"),
            ErrorKind::Other => write!(f, "other error"),
        }
    }
}

/// I/O Error type for no-std environments
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
}

impl Error {
    /// Create a new error with the given kind
    pub const fn new(kind: ErrorKind, _msg: &'static str) -> Self {
        Self { kind }
    }

    /// Create a new error from an error kind
    pub const fn from_kind(kind: ErrorKind) -> Self {
        Self { kind }
    }

    /// Create an error with `ErrorKind::Other`.
    ///
    /// This mirrors `std::io::Error::other()` for no-std compatibility.
    pub const fn other(_msg: &'static str) -> Self {
        Self {
            kind: ErrorKind::Other,
        }
    }

    /// Get the error kind
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self { kind }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

/// Result type alias for I/O operations
pub type Result<T> = core::result::Result<T, Error>;
