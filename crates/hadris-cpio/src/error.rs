use core::fmt;

/// Errors that can occur during CPIO archive operations.
#[derive(Debug)]
pub enum CpioError {
    /// An I/O error occurred while reading or writing the archive.
    Io(hadris_io::Error),
    /// The header magic bytes are not `070701` or `070702`.
    InvalidMagic { found: [u8; 6] },
    /// A header field contains non-hexadecimal characters.
    InvalidHexField { field: &'static str },
    /// The entry filename is empty or could not be read.
    InvalidFilename,
    /// The archive ended without a `TRAILER!!!` sentinel.
    MissingTrailer,
    /// The CRC checksum in a `070702` entry does not match the computed value.
    ChecksumMismatch { expected: u32, computed: u32 },
    /// A hard link references a target path that was not seen earlier in the archive.
    #[cfg(feature = "write")]
    UnresolvedHardLink { ino: u32 },
    /// The filename exceeds the maximum length representable in a newc header.
    #[cfg(feature = "write")]
    FilenameTooLong,
    /// The file data exceeds the maximum size representable in a newc header (4 GiB).
    #[cfg(feature = "write")]
    FileTooLarge,
}

impl fmt::Display for CpioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e:?}"),
            Self::InvalidMagic { found } => {
                write!(
                    f,
                    "invalid CPIO magic: expected 070701 or 070702, found {:?}",
                    core::str::from_utf8(found).unwrap_or("<invalid>")
                )
            }
            Self::InvalidHexField { field } => {
                write!(f, "invalid hex field: {field}")
            }
            Self::InvalidFilename => write!(f, "invalid filename"),
            Self::MissingTrailer => write!(f, "missing TRAILER!!! sentinel"),
            Self::ChecksumMismatch { expected, computed } => {
                write!(
                    f,
                    "checksum mismatch: expected {expected:#010x}, computed {computed:#010x}"
                )
            }
            #[cfg(feature = "write")]
            Self::UnresolvedHardLink { ino } => {
                write!(f, "unresolved hard link: inode {ino}")
            }
            #[cfg(feature = "write")]
            Self::FilenameTooLong => write!(f, "filename too long"),
            #[cfg(feature = "write")]
            Self::FileTooLarge => write!(f, "file too large"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for CpioError {}

impl From<hadris_io::Error> for CpioError {
    fn from(e: hadris_io::Error) -> Self {
        Self::Io(e)
    }
}

/// Convenience type alias for `core::result::Result<T, CpioError>`.
pub type Result<T> = core::result::Result<T, CpioError>;
