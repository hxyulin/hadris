use core::fmt;

pub(crate) use hadris_fs::Error;
use hadris_fs::{DetailCode, ErrorKind};
use hadris_io::ExactError;

const DOMAIN: &str = "hadris-cpio";

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
}
