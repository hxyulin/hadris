use core::fmt;

use hadris_fs::ErrorKind;

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which structure is at
/// fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The OEM identifier or the end of sector marker of the boot sector is
    /// wrong: the device holds no NTFS volume.
    BootSector,
    /// The sector size, cluster size, record sizes, volume size or `$MFT`
    /// location in the boot sector is invalid.
    Geometry,
    /// An MFT or index record is larger than the 4096 bytes the reader
    /// handles.
    RecordSize,
    /// The device's blocks are larger than 4096 bytes.
    BlockSize,
    /// An MFT record has the wrong magic or is not in use, or an index
    /// record has the wrong magic.
    Record,
    /// An update sequence array does not match its record.
    UpdateSequence,
    /// A file reference names a record that was reused for another file.
    StaleReference,
    /// An attribute is malformed, or one a structure needs is missing.
    Attribute,
    /// Mapping pairs are malformed or end before the attribute's data.
    DataRun,
    /// A `$FILE_NAME` value is malformed.
    FileName,
    /// An index root, index record or index entry is malformed.
    Index,
    /// `$UpCase` is missing or not 65536 entries long.
    Upcase,
    /// The stream is compressed.
    Compressed,
    /// The stream is encrypted.
    Encrypted,
    /// An `$ATTRIBUTE_LIST` is malformed or names a missing attribute, or
    /// `$MFT` has more than the 32 extents the reader keeps.
    AttributeList,
    /// A structure lies outside the device.
    OutsideVolume,
}

impl Detail {
    const fn description(self) -> &'static str {
        match self {
            Self::BootSector => "not an NTFS boot sector",
            Self::Geometry => "invalid volume geometry",
            Self::RecordSize => "records larger than 4096 bytes",
            Self::BlockSize => "device blocks larger than 4096 bytes",
            Self::Record => "invalid or unused record",
            Self::UpdateSequence => "update sequence mismatch",
            Self::StaleReference => "stale file reference",
            Self::Attribute => "malformed or missing attribute",
            Self::DataRun => "malformed mapping pairs",
            Self::FileName => "malformed file name",
            Self::Index => "malformed index",
            Self::Upcase => "missing or malformed $UpCase",
            Self::Compressed => "compressed stream",
            Self::Encrypted => "encrypted stream",
            Self::AttributeList => "malformed or unsupported $ATTRIBUTE_LIST",
            Self::OutsideVolume => "a structure lies outside the device",
        }
    }

    const fn kind(self) -> ErrorKind {
        match self {
            Self::RecordSize | Self::BlockSize | Self::Compressed | Self::Encrypted => {
                ErrorKind::Unsupported
            }
            _ => ErrorKind::Corrupt,
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// Error of reading an NTFS volume on a device with error `E`.
///
/// Like [`hadris_fs::Error`], it keeps the device's own error without
/// allocation. Callers match on [`kind`](Self::kind); [`detail`](Self::detail)
/// names the structure. Two errors are equal when their kinds and device
/// errors are.
///
/// `?` converts it into [`hadris_fs::Error<E>`], and with `std` into
/// [`std::io::Error`], returning an `io::Error` device error as itself.
#[derive(Debug)]
pub struct Error<E> {
    kind: ErrorKind,
    detail: Option<Detail>,
    device: Option<E>,
}

impl<E> Error<E> {
    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            detail: Some(detail),
            device: None,
        }
    }

    /// What went wrong. [`ErrorKind::Io`] when the device failed.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which structure it concerns, when known.
    pub const fn detail(&self) -> Option<Detail> {
        self.detail
    }

    /// The device error, if the device failed.
    pub const fn device_error(&self) -> Option<&E> {
        self.device.as_ref()
    }

    /// Takes the device error, if the device failed.
    pub fn into_device_error(self) -> Option<E> {
        self.device
    }

    /// Converts the device error.
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> Error<F> {
        Error {
            kind: self.kind,
            detail: self.detail,
            device: self.device.map(f),
        }
    }
}

impl<E> From<Detail> for Error<E> {
    fn from(detail: Detail) -> Self {
        Self::new(detail.kind(), detail)
    }
}

impl<E: PartialEq> PartialEq for Error<E> {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.device == other.device
    }
}

impl<E: Eq> Eq for Error<E> {}

impl<E> From<ErrorKind> for Error<E> {
    fn from(kind: ErrorKind) -> Self {
        Self {
            kind,
            detail: None,
            device: None,
        }
    }
}

impl<E> From<hadris_fs::Error<E>> for Error<E> {
    fn from(err: hadris_fs::Error<E>) -> Self {
        Self {
            kind: err.kind(),
            detail: None,
            device: err.into_device_error(),
        }
    }
}

impl<E> From<Error<E>> for hadris_fs::Error<E> {
    fn from(err: Error<E>) -> Self {
        match err.device {
            Some(device) => hadris_fs::Error::device(device, "device failed"),
            None => err.kind.into(),
        }
    }
}

#[cfg(feature = "alloc")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for hadris_fs::AnyError {
    fn from(err: Error<E>) -> Self {
        hadris_fs::Error::from(err).into()
    }
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(device) = &self.device {
            return write!(f, "device error: {device}");
        }
        match self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.kind),
            None => self.kind.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.device
            .as_ref()
            .map(|device| device as &(dyn core::error::Error + 'static))
    }
}

/// A device error that is a `std::io::Error` comes back as itself.
#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for std::io::Error {
    fn from(err: Error<E>) -> Self {
        if err.device.is_some() {
            return hadris_fs::Error::from(err).into();
        }
        std::io::Error::new(err.kind.into(), StdMessage(err.kind, err.detail))
    }
}

#[cfg(feature = "std")]
#[derive(Debug)]
struct StdMessage(ErrorKind, Option<Detail>);

#[cfg(feature = "std")]
impl fmt::Display for StdMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.1 {
            Some(detail) => write!(f, "{}: {detail}", self.0),
            None => self.0.fmt(f),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for StdMessage {}
