use core::fmt;

use hadris_fs::{AnyError, ErrorKind};

/// Which writer failed and why, beyond the [`ErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The ISO 9660 writer failed.
    Iso(hadris_iso::Detail),
    /// The UDF writer failed.
    Udf(hadris_udf::Detail),
    /// The ISO 9660 writer stored no data for a file that has some.
    MissingExtent,
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Iso(detail) => write!(f, "ISO 9660: {detail}"),
            Self::Udf(detail) => write!(f, "UDF: {detail}"),
            Self::MissingExtent => f.write_str("a file has no ISO 9660 extent"),
        }
    }
}

/// Error of writing a hybrid image on a device with error `E`.
///
/// It keeps the kind, the detail and the device error of the ISO 9660 or
/// UDF error it wraps. Callers match on [`kind`](Self::kind). Two errors
/// are equal when their kinds and device errors are.
///
/// `?` converts it into [`hadris_fs::Error<E>`], [`AnyError`] and, with
/// `std`, [`std::io::Error`], returning an `io::Error` device error as
/// itself.
#[derive(Debug)]
pub struct Error<E> {
    kind: ErrorKind,
    detail: Option<Detail>,
    device: Option<E>,
    content: Option<AnyError>,
}

impl<E> Error<E> {
    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            detail: Some(detail),
            device: None,
            content: None,
        }
    }

    /// What went wrong. [`ErrorKind::Io`] when the device failed.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which writer failed and why, when known.
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

    /// The error of reading a file's content from the tree.
    pub fn content_error(&self) -> Option<&AnyError> {
        self.content.as_ref()
    }

    /// Converts the device error.
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> Error<F> {
        Error {
            kind: self.kind,
            detail: self.detail,
            device: self.device.map(f),
            content: self.content,
        }
    }
}

impl<E: core::error::Error + Send + Sync + 'static> From<hadris_iso::Error<E>> for Error<E> {
    fn from(err: hadris_iso::Error<E>) -> Self {
        let kind = err.kind();
        let detail = err.detail().map(Detail::Iso);
        if err.content_error().is_some() {
            return Self {
                kind,
                detail,
                device: None,
                content: Some(err.into()),
            };
        }
        Self {
            kind,
            detail,
            device: err.into_device_error(),
            content: None,
        }
    }
}

impl<E: core::error::Error + Send + Sync + 'static> From<hadris_udf::Error<E>> for Error<E> {
    fn from(err: hadris_udf::Error<E>) -> Self {
        let kind = err.kind();
        let detail = err.detail().map(Detail::Udf);
        if err.content_error().is_some() {
            return Self {
                kind,
                detail,
                device: None,
                content: Some(err.into()),
            };
        }
        Self {
            kind,
            detail,
            device: err.into_device_error(),
            content: None,
        }
    }
}

impl<E> From<hadris_fs::tree::TreeError> for Error<E> {
    fn from(err: hadris_fs::tree::TreeError) -> Self {
        Self {
            kind: err.kind(),
            detail: None,
            device: None,
            content: None,
        }
    }
}

impl<E: PartialEq> PartialEq for Error<E> {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.device == other.device
    }
}

impl<E: Eq> Eq for Error<E> {}

impl<E> From<Error<E>> for hadris_fs::Error<E> {
    fn from(err: Error<E>) -> Self {
        match err.device {
            Some(device) => hadris_fs::Error::from_device(device),
            None => err.kind.into(),
        }
    }
}

impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for AnyError {
    fn from(err: Error<E>) -> Self {
        match err.content {
            Some(content) => content,
            None => hadris_fs::Error::from(err).into(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(device) = &self.device {
            return write!(f, "device error: {device}");
        }
        if let Some(content) = &self.content {
            return write!(f, "reading file content failed: {content}");
        }
        match self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.kind),
            None => self.kind.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        if let Some(device) = &self.device {
            return Some(device);
        }
        if let Some(content) = &self.content {
            return Some(content);
        }
        None
    }
}

/// A device error that is a `std::io::Error` comes back as itself.
#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for std::io::Error {
    fn from(err: Error<E>) -> Self {
        if err.device.is_some() {
            return hadris_fs::Error::from(err).into();
        }
        if let Some(content) = err.content {
            return content.into();
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
