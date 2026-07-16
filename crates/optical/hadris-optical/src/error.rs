use core::fmt;

/// Filesystem requested from an optical image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpticalFormat {
    /// An ISO 9660 filesystem.
    Iso9660,
    /// A Universal Disk Format filesystem.
    Udf,
}

/// Error returned by category-level optical operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The source could not be read or repositioned during detection.
    Io(hadris_io::Error),
    /// No supported optical filesystem was recognized.
    UnknownFormat,
    /// The requested filesystem is not present in the image.
    RequestedFormatUnavailable(OpticalFormat),
    /// ISO 9660 validation or opening failed.
    Iso(hadris_io::Error),
    /// UDF validation or opening failed.
    Udf(hadris_udf::UdfError),
}

/// Result type for category-level optical operations.
pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "optical image I/O error: {error}"),
            Self::UnknownFormat => formatter.write_str("unknown optical image format"),
            Self::RequestedFormatUnavailable(format) => {
                write!(
                    formatter,
                    "requested optical format is unavailable: {format:?}"
                )
            }
            Self::Iso(error) => write!(formatter, "ISO 9660 open failed: {error}"),
            Self::Udf(error) => write!(formatter, "UDF open failed: {error}"),
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
