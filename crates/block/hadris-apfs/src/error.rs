use core::fmt;

/// APFS operation error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// The image uses an APFS feature this reader does not implement.
    Unsupported(&'static str),
}

/// Result type used by APFS operations.
pub type Result<T> = core::result::Result<T, ApfsError>;

impl fmt::Display for ApfsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooSmall => f.write_str("APFS input too small"),
            Self::InvalidMagic { expected, actual } => {
                write!(f, "invalid APFS magic {actual:?}, expected {expected:?}",)
            }
            Self::InvalidValue(name) => write!(f, "invalid APFS value: {name}"),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "APFS checksum mismatch: expected {expected:#x}, got {actual:#x}"
            ),
            Self::AddressOverflow => f.write_str("APFS address calculation overflowed"),
            Self::Unsupported(feature) => write!(f, "unsupported APFS feature: {feature}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ApfsError {}

/// APFS error detail recorded on shared filesystem errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// An APFS signature is missing.
    Magic = 1,
    /// An on-disk structure is malformed or truncated.
    Structure = 2,
    /// An object checksum is invalid.
    Checksum = 3,
    /// An address calculation overflowed or exceeded the container.
    Address = 4,
    /// An APFS feature has no implementation in this reader.
    Feature = 5,
    /// The container needs an explicit volume selection.
    VolumeSelection = 6,
    /// Software-encryption credentials did not unlock the selected volume.
    Credentials = 7,
}

impl Detail {
    /// Returns the stable APFS detail code.
    pub const fn code(self) -> hadris_fs::DetailCode {
        hadris_fs::DetailCode::new("hadris-apfs", self as u16)
    }

    /// Reads an APFS detail from a shared error.
    pub fn of<E>(error: &hadris_fs::Error<E>) -> Option<Self> {
        let code = error.detail()?.code_in("hadris-apfs")?;
        match code {
            1 => Some(Self::Magic),
            2 => Some(Self::Structure),
            3 => Some(Self::Checksum),
            4 => Some(Self::Address),
            5 => Some(Self::Feature),
            6 => Some(Self::VolumeSelection),
            7 => Some(Self::Credentials),
            _ => None,
        }
    }
}

impl<E> From<ApfsError> for hadris_fs::Error<E> {
    fn from(error: ApfsError) -> Self {
        use hadris_fs::ErrorKind;
        let (kind, message, detail) = match error {
            ApfsError::InputTooSmall => (
                ErrorKind::Corrupt,
                "truncated APFS structure",
                Detail::Structure,
            ),
            ApfsError::InvalidMagic { .. } => {
                (ErrorKind::Corrupt, "invalid APFS magic", Detail::Magic)
            }
            ApfsError::InvalidValue(message) => (ErrorKind::Corrupt, message, Detail::Structure),
            ApfsError::ChecksumMismatch { .. } => (
                ErrorKind::Corrupt,
                "APFS checksum mismatch",
                Detail::Checksum,
            ),
            ApfsError::AddressOverflow => {
                (ErrorKind::Corrupt, "APFS address overflow", Detail::Address)
            }
            ApfsError::Unsupported(message) => (ErrorKind::Unsupported, message, Detail::Feature),
        };
        Self::new(kind, message).with_detail(detail.code())
    }
}
