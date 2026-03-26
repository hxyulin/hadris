//! No-std compatible I/O error types

use core::fmt::{self, Display};

#[cfg(feature = "alloc")]
extern crate alloc;

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

// ---------------------------------------------------------------------------
// Error message storage
// ---------------------------------------------------------------------------

/// Internal message representation.
///
/// Always has the `Static` variant. The `Dynamic` variant is only available
/// when the `alloc` feature is enabled, allowing heap-allocated messages.
#[derive(Debug)]
enum ErrorMessage {
    Static(&'static str),
    #[cfg(feature = "alloc")]
    Dynamic(alloc::string::String),
}

impl ErrorMessage {
    fn as_str(&self) -> &str {
        match self {
            ErrorMessage::Static(s) => s,
            #[cfg(feature = "alloc")]
            ErrorMessage::Dynamic(s) => s.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// I/O Error type for no-std environments
///
/// When the `alloc` feature is enabled, errors can store dynamic messages
/// via heap allocation. Without `alloc`, only `&'static str` messages are
/// supported.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    msg: ErrorMessage,
}

impl Error {
    /// Create a new error with the given kind and a static message.
    ///
    /// This is always available and `const`-compatible.
    pub const fn new_static(kind: ErrorKind, msg: &'static str) -> Self {
        Self {
            kind,
            msg: ErrorMessage::Static(msg),
        }
    }

    /// Create a new error with the given kind and message.
    ///
    /// Without `alloc`, the message must be a `&'static str`.
    /// With `alloc`, any type implementing `Into<String>` is accepted.
    #[cfg(not(feature = "alloc"))]
    pub const fn new(kind: ErrorKind, msg: &'static str) -> Self {
        Self::new_static(kind, msg)
    }

    /// Create a new error with the given kind and message.
    ///
    /// Accepts any type implementing `Into<String>`, including `&str`,
    /// `String`, and `format!(...)` results.
    #[cfg(feature = "alloc")]
    pub fn new(kind: ErrorKind, msg: impl Into<alloc::string::String>) -> Self {
        Self {
            kind,
            msg: ErrorMessage::Dynamic(msg.into()),
        }
    }

    /// Create a new error from an error kind
    pub const fn from_kind(kind: ErrorKind) -> Self {
        Self {
            kind,
            msg: ErrorMessage::Static(""),
        }
    }

    /// Create an error with `ErrorKind::Other`.
    ///
    /// This mirrors `std::io::Error::other()` for no-std compatibility.
    #[cfg(not(feature = "alloc"))]
    pub const fn other(msg: &'static str) -> Self {
        Self {
            kind: ErrorKind::Other,
            msg: ErrorMessage::Static(msg),
        }
    }

    /// Create an error with `ErrorKind::Other`.
    ///
    /// This mirrors `std::io::Error::other()` for no-std compatibility.
    /// Accepts any type implementing `Into<String>`.
    #[cfg(feature = "alloc")]
    pub fn other(msg: impl Into<alloc::string::String>) -> Self {
        Self {
            kind: ErrorKind::Other,
            msg: ErrorMessage::Dynamic(msg.into()),
        }
    }

    /// Get the error kind
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self::from_kind(kind)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = self.msg.as_str();
        if msg.is_empty() {
            write!(f, "{}", self.kind)
        } else {
            write!(f, "{}: {}", self.kind, msg)
        }
    }
}

/// Result type alias for I/O operations
pub type Result<T> = core::result::Result<T, Error>;
