use core::fmt;

use hadris_fs::DetailCode;
pub(crate) use hadris_fs::Error;

const DOMAIN: &str = "hadris-optical";

/// A filesystem of an optical image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpticalFormat {
    /// ISO 9660.
    Iso9660,
    /// Universal Disk Format.
    Udf,
}

/// Why opening an image failed, beyond the [`ErrorKind`](hadris_fs::ErrorKind).
///
/// An image with no ISO 9660 or UDF volume fails with
/// [`ErrorKind::NotRecognized`](hadris_fs::ErrorKind::NotRecognized) and no detail; a driver that refuses the
/// volume returns its own error, with its own detail code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The policy requires a filesystem the image does not have.
    FormatUnavailable = 1,
}

impl Detail {
    const ALL: [Self; 1] = [Self::FormatUnavailable];

    const fn description(self) -> &'static str {
        match self {
            Self::FormatUnavailable => "the image has no volume of the required filesystem",
        }
    }

    /// The detail opening an image recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of this crate's codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-optical`
    /// domain. Codes never change meaning.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn error<E>(self, kind: hadris_fs::ErrorKind) -> Error<E> {
        Error::new(kind, self.description()).with_detail(self.code())
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl From<Detail> for DetailCode {
    fn from(detail: Detail) -> Self {
        detail.code()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hadris_fs::ErrorKind;

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
