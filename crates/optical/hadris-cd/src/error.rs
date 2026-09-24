use core::fmt;

pub(crate) use hadris_fs::Error;
use hadris_fs::{DetailCode, ErrorKind};

const DOMAIN: &str = "hadris-cd";

/// What exactly went wrong in the hybrid writer itself, beyond the
/// [`ErrorKind`].
///
/// An error of the ISO 9660 or UDF writer keeps that writer's detail code:
/// read it with [`hadris_iso::Detail::from_code`] or
/// [`hadris_udf::Detail::from_code`] on
/// [`PathError::detail`](hadris_fs::PathError::detail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The ISO 9660 writer stored no data for a file that has some.
    MissingExtent = 1,
}

impl Detail {
    const ALL: [Self; 1] = [Self::MissingExtent];

    const fn description(self) -> &'static str {
        match self {
            Self::MissingExtent => "a file has no ISO 9660 extent",
        }
    }

    /// The detail a hybrid image operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of this crate's codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-cd` domain.
    /// Codes never change meaning.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    pub(crate) fn error<E>(self, kind: ErrorKind) -> Error<E> {
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

    #[test]
    fn details_round_trip_through_their_codes() {
        for detail in Detail::ALL {
            let err: Error<()> = Error::new(ErrorKind::Corrupt, "").with_detail(detail.code());
            assert_eq!(Detail::of(&err), Some(detail));
        }
        let foreign = DetailCode::new("hadris-iso", Detail::ALL[0] as u16);
        assert_eq!(Detail::from_code(foreign), None);
    }
}
