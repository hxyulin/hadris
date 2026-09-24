use core::fmt;

pub(crate) use hadris_fs::Error;
use hadris_fs::{DetailCode, ErrorKind};

const DOMAIN: &str = "hadris-ntfs";

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which structure is at
/// fault. Read it back from an [`Error`] with [`Detail::of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// The OEM identifier or the end of sector marker of the boot sector is
    /// wrong: the device holds no NTFS volume.
    BootSector = 1,
    /// The sector size, cluster size, record sizes, volume size or `$MFT`
    /// location in the boot sector is invalid.
    Geometry = 2,
    /// An MFT or index record is larger than the 4096 bytes the reader
    /// handles.
    RecordSize = 3,
    /// The device's blocks are larger than 4096 bytes.
    BlockSize = 4,
    /// An MFT record has the wrong magic or is not in use, or an index
    /// record has the wrong magic.
    Record = 5,
    /// An update sequence array does not match its record.
    UpdateSequence = 6,
    /// A file reference names a record that was reused for another file.
    StaleReference = 7,
    /// An attribute is malformed, or one a structure needs is missing.
    Attribute = 8,
    /// Mapping pairs are malformed or end before the attribute's data.
    DataRun = 9,
    /// A `$FILE_NAME` value is malformed.
    FileName = 10,
    /// An index root, index record or index entry is malformed.
    Index = 11,
    /// `$UpCase` is missing or not 65536 entries long.
    Upcase = 12,
    /// The stream is compressed.
    Compressed = 13,
    /// The stream is encrypted.
    Encrypted = 14,
    /// An `$ATTRIBUTE_LIST` is malformed or names a missing attribute, or
    /// `$MFT` has more than the 32 extents the reader keeps.
    AttributeList = 15,
    /// A structure lies outside the device.
    OutsideVolume = 16,
}

impl Detail {
    const fn description(self) -> &'static str {
        match self {
            Self::BootSector => "not an NTFS boot sector",
            Self::Geometry => "invalid volume geometry",
            Self::RecordSize => "records larger than 4096 bytes",
            Self::BlockSize => "device blocks larger than 4096 bytes",
            Self::Record => "invalid or unused record",
            Self::UpdateSequence => "update sequence mismatch",
            Self::StaleReference => "stale file reference",
            Self::Attribute => "malformed or missing attribute",
            Self::DataRun => "malformed mapping pairs",
            Self::FileName => "malformed file name",
            Self::Index => "malformed index",
            Self::Upcase => "missing or malformed $UpCase",
            Self::Compressed => "compressed stream",
            Self::Encrypted => "encrypted stream",
            Self::AttributeList => "malformed or unsupported $ATTRIBUTE_LIST",
            Self::OutsideVolume => "a structure lies outside the device",
        }
    }

    const fn kind(self) -> ErrorKind {
        match self {
            Self::RecordSize | Self::BlockSize | Self::Compressed | Self::Encrypted => {
                ErrorKind::Unsupported
            }
            Self::BootSector => ErrorKind::NotRecognized,
            _ => ErrorKind::Corrupt,
        }
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl Detail {
    const ALL: [Self; 16] = [
        Self::BootSector,
        Self::Geometry,
        Self::RecordSize,
        Self::BlockSize,
        Self::Record,
        Self::UpdateSequence,
        Self::StaleReference,
        Self::Attribute,
        Self::DataRun,
        Self::FileName,
        Self::Index,
        Self::Upcase,
        Self::Compressed,
        Self::Encrypted,
        Self::AttributeList,
        Self::OutsideVolume,
    ];

    /// The detail an NTFS operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of this crate's codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-ntfs` domain.
    /// Codes never change meaning.
    pub const fn code(self) -> DetailCode {
        DetailCode::new(DOMAIN, self as u16)
    }

    pub(crate) fn error<E>(self, kind: ErrorKind) -> Error<E> {
        Error::new(kind, self.description()).with_detail(self.code())
    }
}

impl From<Detail> for DetailCode {
    fn from(detail: Detail) -> Self {
        detail.code()
    }
}

impl<E> From<Detail> for Error<E> {
    fn from(detail: Detail) -> Self {
        detail.error(detail.kind())
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
