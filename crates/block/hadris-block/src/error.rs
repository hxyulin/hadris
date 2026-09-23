use core::fmt;

use crate::detect::{BlockFormat, FatVariant, PartitionTableKind};

/// Error returned by category-level block operations on a device whose
/// errors are `E`.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error<E> {
    /// The device failed while the format was being detected.
    Device(E),
    /// No supported format was recognized.
    UnknownFormat,
    /// The source is a partitioned disk rather than a directly openable volume.
    PartitionedDisk(PartitionTableKind),
    /// The detected format has no category-level opener enabled.
    UnsupportedFormat(BlockFormat),
    /// Cheap detection and full filesystem validation disagreed.
    DetectedFormatMismatch {
        /// Format reported by lightweight detection.
        detected: FatVariant,
        /// Format reported after the filesystem was fully opened.
        opened: FatVariant,
    },
    /// The FAT driver failed to mount the volume.
    Fat(hadris_fs::Error<E>),
}

/// Result type for category-level block operations.
pub type Result<T, E> = core::result::Result<T, Error<E>>;

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(error) => write!(formatter, "block detection I/O error: {error}"),
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

impl<E: core::error::Error + 'static> core::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Device(error) => Some(error),
            Self::Fat(error) => Some(error),
            _ => None,
        }
    }
}

impl<E> From<hadris_fs::Error<E>> for Error<E> {
    fn from(error: hadris_fs::Error<E>) -> Self {
        Self::Fat(error)
    }
}
