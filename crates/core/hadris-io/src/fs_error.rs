use core::fmt;

use crate::{ErrorKind, ExactError};

/// Error of every device and filesystem operation.
///
/// `E` is the device's own error, kept without allocation: a kernel gets its
/// driver's error back through [`device_error`](Self::device_error), and
/// failures of the filesystem itself have none. Callers match on
/// [`kind`](Self::kind). The rest of the context is `Copy` and static: a
/// message, an optional [`Location`] and an optional [`DetailCode`] that a
/// format crate's `Detail::of` reads back.
///
/// `Display` shows the message and the location. A device error is the
/// [`source`](core::error::Error::source), never part of the message, so a
/// chain printer shows each text once.
///
/// Two errors are equal when their kinds and device errors are. The message,
/// location and detail never take part in the comparison, so tests that
/// compare errors keep passing when a driver reports more context.
///
/// ```rust
/// use hadris_io::{Error, ErrorKind, Location};
///
/// #[derive(Debug, Clone, Copy, PartialEq)]
/// enum AtaError { Timeout }
///
/// let err = Error::device(AtaError::Timeout, "reading a sector failed")
///     .with_location(Location::Block(7));
/// assert_eq!(err.kind(), ErrorKind::Io);
/// assert_eq!(err.device_error(), Some(&AtaError::Timeout));
/// assert_eq!(err.location(), Some(Location::Block(7)));
///
/// let err: Error<AtaError> = ErrorKind::NotFound.into();
/// assert_eq!((err.kind(), err.device_error()), (ErrorKind::NotFound, None));
/// ```
#[derive(Clone, Copy)]
pub struct Error<E> {
    message: &'static str,
    domain: &'static str,
    at: u64,
    code: u16,
    kind: ErrorKind,
    place: Place,
    device: Option<E>,
}

/// The kind of [`Location`] an error holds, kept apart from its value so
/// the context packs into as few words as possible.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    None,
    Byte,
    Block,
    Cluster,
    NameByte,
}

/// Result of an operation on a device with error `E`.
pub type FsResult<T, E> = Result<T, Error<E>>;

impl<E> Error<E> {
    /// An error of `kind` with no device error.
    pub const fn new(kind: ErrorKind, message: &'static str) -> Self {
        Self {
            message,
            domain: "",
            at: 0,
            code: 0,
            kind,
            place: Place::None,
            device: None,
        }
    }

    /// A device failure, with kind [`ErrorKind::Io`].
    pub const fn device(error: E, message: &'static str) -> Self {
        Self {
            message,
            domain: "",
            at: 0,
            code: 0,
            kind: ErrorKind::Io,
            place: Place::None,
            device: Some(error),
        }
    }

    /// Records where the failure happened.
    #[must_use]
    pub fn with_location(mut self, location: Location) -> Self {
        (self.place, self.at) = match location {
            Location::Byte(at) => (Place::Byte, at),
            Location::Block(at) => (Place::Block, at),
            Location::Cluster(at) => (Place::Cluster, at),
            Location::NameByte(at) => (Place::NameByte, u64::from(at)),
        };
        self
    }

    /// Records a format crate's detail code.
    #[must_use]
    pub fn with_detail(mut self, detail: DetailCode) -> Self {
        self.domain = detail.domain;
        self.code = detail.code;
        self
    }

    /// What went wrong. [`ErrorKind::Io`] when the device failed.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// A short static description of the failure.
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// Where the failure happened, when known.
    pub const fn location(&self) -> Option<Location> {
        match self.place {
            Place::None => None,
            Place::Byte => Some(Location::Byte(self.at)),
            Place::Block => Some(Location::Block(self.at)),
            Place::Cluster => Some(Location::Cluster(self.at)),
            Place::NameByte => Some(Location::NameByte(self.at as u32)),
        }
    }

    /// The format crate's detail code, when one was recorded.
    pub const fn detail(&self) -> Option<DetailCode> {
        if self.domain.is_empty() {
            None
        } else {
            Some(DetailCode::new(self.domain, self.code))
        }
    }

    /// The device error, if the device failed.
    pub const fn device_error(&self) -> Option<&E> {
        self.device.as_ref()
    }

    /// Takes the device error, if the device failed.
    pub fn into_device_error(self) -> Option<E> {
        self.device
    }

    /// Converts the device error, keeping the rest of the context.
    pub fn map_device<F>(self, f: impl FnOnce(E) -> F) -> Error<F> {
        Error {
            message: self.message,
            domain: self.domain,
            at: self.at,
            code: self.code,
            kind: self.kind,
            place: self.place,
            device: self.device.map(f),
        }
    }

    /// The same error without its device error.
    pub fn without_device<F>(&self) -> Error<F> {
        Error {
            message: self.message,
            domain: self.domain,
            at: self.at,
            code: self.code,
            kind: self.kind,
            place: self.place,
            device: None,
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
        Self::new(kind, kind.description())
    }
}

/// `UnexpectedEof` becomes [`ErrorKind::InvalidInput`], `WriteZero`
/// [`ErrorKind::NoSpace`], and a stream error a device error.
impl<E> From<ExactError<E>> for Error<E> {
    fn from(err: ExactError<E>) -> Self {
        match err {
            ExactError::Io(err) => Self::device(err, "stream failed"),
            ExactError::WriteZero => Self::new(ErrorKind::NoSpace, "output accepted no bytes"),
            _ => Self::new(ErrorKind::InvalidInput, "unexpected end of input"),
        }
    }
}

/// For streams whose own error is already an [`Error`].
impl<E> From<ExactError<Error<E>>> for Error<E> {
    fn from(err: ExactError<Error<E>>) -> Self {
        match err {
            ExactError::Io(err) => err,
            ExactError::WriteZero => Self::new(ErrorKind::NoSpace, "output accepted no bytes"),
            _ => Self::new(ErrorKind::InvalidInput, "unexpected end of input"),
        }
    }
}

impl<E> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            self.kind.fmt(f)?;
        } else {
            f.write_str(self.message)?;
        }
        match self.location() {
            Some(location) => write!(f, " at {location}"),
            None => Ok(()),
        }
    }
}

impl<E: fmt::Debug> fmt::Debug for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("location", &self.location())
            .field("detail", &self.detail())
            .field("device", &self.device)
            .finish()
    }
}

impl<E: core::error::Error + 'static> core::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.device.as_ref().map(|err| err as _)
    }
}

/// A device error that is a `std::io::Error` comes back as itself, so
/// `raw_os_error()` survives. Any other device error becomes the source of an
/// error with kind `Other`. Without a device error the kind maps across and
/// the payload is the error without its device, an `Error<Infallible>`.
#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for std::io::Error {
    fn from(err: Error<E>) -> Self {
        let context = err.without_device::<core::convert::Infallible>();
        match err.device {
            Some(device) => crate::into_std_error(device),
            None => std::io::Error::new(err.kind.into(), context),
        }
    }
}

/// Where on the device or in a name an error happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Location {
    /// A byte offset on the device.
    Byte(u64),
    /// A block of the device.
    Block(u64),
    /// A cluster of the filesystem.
    Cluster(u64),
    /// A byte offset within a name or path given by the caller.
    NameByte(u32),
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Byte(at) => write!(f, "byte {at}"),
            Self::Block(at) => write!(f, "block {at}"),
            Self::Cluster(at) => write!(f, "cluster {at}"),
            Self::NameByte(at) => write!(f, "name byte {at}"),
        }
    }
}

/// A format crate's detail code: a number within a static domain.
///
/// Each format crate names one domain and converts its `Detail` enum to and
/// from these codes, so `Detail::of(&err)` finds its own codes and ignores
/// another crate's. Codes are stable within a domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DetailCode {
    domain: &'static str,
    code: u16,
}

impl DetailCode {
    /// Code `code` of `domain`, which is the crate name by convention and
    /// must not be empty.
    pub const fn new(domain: &'static str, code: u16) -> Self {
        Self { domain, code }
    }

    /// The domain, such as `"hadris-iso"`.
    pub const fn domain(self) -> &'static str {
        self.domain
    }

    /// The number within the domain.
    pub const fn code(self) -> u16 {
        self.code
    }

    /// The number, when the code belongs to `domain`.
    pub fn code_in(self, domain: &str) -> Option<u16> {
        (self.domain == domain).then_some(self.code)
    }
}

impl fmt::Display for DetailCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.domain, self.code)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::format;

    #[derive(Debug, Clone, Copy, PartialEq)]
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
    fn context_is_copy_and_ignored_by_equality() {
        let err = Error::new(ErrorKind::Corrupt, "bad boot sector")
            .with_location(Location::Byte(510))
            .with_detail(DetailCode::new("hadris-fat", 3));
        let copy: Error<Ata> = err;
        assert_eq!(copy, err);
        assert_eq!(err, Error::from(ErrorKind::Corrupt));
        assert_eq!(err.detail().and_then(|d| d.code_in("hadris-fat")), Some(3));
        assert_eq!(err.detail().and_then(|d| d.code_in("hadris-iso")), None);
    }

    #[test]
    fn display_shows_the_message_and_source_the_device() {
        let err =
            Error::device(Ata::Timeout, "reading a block failed").with_location(Location::Block(9));
        assert_eq!(format!("{err}"), "reading a block failed at block 9");
        let source = core::error::Error::source(&err).unwrap();
        assert_eq!(format!("{source}"), "timeout");

        let err: Error<Ata> = ErrorKind::NotFound.into();
        assert_eq!(format!("{err}"), "not found");
        assert!(core::error::Error::source(&err).is_none());
        assert_eq!(
            format!("{}", Error::<Ata>::new(ErrorKind::Busy, "")),
            "resource busy"
        );
    }

    #[test]
    fn exact_errors_convert() {
        let err: Error<Ata> = ExactError::Io(Ata::Timeout).into();
        assert_eq!(err.device_error(), Some(&Ata::Timeout));
        let err: Error<Ata> = ExactError::<Ata>::WriteZero.into();
        assert_eq!(err.kind(), ErrorKind::NoSpace);
        let err: Error<Ata> = ExactError::Io(Error::device(Ata::Timeout, "x")).into();
        assert_eq!(err.kind(), ErrorKind::Io);
        let err: Error<Ata> = ExactError::<Error<Ata>>::UnexpectedEof.into();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn map_device_keeps_the_context() {
        let err = Error::device(Ata::Timeout, "m")
            .with_location(Location::Cluster(4))
            .map_device(|_| 5u8);
        assert_eq!(err.device_error(), Some(&5));
        assert_eq!(
            (err.message(), err.location()),
            ("m", Some(Location::Cluster(4)))
        );
    }

    #[test]
    fn every_kind_has_one_errno() {
        assert_eq!(ErrorKind::Unsupported.errno(), crate::Errno::EOPNOTSUPP);
        assert_eq!(ErrorKind::DirectoryNotEmpty.errno().linux(), 39);
        assert_eq!(ErrorKind::Symlink.errno().name(), "ELOOP");
        assert_eq!(ErrorKind::Corrupt.errno().linux(), 117);
    }

    #[cfg(feature = "std")]
    #[test]
    fn std_errors_come_back_unchanged() {
        let err = Error::device(std::io::Error::from_raw_os_error(30), "write failed");
        let io: std::io::Error = err.into();
        assert_eq!(io.raw_os_error(), Some(30));

        let err = Error::<Ata>::new(ErrorKind::NotFound, "no such entry")
            .with_detail(DetailCode::new("hadris-iso", 1));
        let io: std::io::Error = err.into();
        assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
        let inner = io.get_ref().unwrap();
        let back = inner
            .downcast_ref::<Error<core::convert::Infallible>>()
            .unwrap();
        assert_eq!(back.detail(), Some(DetailCode::new("hadris-iso", 1)));

        let io: std::io::Error = Error::device(Ata::Timeout, "m").into();
        assert_eq!(io.kind(), std::io::ErrorKind::Other);
        assert!(io.into_inner().unwrap().downcast::<Ata>().is_ok());
    }
}
