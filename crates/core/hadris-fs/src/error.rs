use core::fmt;

/// The category of a filesystem error, shared by every Hadris crate.
///
/// Callers match on the kind. New failure modes add context to crate errors,
/// not new kinds. Each kind maps to one errno, so a VFS or FUSE layer can
/// translate it without looking at the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The underlying device failed.
    Io,
    /// The named node does not exist.
    NotFound,
    /// A node with that name already exists.
    AlreadyExists,
    /// A directory was expected.
    NotADirectory,
    /// A non-directory was expected.
    IsADirectory,
    /// The directory still has entries.
    DirectoryNotEmpty,
    /// The volume has no space left.
    NoSpace,
    /// The volume or node is read-only.
    ReadOnly,
    /// Bad options, names or arguments from the caller.
    InvalidInput,
    /// Disk data violates the specification.
    Corrupt,
    /// Valid but not implemented, or not in the filesystem's capabilities.
    Unsupported,
    /// A value does not fit an on-disk field or a caller's buffer, a path is
    /// too long, or a table (such as a node table) is full.
    LimitExceeded,
    /// Too many symbolic links, or a symbolic link where a file or directory
    /// was needed (`ELOOP`).
    Symlink,
    /// Unknown or forgotten node identifier.
    InvalidHandle,
    /// The resource is in use, for example an open node that would be
    /// removed.
    Busy,
    /// A name is longer than the filesystem or the buffer accepts
    /// (`ENAMETOOLONG`).
    NameTooLong,
    /// A file would grow past the format's size limit (`EFBIG`).
    FileTooLarge,
}

impl ErrorKind {
    const fn description(self) -> &'static str {
        match self {
            Self::Io => "I/O error",
            Self::NotFound => "not found",
            Self::AlreadyExists => "already exists",
            Self::NotADirectory => "not a directory",
            Self::IsADirectory => "is a directory",
            Self::DirectoryNotEmpty => "directory not empty",
            Self::NoSpace => "no space left on volume",
            Self::ReadOnly => "read-only",
            Self::InvalidInput => "invalid input",
            Self::Corrupt => "corrupt filesystem data",
            Self::Unsupported => "unsupported operation",
            Self::LimitExceeded => "value exceeds a limit",
            Self::Symlink => "too many symbolic links, or a symbolic link where none is allowed",
            Self::InvalidHandle => "invalid node handle",
            Self::Busy => "resource busy",
            Self::NameTooLong => "name too long",
            Self::FileTooLarge => "file too large",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl core::error::Error for ErrorKind {}

#[cfg(feature = "std")]
impl From<ErrorKind> for std::io::ErrorKind {
    fn from(kind: ErrorKind) -> Self {
        use std::io::ErrorKind as Io;
        match kind {
            ErrorKind::NotFound => Io::NotFound,
            ErrorKind::AlreadyExists => Io::AlreadyExists,
            ErrorKind::NotADirectory => Io::NotADirectory,
            ErrorKind::IsADirectory => Io::IsADirectory,
            ErrorKind::DirectoryNotEmpty => Io::DirectoryNotEmpty,
            ErrorKind::NoSpace => Io::StorageFull,
            ErrorKind::ReadOnly => Io::ReadOnlyFilesystem,
            ErrorKind::InvalidInput | ErrorKind::InvalidHandle => Io::InvalidInput,
            ErrorKind::Corrupt => Io::InvalidData,
            ErrorKind::Unsupported => Io::Unsupported,
            ErrorKind::Busy => Io::ResourceBusy,
            ErrorKind::NameTooLong => Io::InvalidFilename,
            ErrorKind::FileTooLarge => Io::FileTooLarge,
            ErrorKind::Io | ErrorKind::LimitExceeded | ErrorKind::Symlink => Io::Other,
        }
    }
}

/// Error of every filesystem operation.
///
/// `E` is the device's own error, kept without allocation: a kernel gets its
/// driver's error back through [`device_error`](Self::device_error), and
/// failures of the filesystem itself have none. Callers match on
/// [`kind`](Self::kind).
///
/// Two errors are equal when their kinds and device errors are. Context a
/// crate adds to an error later (a sector, a cluster, a field name) will
/// never take part in the comparison, so tests that compare errors keep
/// passing when a driver reports more detail.
///
/// ```rust
/// use hadris_fs::{Error, ErrorKind};
///
/// #[derive(Debug, PartialEq)]
/// enum AtaError { Timeout { lba: u64 } }
///
/// let err = Error::from_device(AtaError::Timeout { lba: 7 });
/// assert_eq!(err.kind(), ErrorKind::Io);
/// assert_eq!(err.device_error(), Some(&AtaError::Timeout { lba: 7 }));
///
/// let err: Error<AtaError> = ErrorKind::NotFound.into();
/// assert_eq!((err.kind(), err.device_error()), (ErrorKind::NotFound, None));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error<E> {
    kind: ErrorKind,
    device: Option<E>,
}

/// Result of a filesystem operation on a device with error `E`.
pub type FsResult<T, E> = Result<T, Error<E>>;

impl<E> Error<E> {
    /// A device failure, with kind [`ErrorKind::Io`].
    pub const fn from_device(err: E) -> Self {
        Self {
            kind: ErrorKind::Io,
            device: Some(err),
        }
    }

    /// What went wrong. [`ErrorKind::Io`] when the device failed.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
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
            device: self.device.map(f),
        }
    }
}

impl<E> From<ErrorKind> for Error<E> {
    fn from(kind: ErrorKind) -> Self {
        Self { kind, device: None }
    }
}

impl<E> From<crate::NameError> for Error<E> {
    fn from(err: crate::NameError) -> Self {
        err.kind().into()
    }
}

impl<E> From<hadris_storage::WriteError<E>> for Error<E> {
    fn from(err: hadris_storage::WriteError<E>) -> Self {
        match err {
            hadris_storage::WriteError::Device(err) => Self::from_device(err),
            _ => ErrorKind::ReadOnly.into(),
        }
    }
}

impl<E> From<hadris_io::ExactError<Error<E>>> for Error<E> {
    fn from(err: hadris_io::ExactError<Error<E>>) -> Self {
        match err {
            hadris_io::ExactError::Io(err) => err,
            hadris_io::ExactError::WriteZero => ErrorKind::NoSpace.into(),
            _ => ErrorKind::InvalidInput.into(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.device {
            Some(err) => write!(f, "device error: {err}"),
            None => self.kind.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.device.as_ref().map(|err| err as _)
    }
}

/// A device error that is a `std::io::Error` comes back as itself, so
/// `raw_os_error()` survives. Any other device error becomes the source.
#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for std::io::Error {
    fn from(err: Error<E>) -> Self {
        match err.device {
            Some(err) => hadris_io::into_std_error(err),
            None => std::io::Error::new(err.kind.into(), err.kind),
        }
    }
}

/// Error of mounting or formatting a filesystem that takes its device by
/// value: the [`Error`] and the device, given back instead of dropped.
///
/// `?` converts it into [`Error`], dropping the device.
///
/// ```rust
/// use hadris_fs::{ErrorKind, MountError};
///
/// let err = MountError::<_, ()>::new(ErrorKind::Corrupt.into(), [0u8; 4]);
/// assert_eq!(err.kind(), ErrorKind::Corrupt);
/// let (error, device) = err.into_parts();
/// assert_eq!((error.kind(), device), (ErrorKind::Corrupt, [0u8; 4]));
/// ```
pub struct MountError<D, E> {
    error: Error<E>,
    device: D,
}

impl<D, E> MountError<D, E> {
    /// Pairs the reason the mount failed with the device.
    pub const fn new(error: Error<E>, device: D) -> Self {
        Self { error, device }
    }

    /// What went wrong.
    pub const fn kind(&self) -> ErrorKind {
        self.error.kind()
    }

    /// Borrows the reason the mount failed.
    pub const fn error(&self) -> &Error<E> {
        &self.error
    }

    /// Borrows the device.
    pub const fn device(&self) -> &D {
        &self.device
    }

    /// Returns the reason the mount failed, dropping the device.
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

impl<D, E: fmt::Debug> fmt::Debug for MountError<D, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MountError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl<D, E: fmt::Display> fmt::Display for MountError<D, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl<D, E: core::error::Error + 'static> core::error::Error for MountError<D, E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.error.source()
    }
}

impl<D, E> From<MountError<D, E>> for Error<E> {
    fn from(err: MountError<D, E>) -> Self {
        err.error
    }
}

#[cfg(feature = "std")]
impl<D, E: core::error::Error + Send + Sync + 'static> From<MountError<D, E>> for std::io::Error {
    fn from(err: MountError<D, E>) -> Self {
        err.error.into()
    }
}

/// An error with the device type erased, for code that mixes volumes on
/// different devices.
///
/// Keeps the kind and the boxed device error. Every [`Error<E>`] converts
/// with `?`.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct AnyError {
    kind: ErrorKind,
    device: Option<alloc::boxed::Box<dyn core::error::Error + Send + Sync>>,
}

#[cfg(feature = "alloc")]
impl AnyError {
    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The device error, if the device failed and its type is `E`.
    pub fn downcast_device<E: core::error::Error + 'static>(&self) -> Option<&E> {
        self.device.as_deref()?.downcast_ref()
    }
}

#[cfg(feature = "alloc")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for AnyError {
    fn from(err: Error<E>) -> Self {
        Self {
            kind: err.kind,
            device: err.device.map(|err| alloc::boxed::Box::new(err) as _),
        }
    }
}

#[cfg(feature = "alloc")]
impl<D, E: core::error::Error + Send + Sync + 'static> From<MountError<D, E>> for AnyError {
    fn from(err: MountError<D, E>) -> Self {
        err.error.into()
    }
}

#[cfg(feature = "alloc")]
impl From<ErrorKind> for AnyError {
    fn from(kind: ErrorKind) -> Self {
        Self { kind, device: None }
    }
}

#[cfg(feature = "alloc")]
impl fmt::Display for AnyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.device {
            Some(err) => write!(f, "device error: {err}"),
            None => self.kind.fmt(f),
        }
    }
}

#[cfg(feature = "alloc")]
impl core::error::Error for AnyError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.device.as_deref().map(|err| err as _)
    }
}

/// An `std::io::Error` device error comes back as itself.
#[cfg(feature = "std")]
impl From<AnyError> for std::io::Error {
    fn from(err: AnyError) -> Self {
        match err.device {
            Some(device) => match device.downcast::<std::io::Error>() {
                Ok(io) => *io,
                Err(other) => std::io::Error::other(other),
            },
            None => std::io::Error::new(err.kind.into(), err.kind),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hadris_storage::WriteError;

    #[derive(Debug, Clone, PartialEq)]
    enum Ata {
        Timeout,
    }

    impl fmt::Display for Ata {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("timeout")
        }
    }

    impl core::error::Error for Ata {}

    #[test]
    fn write_errors_map_to_kinds() {
        let err: Error<Ata> = WriteError::ReadOnly.into();
        assert_eq!(
            (err.kind(), err.device_error()),
            (ErrorKind::ReadOnly, None)
        );
        let err: Error<Ata> = WriteError::Device(Ata::Timeout).into();
        assert_eq!(err.kind(), ErrorKind::Io);
        assert_eq!(err.into_device_error(), Some(Ata::Timeout));
    }

    #[test]
    fn exact_errors_unwrap() {
        let err: Error<Ata> = hadris_io::ExactError::Io(Error::from_device(Ata::Timeout)).into();
        assert_eq!(err.device_error(), Some(&Ata::Timeout));
        let err: Error<Ata> = hadris_io::ExactError::<Error<Ata>>::WriteZero.into();
        assert_eq!(err.kind(), ErrorKind::NoSpace);
    }

    #[test]
    fn map_device_keeps_the_kind() {
        let err = Error::from_device(Ata::Timeout).map_device(|_| 5u8);
        assert_eq!((err.kind(), err.device_error()), (ErrorKind::Io, Some(&5)));
    }

    #[test]
    fn mount_errors_give_the_device_back() {
        let err = MountError::new(Error::from_device(Ata::Timeout), [7u8; 3]);
        assert_eq!(err.kind(), ErrorKind::Io);
        assert_eq!(err.device(), &[7u8; 3]);
        let plain: Error<Ata> = MountError::new(Error::from_device(Ata::Timeout), ()).into();
        assert_eq!(plain.into_device_error(), Some(Ata::Timeout));
        let (error, device) = err.into_parts();
        assert_eq!((error.kind(), device), (ErrorKind::Io, [7u8; 3]));
    }

    #[cfg(feature = "std")]
    #[test]
    fn std_errors_come_back_unchanged() {
        let err = Error::from_device(std::io::Error::from_raw_os_error(30));
        let io: std::io::Error = err.into();
        assert_eq!(io.raw_os_error(), Some(30));

        let io: std::io::Error = Error::<Ata>::from(ErrorKind::NotFound).into();
        assert_eq!(io.kind(), std::io::ErrorKind::NotFound);

        let io: std::io::Error = Error::from_device(Ata::Timeout).into();
        assert_eq!(io.kind(), std::io::ErrorKind::Other);
        assert!(io.into_inner().unwrap().downcast::<Ata>().is_ok());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn any_error_mixes_devices() {
        fn copy() -> Result<(), AnyError> {
            let ok: FsResult<(), Ata> = Ok(());
            ok?;
            let failed: FsResult<(), core::convert::Infallible> = Err(ErrorKind::NoSpace.into());
            failed?;
            Ok(())
        }
        assert_eq!(copy().unwrap_err().kind(), ErrorKind::NoSpace);
        let err = AnyError::from(Error::from_device(Ata::Timeout));
        assert_eq!(err.downcast_device::<Ata>(), Some(&Ata::Timeout));
    }
}
