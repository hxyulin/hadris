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

impl<E> From<crate::path::PathError> for Error<E> {
    fn from(err: crate::path::PathError) -> Self {
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

/// An error with the device type erased, for code that mixes volumes on
/// different devices.
///
/// Keeps the whole context of the [`Error`] (kind, message, location and
/// detail) and the boxed device error. Every [`Error<E>`] converts with `?`.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct AnyError {
    context: Error<core::convert::Infallible>,
    device: Option<alloc::boxed::Box<dyn core::error::Error + Send + Sync>>,
}

#[cfg(feature = "alloc")]
impl AnyError {
    /// What went wrong.
    pub fn kind(&self) -> ErrorKind {
        self.context.kind()
    }

    /// A short static description of the failure.
    pub fn message(&self) -> &'static str {
        self.context.message()
    }

    /// Where the failure happened, when known.
    pub fn location(&self) -> Option<Location> {
        self.context.location()
    }

    /// The format crate's detail code, when one was recorded.
    pub fn detail(&self) -> Option<DetailCode> {
        self.context.detail()
    }

    /// The error without its device error.
    pub fn context(&self) -> &Error<core::convert::Infallible> {
        &self.context
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
            context: err.without_device(),
            device: err
                .into_device_error()
                .map(|err| alloc::boxed::Box::new(err) as _),
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
        Self {
            context: kind.into(),
            device: None,
        }
    }
}

#[cfg(feature = "alloc")]
impl fmt::Display for AnyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.context.fmt(f)
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
            None => std::io::Error::new(err.context.kind().into(), err.context),
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
        let err: Error<Ata> = crate::path::PathError::EscapesRoot.into();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert_eq!(err.message(), "parent component escapes the virtual root");
        let err: Error<Ata> = crate::TableFull::new(3u8).into();
        assert_eq!(err.kind(), ErrorKind::LimitExceeded);
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
    fn any_error_mixes_devices() {
        fn copy() -> Result<(), AnyError> {
            let ok: FsResult<(), Ata> = Ok(());
            ok?;
            let failed: FsResult<(), core::convert::Infallible> = Err(ErrorKind::NoSpace.into());
            failed?;
            Ok(())
        }
        assert_eq!(copy().unwrap_err().kind(), ErrorKind::NoSpace);
        let err = AnyError::from(
            Error::device(Ata::Timeout, "read failed").with_location(Location::Block(3)),
        );
        assert_eq!(err.downcast_device::<Ata>(), Some(&Ata::Timeout));
        assert_eq!(err.location(), Some(Location::Block(3)));
        assert_eq!(alloc::format!("{err}"), "read failed at block 3");
    }

    #[cfg(feature = "std")]
    #[test]
    fn any_error_gives_std_errors_back() {
        let err = AnyError::from(Error::device(std::io::Error::from_raw_os_error(30), "m"));
        let io: std::io::Error = err.into();
        assert_eq!(io.raw_os_error(), Some(30));
        let io: std::io::Error = AnyError::from(ErrorKind::ReadOnly).into();
        assert_eq!(io.kind(), std::io::ErrorKind::ReadOnlyFilesystem);
    }
}
