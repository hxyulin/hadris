use core::fmt;

use hadris_fs::ErrorKind;

/// A filesystem of an optical image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpticalFormat {
    /// ISO 9660.
    Iso9660,
    /// Universal Disk Format.
    Udf,
}

/// Why opening an image failed, beyond the [`ErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// No ISO 9660 or UDF volume was recognized.
    UnknownFormat,
    /// The policy requires a filesystem the image does not have.
    FormatUnavailable(OpticalFormat),
    /// The driver of the selected filesystem refused the volume; the kind
    /// says why.
    Mount(OpticalFormat),
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => f.write_str("no ISO 9660 or UDF volume"),
            Self::FormatUnavailable(format) => write!(f, "the image has no {format:?} volume"),
            Self::Mount(format) => write!(f, "mounting {format:?} failed"),
        }
    }
}

/// Error of detecting or opening an image on a device with error `E`.
///
/// It keeps the kind and the device error of a driver's failure and adds
/// which filesystem it concerns. Callers match on [`kind`](Self::kind). Two
/// errors are equal when their kinds and device errors are.
///
/// `?` converts it into [`hadris_fs::Error<E>`], and with `alloc` into
/// [`hadris_fs::PathError`] and with `std` into [`std::io::Error`],
/// returning an `io::Error` device error as itself.
#[derive(Debug)]
pub struct Error<E> {
    kind: ErrorKind,
    detail: Option<Detail>,
    device: Option<E>,
}

impl<E> Error<E> {
    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            detail: Some(detail),
            device: None,
        }
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn mount(err: hadris_fs::Error<E>, format: OpticalFormat) -> Self {
        Self {
            kind: err.kind(),
            detail: Some(Detail::Mount(format)),
            device: err.into_device_error(),
        }
    }

    /// What went wrong: [`ErrorKind::Io`] when the device failed, the
    /// driver's kind when a mount failed, and [`ErrorKind::Unsupported`]
    /// when no volume, or not the one the policy requires, was found.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which filesystem or step it concerns, when known.
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

impl<E: PartialEq> PartialEq for Error<E> {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.device == other.device
    }
}

impl<E: Eq> Eq for Error<E> {}

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

impl<E> From<hadris_fs::Error<E>> for Error<E> {
    fn from(err: hadris_fs::Error<E>) -> Self {
        Self {
            kind: err.kind(),
            detail: None,
            device: err.into_device_error(),
        }
    }
}

/// Keeps the kind and the device error.
impl<E> From<Error<E>> for hadris_fs::Error<E> {
    fn from(err: Error<E>) -> Self {
        match err.device {
            Some(device) => hadris_fs::Error::device(device, "device failed"),
            None => err.kind.into(),
        }
    }
}

#[cfg(feature = "alloc")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for hadris_fs::PathError {
    fn from(err: Error<E>) -> Self {
        hadris_fs::Error::from(err).into()
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

/// Error of `OpenOpticalImage::open` and `open_detected`: an [`Error`] and
/// the device the opener was given.
///
/// `?` converts it into [`Error`] or [`hadris_fs::Error`], dropping the
/// device.
pub struct OpenError<D, E> {
    error: Error<E>,
    device: D,
}

impl<D, E> OpenError<D, E> {
    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn new(error: Error<E>, device: D) -> Self {
        Self { error, device }
    }

    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.error.kind()
    }

    /// Borrows the reason the open failed.
    pub fn error(&self) -> &Error<E> {
        &self.error
    }

    /// Borrows the device.
    pub fn device(&self) -> &D {
        &self.device
    }

    /// Returns the reason the open failed, dropping the device.
    pub fn into_error(self) -> Error<E> {
        self.error
    }

    /// Returns the device, dropping the reason.
    pub fn into_device(self) -> D {
        self.device
    }

    /// Returns the reason and the device.
    pub fn into_parts(self) -> (Error<E>, D) {
        (self.error, self.device)
    }
}

impl<D, E: fmt::Debug> fmt::Debug for OpenError<D, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl<D, E: fmt::Display> fmt::Display for OpenError<D, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl<D, E: core::error::Error + 'static> core::error::Error for OpenError<D, E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.error.source()
    }
}

impl<D, E> From<OpenError<D, E>> for Error<E> {
    fn from(err: OpenError<D, E>) -> Self {
        err.error
    }
}

impl<D, E> From<OpenError<D, E>> for hadris_fs::Error<E> {
    fn from(err: OpenError<D, E>) -> Self {
        err.error.into()
    }
}

#[cfg(feature = "alloc")]
impl<D, E: core::error::Error + Send + Sync + 'static> From<OpenError<D, E>>
    for hadris_fs::PathError
{
    fn from(err: OpenError<D, E>) -> Self {
        err.error.into()
    }
}

#[cfg(feature = "std")]
impl<D, E: core::error::Error + Send + Sync + 'static> From<OpenError<D, E>> for std::io::Error {
    fn from(err: OpenError<D, E>) -> Self {
        err.error.into()
    }
}
