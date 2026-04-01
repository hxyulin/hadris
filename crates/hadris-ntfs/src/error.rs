//! Error types for the hadris-ntfs crate.

use core::fmt;

/// Errors that can occur when working with NTFS filesystems.
#[derive(Debug)]
pub enum NtfsError {
    /// Invalid boot sector signature (expected 0xAA55)
    InvalidBootSignature {
        /// The signature that was found
        found: u16,
    },
    /// Invalid OEM ID in boot sector (expected "NTFS    ")
    InvalidOemId,
    /// Invalid MFT record magic (expected "FILE")
    InvalidMftMagic,
    /// Invalid index record magic (expected "INDX")
    InvalidIndexMagic,
    /// Invalid or corrupt update sequence (fixup) array
    InvalidFixup,
    /// Update sequence entry does not match the expected value
    FixupMismatch {
        /// Expected update sequence number
        expected: u16,
        /// Value found at sector boundary
        found: u16,
    },
    /// Invalid MFT record size in boot sector
    InvalidRecordSize,
    /// MFT record index is beyond the MFT data extent
    MftRecordOutOfBounds {
        /// The record index that was requested
        index: u64,
    },
    /// Required attribute was not found in the MFT record
    AttributeNotFound {
        /// The attribute type that was expected
        attr_type: u32,
    },
    /// Malformed attribute header or value
    InvalidAttribute,
    /// Could not decode a UTF-16LE filename
    InvalidFileName,
    /// Malformed index entry
    InvalidIndexEntry,
    /// Entry is not a regular file
    NotAFile,
    /// Entry is not a directory
    NotADirectory,
    /// Entry not found in directory
    EntryNotFound,
    /// Path is invalid (empty or malformed)
    InvalidPath,
    /// Compressed data streams are not supported
    UnsupportedCompression,
    /// Encrypted data streams are not supported
    UnsupportedEncryption,
    /// Data read went past the end of the available data runs
    UnexpectedEndOfData,
    /// I/O error from the underlying storage
    Io(hadris_io::Error),
}

impl fmt::Display for NtfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBootSignature { found } => {
                write!(
                    f,
                    "invalid boot signature: expected 0xAA55, found {found:#06x}"
                )
            }
            Self::InvalidOemId => write!(f, "invalid OEM ID (expected \"NTFS    \")"),
            Self::InvalidMftMagic => write!(f, "invalid MFT record magic (expected \"FILE\")"),
            Self::InvalidIndexMagic => {
                write!(f, "invalid index record magic (expected \"INDX\")")
            }
            Self::InvalidFixup => write!(f, "invalid or corrupt update sequence array"),
            Self::FixupMismatch { expected, found } => {
                write!(
                    f,
                    "fixup mismatch: expected {expected:#06x}, found {found:#06x}"
                )
            }
            Self::InvalidRecordSize => write!(f, "invalid record size in boot sector"),
            Self::MftRecordOutOfBounds { index } => {
                write!(f, "MFT record index {index} is out of bounds")
            }
            Self::AttributeNotFound { attr_type } => {
                write!(f, "attribute type {attr_type:#06x} not found")
            }
            Self::InvalidAttribute => write!(f, "malformed attribute header or value"),
            Self::InvalidFileName => write!(f, "could not decode UTF-16LE filename"),
            Self::InvalidIndexEntry => write!(f, "malformed index entry"),
            Self::NotAFile => write!(f, "entry is not a file"),
            Self::NotADirectory => write!(f, "entry is not a directory"),
            Self::EntryNotFound => write!(f, "entry not found in directory"),
            Self::InvalidPath => write!(f, "path is invalid (empty or malformed)"),
            Self::UnsupportedCompression => {
                write!(f, "compressed data streams are not supported")
            }
            Self::UnsupportedEncryption => {
                write!(f, "encrypted data streams are not supported")
            }
            Self::UnexpectedEndOfData => write!(f, "unexpected end of data runs"),
            Self::Io(e) => write!(f, "I/O error: {e:?}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for NtfsError {}

impl From<hadris_io::Error> for NtfsError {
    fn from(e: hadris_io::Error) -> Self {
        Self::Io(e)
    }
}

/// Result type alias for NTFS operations.
pub type Result<T> = core::result::Result<T, NtfsError>;
