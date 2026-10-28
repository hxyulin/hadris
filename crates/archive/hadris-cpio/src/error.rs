use core::fmt;

pub(crate) use hadris_fs::Error;
use hadris_fs::{DetailCode, ErrorKind};
use hadris_io::ExactError;

#[cfg(feature = "write")]
use hadris_fs::PathError;

const DOMAIN: &str = "hadris-cpio";

/// A failed archive finish, retaining its output for inspection or recovery.
///
/// Recover the output with [`into_output`](Self::into_output), or both values
/// with [`into_parts`](Self::into_parts). A failed trailer write may already
/// have emitted part of the trailer. The output is not a retryable writer;
/// appending another trailer can produce an invalid archive.
///
/// `Display` describes the failed finish. The original [`PathError`], including
/// its path and device error, is available through [`error`](Self::error) and
/// [`source`](core::error::Error::source), so an error-chain printer shows each
/// message once. Conversion into [`PathError`] or, with `std`, `std::io::Error`
/// preserves that context and drops the output.
#[cfg(feature = "write")]
pub struct FinishError<W> {
    error: PathError,
    output: W,
}

#[cfg(feature = "write")]
impl<W> FinishError<W> {
    pub(crate) fn new(error: PathError, output: W) -> Self {
        Self { error, output }
    }

    /// Borrows the failure, including its path and original source.
    pub const fn error(&self) -> &PathError {
        &self.error
    }

    /// Borrows the output in the state left by the failed finish.
    pub const fn output(&self) -> &W {
        &self.output
    }

    /// Returns the output, dropping the error. Its archive may be incomplete.
    pub fn into_output(self) -> W {
        self.output
    }

    /// Returns the error, dropping the output.
    pub fn into_error(self) -> PathError {
        self.error
    }

    /// Returns the original error and the output, in that order.
    pub fn into_parts(self) -> (PathError, W) {
        (self.error, self.output)
    }
}

#[cfg(feature = "write")]
impl<W> fmt::Debug for FinishError<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FinishError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "write")]
impl<W> fmt::Display for FinishError<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("finishing the cpio archive failed")
    }
}

#[cfg(feature = "write")]
impl<W> core::error::Error for FinishError<W> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(&self.error)
    }
}

#[cfg(feature = "write")]
impl<W> From<FinishError<W>> for PathError {
    fn from(error: FinishError<W>) -> Self {
        error.into_error()
    }
}

#[cfg(all(feature = "write", feature = "std"))]
impl<W> From<FinishError<W>> for std::io::Error {
    fn from(error: FinishError<W>) -> Self {
        error.into_error().into()
    }
}

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which part of the
/// archive or which input it concerns. Read it back from an [`Error`] with
/// [`Detail::of`], or from a [`PathError`](hadris_fs::PathError) with
/// [`Detail::from_code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// A header starts with no known cpio magic.
    Magic = 1,
    /// A header field is not a valid number, or a value does not fit its
    /// field.
    Field = 2,
    /// A name is empty, too long, not NUL-terminated, or the reserved
    /// trailer name.
    Name = 3,
    /// Alignment padding is not zero.
    Padding = 4,
    /// The check field of a `070701` entry is not zero.
    Check = 5,
    /// The data of a `070702` entry does not sum to its check field.
    Checksum = 6,
    /// The trailer has data, is missing where required, or is cut off.
    Trailer = 7,
    /// The archive ends inside an entry.
    Truncated = 8,
    /// An entry cannot be written: an empty symlink target, an empty hard
    /// link group, or a node kind the format cannot store.
    Entry = 10,
    /// The format cannot be written, such as old binary cpio.
    Format = 11,
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

impl Detail {
    const ALL: [Self; 10] = [
        Self::Magic,
        Self::Field,
        Self::Name,
        Self::Padding,
        Self::Check,
        Self::Checksum,
        Self::Trailer,
        Self::Truncated,
        Self::Entry,
        Self::Format,
    ];

    /// The detail a cpio operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of this crate's codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-cpio` domain.
    /// Codes never change meaning.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    pub(crate) fn error<E>(self, kind: ErrorKind) -> Error<E> {
        Error::new(kind, self.description()).with_detail(self.code())
    }

    pub(crate) fn corrupt<E>(self) -> Error<E> {
        self.error(ErrorKind::Corrupt)
    }

    pub(crate) fn invalid<E>(self) -> Error<E> {
        self.error(ErrorKind::InvalidInput)
    }
}

impl From<Detail> for DetailCode {
    fn from(detail: Detail) -> Self {
        detail.code()
    }
}

/// A short read of the archive: truncated, or the stream failed.
pub(crate) fn read_failed<E>(err: ExactError<E>) -> Error<E> {
    match err {
        ExactError::Io(err) => Error::device(err, "reading the archive failed"),
        _ => Detail::Truncated.corrupt(),
    }
}

/// A failed write of the archive.
pub(crate) fn write_failed<E>(err: ExactError<E>) -> Error<E> {
    match err {
        ExactError::Io(err) => Error::device(err, "writing the archive failed"),
        _ => Error::new(ErrorKind::NoSpace, "the output accepted no bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_round_trip_through_their_codes() {
        for detail in Detail::ALL {
            let err: Error<()> = Error::new(ErrorKind::Corrupt, "").with_detail(detail.code());
            assert_eq!(Detail::of(&err), Some(detail));
        }
        let foreign = DetailCode::new("another-crate", Detail::ALL[0] as u16);
        assert_eq!(Detail::from_code(foreign), None);
    }

    #[test]
    fn detail_codes_are_pinned() {
        let pinned = [
            (Detail::Magic, 1),
            (Detail::Field, 2),
            (Detail::Name, 3),
            (Detail::Padding, 4),
            (Detail::Check, 5),
            (Detail::Checksum, 6),
            (Detail::Trailer, 7),
            (Detail::Truncated, 8),
            (Detail::Entry, 10),
            (Detail::Format, 11),
        ];
        assert_eq!(pinned.len(), Detail::ALL.len());
        for (detail, number) in pinned {
            let code = DetailCode::new("hadris-cpio", number);
            assert_eq!(detail.code(), code);
            assert_eq!(Detail::from_code(code), Some(detail));
        }
    }
}
