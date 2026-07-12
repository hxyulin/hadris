use core::fmt;

use crate::detect::{BlockFormat, FatVariant, PartitionTableKind};

/// Error returned by category-level block operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The source could not be read or repositioned.
    Io(hadris_io::Error),
    /// No supported format was recognized.
    UnknownFormat,
    /// The source is a partitioned disk rather than a directly openable volume.
    PartitionedDisk(PartitionTableKind),
    /// The detected format has no category-level opener enabled.
    UnsupportedFormat(BlockFormat),
    /// Cheap detection and full filesystem validation disagreed.
    DetectedFormatMismatch {
        detected: FatVariant,
        opened: FatVariant,
    },
    /// FAT validation failed.
    Fat(hadris_fat::FatError),
}

pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "block detection I/O error: {error}"),
            Self::UnknownFormat => formatter.write_str("unknown block volume format"),
            Self::PartitionedDisk(kind) => write!(
                formatter,
                "{kind:?} disk must be opened through a partition view"
            ),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "unsupported block format: {format:?}")
            }
            Self::DetectedFormatMismatch { detected, opened } => write!(
                formatter,
                "detected {detected:?}, but full validation opened {opened:?}"
            ),
            Self::Fat(error) => write!(formatter, "FAT open failed: {error}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

impl From<hadris_io::Error> for Error {
    fn from(error: hadris_io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<hadris_fat::FatError> for Error {
    fn from(error: hadris_fat::FatError) -> Self {
        Self::Fat(error)
    }
}
