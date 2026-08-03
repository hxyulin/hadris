use core::fmt;

/// APFS operation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApfsError {
    /// Input ended before the requested structure could be parsed.
    InputTooSmall,
    /// The APFS magic value was not present.
    InvalidMagic {
        /// Expected magic bytes.
        expected: [u8; 4],
        /// Actual magic bytes found in input.
        actual: [u8; 4],
    },
    /// A field contained an invalid value.
    InvalidValue(&'static str),
    /// Fletcher checksum verification failed.
    ChecksumMismatch {
        /// Checksum stored in the object header.
        expected: u64,
        /// Checksum computed from the object bytes.
        actual: u64,
    },
    /// Arithmetic overflow while calculating an address or length.
    AddressOverflow,
    /// Underlying I/O failed.
    Io(hadris_io::ErrorKind),
}

/// Result type used by APFS operations.
pub type Result<T> = core::result::Result<T, ApfsError>;

impl ApfsError {
    /// Converts a portable I/O error into an APFS error without retaining the backend type.
    pub fn from_io<E: hadris_io::IoError>(error: hadris_io::Error<E>) -> Self {
        Self::Io(error.kind())
    }
}

impl fmt::Display for ApfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooSmall => f.write_str("APFS input too small"),
            Self::InvalidMagic { expected, actual } => write!(
                f,
                "invalid APFS magic {:?}, expected {:?}",
                actual, expected
            ),
            Self::InvalidValue(name) => write!(f, "invalid APFS value: {name}"),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "APFS checksum mismatch: expected {expected:#x}, got {actual:#x}"
            ),
            Self::AddressOverflow => f.write_str("APFS address calculation overflowed"),
            Self::Io(kind) => write!(f, "APFS I/O error: {kind:?}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ApfsError {}
