use core::fmt;

use hadris_fs::{DetailCode, Error, ErrorKind};

const DOMAIN: &str = "hadris-apfs";

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

impl core::error::Error for ApfsError {}

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
    const ALL: [Self; 7] = [
        Self::Magic,
        Self::Structure,
        Self::Checksum,
        Self::Address,
        Self::Feature,
        Self::VolumeSelection,
        Self::Credentials,
    ];

    /// Returns the stable APFS detail code.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    /// Reads an APFS detail from a shared error.
    pub fn of<E>(error: &Error<E>) -> Option<Self> {
        error.detail().and_then(Self::from_code)
    }

    /// Reads an APFS detail code, ignoring unknown codes and other domains.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }
}

impl From<Detail> for DetailCode {
    fn from(detail: Detail) -> Self {
        detail.code()
    }
}

impl<E> From<ApfsError> for Error<E> {
    fn from(error: ApfsError) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_errors_implement_core_error_in_every_feature_tier() {
        fn assert_error<T: core::error::Error + Send + Sync + 'static>() {}
        assert_error::<ApfsError>();
    }

    #[test]
    fn detail_codes_are_pinned() {
        let pinned = [
            (Detail::Magic, 1),
            (Detail::Structure, 2),
            (Detail::Checksum, 3),
            (Detail::Address, 4),
            (Detail::Feature, 5),
            (Detail::VolumeSelection, 6),
            (Detail::Credentials, 7),
        ];
        assert_eq!(pinned.len(), Detail::ALL.len());
        for (detail, number) in pinned {
            let code = DetailCode::new("hadris-apfs", number);
            assert_eq!(detail.code(), code);
            assert_eq!(Detail::from_code(code), Some(detail));
            assert_eq!(DetailCode::from(detail), code);
            let err: Error<()> = Error::new(ErrorKind::Corrupt, "").with_detail(code);
            assert_eq!(Detail::of(&err), Some(detail));
        }
        assert_eq!(Detail::from_code(DetailCode::new("other-crate", 1)), None);
        assert_eq!(Detail::from_code(DetailCode::new(DOMAIN, u16::MAX)), None);
    }
}
