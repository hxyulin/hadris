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

/// Error of `OpenVolume::open` and `OpenVolume::open_detected`: an [`Error`]
/// and, in every case the opener can arrange, the device it was given.
///
/// The device is missing only when it fails while the validated volume is
/// mounted for keeps, after a first mount on a borrow of it succeeded. `?`
/// converts it into [`Error`], dropping the device.
pub struct OpenError<D, E> {
    error: Error<E>,
    device: Option<D>,
}

impl<D, E> OpenError<D, E> {
    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn new(error: Error<E>, device: D) -> Self {
        Self {
            error,
            device: Some(device),
        }
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn without_device(error: Error<E>) -> Self {
        Self {
            error,
            device: None,
        }
    }

    /// Borrows the reason the open failed.
    pub fn error(&self) -> &Error<E> {
        &self.error
    }

    /// Returns the reason the open failed, dropping the device.
    pub fn into_error(self) -> Error<E> {
        self.error
    }

    /// Returns the device, if it could be given back.
    pub fn into_device(self) -> Option<D> {
        self.device
    }

    /// Returns the reason and the device.
    pub fn into_parts(self) -> (Error<E>, Option<D>) {
        (self.error, self.device)
    }
}

impl<D, E: fmt::Debug> fmt::Debug for OpenError<D, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenError")
            .field("error", &self.error)
            .field("device", &self.device.as_ref().map(|_| ..))
            .finish()
    }
}

impl<D, E: fmt::Display> fmt::Display for OpenError<D, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<D, E: core::error::Error + 'static> core::error::Error for OpenError<D, E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.error.source()
    }
}

impl<D, E> From<OpenError<D, E>> for Error<E> {
    fn from(error: OpenError<D, E>) -> Self {
        error.error
    }
}
