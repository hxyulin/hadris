//! UDF-specific error types

use hadris_io as io;

/// Errors that can occur when reading or writing UDF filesystems
#[derive(Debug)]
pub enum UdfError {
    /// I/O error
    Io(io::Error),
    /// Invalid or missing Volume Recognition Sequence
    InvalidVrs,
    /// No Anchor Volume Descriptor Pointer found
    NoAnchor,
    /// Invalid descriptor tag
    InvalidTag { expected: u16, found: u16 },
    /// Descriptor CRC mismatch
    CrcMismatch { expected: u16, computed: u16 },
    /// Unsupported UDF revision
    UnsupportedRevision(u16),
    /// Invalid partition reference
    InvalidPartition(u16),
    /// Invalid ICB (Information Control Block)
    InvalidIcb,
    /// File not found
    NotFound,
    /// Not a directory
    NotADirectory,
    /// Not a file
    NotAFile,
    /// Path too long
    PathTooLong,
    /// Invalid filename encoding
    InvalidEncoding,
}

impl From<io::Error> for UdfError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl core::fmt::Display for UdfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::InvalidVrs => write!(f, "invalid or missing Volume Recognition Sequence"),
            Self::NoAnchor => write!(f, "no Anchor Volume Descriptor Pointer found"),
            Self::InvalidTag { expected, found } => {
                write!(
                    f,
                    "invalid descriptor tag: expected {}, found {}",
                    expected, found
                )
            }
            Self::CrcMismatch { expected, computed } => {
                write!(
                    f,
                    "CRC mismatch: expected {:04x}, computed {:04x}",
                    expected, computed
                )
            }
            Self::UnsupportedRevision(rev) => {
                write!(f, "unsupported UDF revision: {:04x}", rev)
            }
            Self::InvalidPartition(num) => write!(f, "invalid partition reference: {}", num),
            Self::InvalidIcb => write!(f, "invalid Information Control Block"),
            Self::NotFound => write!(f, "file or directory not found"),
            Self::NotADirectory => write!(f, "not a directory"),
            Self::NotAFile => write!(f, "not a file"),
            Self::PathTooLong => write!(f, "path too long"),
            Self::InvalidEncoding => write!(f, "invalid filename encoding"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for UdfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Result type for UDF operations
pub type UdfResult<T> = Result<T, UdfError>;
