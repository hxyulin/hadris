use core::fmt;

use hadris_fs::ErrorKind;
use hadris_io::ExactError;

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which part of the
/// archive or which input it concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// A header starts with no known cpio magic.
    Magic,
    /// A header field is not a valid number, or a value does not fit its
    /// field.
    Field,
    /// A name is empty, too long, not NUL-terminated, or the reserved
    /// trailer name.
    Name,
    /// Alignment padding is not zero.
    Padding,
    /// The check field of a `070701` entry is not zero.
    Check,
    /// The data of a `070702` entry does not sum to its check field.
    Checksum,
    /// The trailer has data, is missing where required, or is cut off.
    Trailer,
    /// The archive ends inside an entry.
    Truncated,
    /// Reading the content of a file from the tree failed; see
    /// [`Error::content_error`].
    Content,
    /// An entry cannot be written: an empty symlink target, an empty hard
    /// link group, or a node kind the format cannot store.
    Entry,
    /// The format cannot be written, such as old binary cpio.
    Format,
}

impl Detail {
    const fn description(self) -> &'static str {
        match self {
            Self::Magic => "unknown cpio magic",
            Self::Field => "invalid or oversized header field",
            Self::Name => "invalid entry name",
            Self::Padding => "alignment padding is not zero",
            Self::Check => "070701 check field is not zero",
            Self::Checksum => "070702 checksum mismatch",
            Self::Trailer => "invalid or missing trailer",
            Self::Truncated => "archive ends inside an entry",
            Self::Content => "reading file content failed",
            Self::Entry => "entry cannot be written",
            Self::Format => "format cannot be written",
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// Error of reading or writing a cpio archive on a stream with error `E`.
///
/// Like [`hadris_fs::Error`], it keeps the stream's own error without
/// allocation. Callers match on [`kind`](Self::kind); [`detail`](Self::detail)
/// names the structure or input. Two errors are equal when their kinds and
/// stream errors are.
///
/// `?` converts it into [`hadris_fs::Error<E>`], and with `std` into
/// [`std::io::Error`], returning an `io::Error` stream error as itself.
#[derive(Debug)]
pub struct Error<E> {
    kind: ErrorKind,
    detail: Option<Detail>,
    device: Option<E>,
    #[cfg(feature = "alloc")]
    content: Option<hadris_fs::AnyError>,
}

impl<E> Error<E> {
    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            detail: Some(detail),
            device: None,
            #[cfg(feature = "alloc")]
            content: None,
        }
    }

    pub(crate) const fn device(err: E) -> Self {
        Self {
            kind: ErrorKind::Io,
            detail: None,
            device: Some(err),
            #[cfg(feature = "alloc")]
            content: None,
        }
    }

    pub(crate) const fn corrupt(detail: Detail) -> Self {
        Self::new(ErrorKind::Corrupt, detail)
    }

    pub(crate) const fn invalid(detail: Detail) -> Self {
        Self::new(ErrorKind::InvalidInput, detail)
    }

    /// A short read of the archive: truncated, or the stream failed.
    pub(crate) fn read(err: ExactError<E>) -> Self {
        match err {
            ExactError::Io(err) => Self::device(err),
            _ => Self::corrupt(Detail::Truncated),
        }
    }

    /// A failed write of the archive.
    pub(crate) fn write(err: ExactError<E>) -> Self {
        match err {
            ExactError::Io(err) => Self::device(err),
            _ => ErrorKind::NoSpace.into(),
        }
    }

    #[cfg(feature = "alloc")]
    pub(crate) fn content(err: hadris_fs::AnyError) -> Self {
        Self {
            kind: err.kind(),
            detail: Some(Detail::Content),
            device: None,
            content: Some(err),
        }
    }

    /// What went wrong. [`ErrorKind::Io`] when the stream failed.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which structure or input it concerns, when known.
    pub const fn detail(&self) -> Option<Detail> {
        self.detail
    }

    /// The stream error, if the stream failed.
    pub const fn device_error(&self) -> Option<&E> {
        self.device.as_ref()
    }

    /// Takes the stream error, if the stream failed.
    pub fn into_device_error(self) -> Option<E> {
        self.device
    }

    /// The error of reading a file's content from the tree, for
    /// [`Detail::Content`].
    #[cfg(feature = "alloc")]
    pub fn content_error(&self) -> Option<&hadris_fs::AnyError> {
        self.content.as_ref()
    }

    /// Converts the stream error.
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> Error<F> {
        Error {
            kind: self.kind,
            detail: self.detail,
            device: self.device.map(f),
            #[cfg(feature = "alloc")]
            content: self.content,
        }
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
            #[cfg(feature = "alloc")]
            content: None,
        }
    }
}

impl<E> From<hadris_fs::Error<E>> for Error<E> {
    fn from(err: hadris_fs::Error<E>) -> Self {
        let kind = err.kind();
        Self {
            kind,
            detail: None,
            device: err.into_device_error(),
            #[cfg(feature = "alloc")]
            content: None,
        }
    }
}

#[cfg(feature = "alloc")]
impl<E> From<hadris_fs::tree::TreeError> for Error<E> {
    fn from(err: hadris_fs::tree::TreeError) -> Self {
        err.kind().into()
    }
}

impl<E> From<Error<E>> for hadris_fs::Error<E> {
    fn from(err: Error<E>) -> Self {
        match err.device {
            Some(device) => hadris_fs::Error::from_device(device),
            None => err.kind.into(),
        }
    }
}

#[cfg(feature = "alloc")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for hadris_fs::AnyError {
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
            return write!(f, "stream error: {device}");
        }
        #[cfg(feature = "alloc")]
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
        #[cfg(feature = "alloc")]
        if let Some(content) = &self.content {
            return Some(content);
        }
        None
    }
}

/// A stream error that is a `std::io::Error` comes back as itself.
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
