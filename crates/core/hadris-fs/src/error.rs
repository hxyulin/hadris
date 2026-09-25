use core::fmt;

pub use hadris_io::{DetailCode, Errno, Error, ErrorKind, FsResult, Location};

impl<E> From<crate::NameError> for Error<E> {
    fn from(err: crate::NameError) -> Self {
        Error::new(err.kind(), err.description())
    }
}

impl<E> From<crate::DateTimeError> for Error<E> {
    fn from(err: crate::DateTimeError) -> Self {
        Error::new(err.kind(), err.description())
    }
}

impl<E> From<crate::OpenOptionsError> for Error<E> {
    fn from(err: crate::OpenOptionsError) -> Self {
        Error::new(err.kind(), err.description())
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

/// An error that says which path failed, with the device error erased.
///
/// Returned by code that needs paths or mixes devices: writers, tree edits,
/// `copy_tree` and host helpers. It keeps the whole context of the
/// [`Error`] (kind, message, location and detail), the path within the tree
/// or volume, with `std` the host path, and the device or source error,
/// boxed, as its [`source`](core::error::Error::source). Every
/// [`Error<E>`] and [`MountError<D, E>`] converts with `?`.
///
/// ```rust
/// use hadris_fs::{Error, ErrorKind, PathError};
///
/// fn copy() -> Result<(), PathError> {
///     let failed: Result<(), Error<std::io::Error>> =
///         Err(Error::new(ErrorKind::NoSpace, "volume full"));
///     failed.map_err(|err| PathError::from(err).with_path("/boot/kernel"))?;
///     Ok(())
/// }
///
/// let err = copy().unwrap_err();
/// assert_eq!(err.kind(), ErrorKind::NoSpace);
/// assert_eq!(err.path(), Some(&b"/boot/kernel"[..]));
/// assert_eq!(err.to_string(), "volume full: /boot/kernel");
/// ```
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct PathError {
    context: Error<core::convert::Infallible>,
    path: Option<alloc::vec::Vec<u8>>,
    #[cfg(feature = "std")]
    host: Option<std::path::PathBuf>,
    source: Option<alloc::boxed::Box<dyn core::error::Error + Send + Sync>>,
}

#[cfg(feature = "alloc")]
impl PathError {
    /// An error of `kind` with no path and no source.
    pub fn new(kind: ErrorKind, message: &'static str) -> Self {
        Error::<core::convert::Infallible>::new(kind, message).into()
    }

    /// Records the path within the tree or volume that failed. Tree and
    /// volume paths are bytes and need not be UTF-8.
    #[must_use]
    pub fn with_path(mut self, path: impl AsRef<[u8]>) -> Self {
        self.path = Some(path.as_ref().to_vec());
        self
    }

    /// Records the host path that failed.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn with_host_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.host = Some(path.into());
        self
    }

    /// The error with `kind` instead of its kind, keeping the rest.
    #[cfg(all(feature = "std", feature = "sync"))]
    pub(crate) fn with_kind(mut self, kind: ErrorKind) -> Self {
        self.context = Error::new(kind, self.context.message());
        self
    }

    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.context.kind()
    }

    /// A short static description of the failure.
    pub fn message(&self) -> &'static str {
        self.context.message()
    }

    /// Where on the device the failure happened, when known.
    pub fn location(&self) -> Option<Location> {
        self.context.location()
    }

    /// The format crate's detail code, when one was recorded.
    pub fn detail(&self) -> Option<DetailCode> {
        self.context.detail()
    }

    /// The error without its path and source.
    pub fn context(&self) -> &Error<core::convert::Infallible> {
        &self.context
    }

    /// The path within the tree or volume, when known.
    pub fn path(&self) -> Option<&[u8]> {
        self.path.as_deref()
    }

    /// The host path, when known.
    #[cfg(feature = "std")]
    pub fn host_path(&self) -> Option<&std::path::Path> {
        self.host.as_deref()
    }

    /// The device or source error, if it has type `E`.
    pub fn downcast_device<E: core::error::Error + 'static>(&self) -> Option<&E> {
        self.source.as_deref()?.downcast_ref()
    }
}

#[cfg(feature = "alloc")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for PathError {
    fn from(err: Error<E>) -> Self {
        Self {
            context: err.without_device(),
            path: None,
            #[cfg(feature = "std")]
            host: None,
            source: err
                .into_device_error()
                .map(|err| alloc::boxed::Box::new(err) as _),
        }
    }
}

#[cfg(feature = "alloc")]
impl<D, E: core::error::Error + Send + Sync + 'static> From<MountError<D, E>> for PathError {
    fn from(err: MountError<D, E>) -> Self {
        err.error.into()
    }
}

#[cfg(feature = "alloc")]
impl From<ErrorKind> for PathError {
    fn from(kind: ErrorKind) -> Self {
        Error::<core::convert::Infallible>::from(kind).into()
    }
}

/// The host path is shown when there is no tree path.
#[cfg(feature = "alloc")]
impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.context.fmt(f)?;
        if let Some(path) = &self.path {
            return write!(f, ": {}", crate::Name::new(path));
        }
        #[cfg(feature = "std")]
        if let Some(host) = &self.host {
            return write!(f, ": {}", host.display());
        }
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl core::error::Error for PathError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.source.as_deref().map(|err| err as _)
    }
}

/// A source error that is an `std::io::Error` comes back as itself;
/// otherwise the `PathError` is the payload.
#[cfg(feature = "std")]
impl From<PathError> for std::io::Error {
    fn from(mut err: PathError) -> Self {
        match err
            .source
            .take()
            .map(|source| source.downcast::<std::io::Error>())
        {
            Some(Ok(io)) => *io,
            Some(Err(other)) => {
                err.source = Some(other);
                std::io::Error::new(err.kind().into(), err)
            }
            None => std::io::Error::new(err.kind().into(), err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn value_errors_convert_to_their_kind() {
        let err: Error<Ata> = crate::DateTimeError::OutOfRange.into();
        assert_eq!(err.kind(), ErrorKind::LimitExceeded);
        let err: Error<Ata> = crate::OpenOptionsError::RequiresWrite.into();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn mount_errors_give_the_device_back() {
        let err = MountError::new(Error::device(Ata::Timeout, "read failed"), [7u8; 3]);
        assert_eq!(err.kind(), ErrorKind::Io);
        assert_eq!(err.device(), &[7u8; 3]);
        let plain: Error<Ata> = MountError::new(Error::device(Ata::Timeout, ""), ()).into();
        assert_eq!(plain.into_device_error(), Some(Ata::Timeout));
        let (error, device) = err.into_parts();
        assert_eq!((error.kind(), device), (ErrorKind::Io, [7u8; 3]));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn path_errors_mix_devices() {
        fn copy() -> Result<(), PathError> {
            let ok: FsResult<(), Ata> = Ok(());
            ok?;
            let failed: FsResult<(), core::convert::Infallible> = Err(ErrorKind::NoSpace.into());
            failed?;
            Ok(())
        }
        assert_eq!(copy().unwrap_err().kind(), ErrorKind::NoSpace);
        let err = PathError::from(
            Error::device(Ata::Timeout, "read failed").with_location(Location::Block(3)),
        )
        .with_path("/a/b");
        assert_eq!(err.downcast_device::<Ata>(), Some(&Ata::Timeout));
        assert_eq!(err.location(), Some(Location::Block(3)));
        assert_eq!(alloc::format!("{err}"), "read failed at block 3: /a/b");
        let source = core::error::Error::source(&err).unwrap();
        assert_eq!(alloc::format!("{source}"), "timeout");
    }

    #[cfg(feature = "std")]
    #[test]
    fn path_errors_give_std_errors_back() {
        let err = PathError::from(Error::device(std::io::Error::from_raw_os_error(30), "m"));
        let io: std::io::Error = err.into();
        assert_eq!(io.raw_os_error(), Some(30));

        let err = PathError::from(ErrorKind::ReadOnly).with_host_path("/tmp/x");
        assert_eq!(err.host_path(), Some(std::path::Path::new("/tmp/x")));
        assert_eq!(alloc::format!("{err}"), "read-only: /tmp/x");
        let io: std::io::Error = err.into();
        assert_eq!(io.kind(), std::io::ErrorKind::ReadOnlyFilesystem);
        let inner = io.get_ref().unwrap().downcast_ref::<PathError>().unwrap();
        assert_eq!(inner.host_path(), Some(std::path::Path::new("/tmp/x")));

        let err = PathError::from(Error::device(Ata::Timeout, "m")).with_path("/f");
        let io: std::io::Error = err.into();
        assert_eq!(io.kind(), std::io::ErrorKind::Other);
        let inner = io.get_ref().unwrap().downcast_ref::<PathError>().unwrap();
        assert_eq!(inner.downcast_device::<Ata>(), Some(&Ata::Timeout));
    }
}
