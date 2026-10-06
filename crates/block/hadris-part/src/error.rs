use core::fmt;

pub(crate) use hadris_fs::Error;
use hadris_fs::{DetailCode, ErrorKind};

const DOMAIN: &str = "hadris-part";

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which structure or
/// partition to report. Read it back from an [`Error`] with [`Detail::of`],
/// or from a [`TableError`] with [`TableError::detail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// Block 0 lacks the `55 AA` boot signature, so the disk has no
    /// partition table.
    NoTable = 1,
    /// An MBR or EBR entry is invalid: a boot indicator other than `0x00`
    /// or `0x80`, or a second extended partition.
    MbrEntry = 2,
    /// The chain of extended boot records loops, leaves the extended
    /// partition, or is too long.
    EbrChain = 3,
    /// A GPT header field is invalid: signature, revision, size, location or
    /// usable range.
    GptHeader = 4,
    /// A GPT header fails its CRC.
    GptHeaderCrc = 5,
    /// The GPT entry size, count or array location is invalid.
    GptEntries = 6,
    /// A GPT entry array fails its CRC.
    GptEntriesCrc = 7,
    /// The block size is not a power of two of at least 512 bytes, or does
    /// not match the table's.
    BlockSize = 8,
    /// The partition would overlap another partition or a structure of the
    /// table itself; [`TableError::index`] and [`TableError::other`] name
    /// them.
    Overlap = 9,
    /// The partition lies outside the usable area of the disk or the
    /// extended partition; [`TableError::index`] names it.
    OutOfBounds = 10,
    /// Every slot of the table is in use.
    TableFull = 11,
    /// No partition has the index asked for; [`TableError::index`] names
    /// it.
    NoSuchPartition = 12,
    /// A logical partition needs an extended partition, or a second
    /// extended partition was requested.
    Extended = 13,
    /// The extended partition still holds logical partitions.
    ExtendedInUse = 14,
    /// A partition name is too long or contains a NUL.
    Name = 15,
    /// A hybrid MBR mirror is invalid.
    Mirror = 16,
    /// The disk is too small for the table or the layout.
    DiskTooSmall = 17,
    /// A value does not fit its 32-bit MBR field.
    FieldOverflow = 18,
    /// The partition kind does not belong to this table (a GUID in an MBR,
    /// a type code in a GPT), or is the unused kind.
    Kind = 19,
    /// A flag the table cannot store.
    Flags = 20,
    /// A partition size is zero, or `Size::Remaining` is not last.
    Size = 21,
    /// Boot code longer than 446 bytes.
    Bootstrap = 22,
}

impl Detail {
    const fn description(self) -> &'static str {
        match self {
            Self::NoTable => "no partition table signature",
            Self::MbrEntry => "invalid MBR partition entry",
            Self::EbrChain => "invalid extended boot record chain",
            Self::GptHeader => "invalid GPT header",
            Self::GptHeaderCrc => "GPT header CRC mismatch",
            Self::GptEntries => "invalid GPT partition entry array",
            Self::GptEntriesCrc => "GPT partition entry array CRC mismatch",
            Self::BlockSize => "unsupported or mismatched block size",
            Self::Overlap => "partitions overlap",
            Self::OutOfBounds => "partition outside the usable area",
            Self::TableFull => "partition table is full",
            Self::NoSuchPartition => "no such partition",
            Self::Extended => "logical partitions need exactly one extended partition",
            Self::ExtendedInUse => "extended partition holds logical partitions",
            Self::Name => "invalid partition name",
            Self::Mirror => "invalid hybrid MBR mirror",
            Self::DiskTooSmall => "disk too small",
            Self::FieldOverflow => "value does not fit a 32-bit MBR field",
            Self::Kind => "partition kind does not fit the table",
            Self::Flags => "flag not supported by the table",
            Self::Size => "invalid partition size",
            Self::Bootstrap => "boot code longer than 446 bytes",
        }
    }
}

impl Detail {
    const ALL: [Self; 22] = [
        Self::NoTable,
        Self::MbrEntry,
        Self::EbrChain,
        Self::GptHeader,
        Self::GptHeaderCrc,
        Self::GptEntries,
        Self::GptEntriesCrc,
        Self::BlockSize,
        Self::Overlap,
        Self::OutOfBounds,
        Self::TableFull,
        Self::NoSuchPartition,
        Self::Extended,
        Self::ExtendedInUse,
        Self::Name,
        Self::Mirror,
        Self::DiskTooSmall,
        Self::FieldOverflow,
        Self::Kind,
        Self::Flags,
        Self::Size,
        Self::Bootstrap,
    ];

    /// The detail a partition table operation recorded on `err`, if any.
    pub fn of<E>(err: &Error<E>) -> Option<Self> {
        err.detail().and_then(Self::from_code)
    }

    /// The detail `code` stands for, when it is one of this crate's codes.
    pub fn from_code(code: DetailCode) -> Option<Self> {
        let code = code.code_in(DOMAIN)?;
        Self::ALL.into_iter().find(|detail| *detail as u16 == code)
    }

    /// The code this detail is recorded with, in the `hadris-part` domain.
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
}

impl From<Detail> for DetailCode {
    fn from(detail: Detail) -> Self {
        detail.code()
    }
}

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

/// Error of a table operation that touches no device: an edit, a layout,
/// a name.
///
/// Converts with `?` into [`Error<E>`](hadris_fs::Error), keeping the kind
/// and the detail code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableError {
    kind: ErrorKind,
    detail: Detail,
    index: Option<usize>,
    other: Option<usize>,
}

impl TableError {
    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            detail,
            index: None,
            other: None,
        }
    }

    pub(crate) const fn invalid(detail: Detail) -> Self {
        Self::new(ErrorKind::InvalidInput, detail)
    }

    pub(crate) const fn at(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }

    pub(crate) const fn overlapping(mut self, other: usize) -> Self {
        self.other = Some(other);
        self
    }

    /// What went wrong.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which structure or partition it concerns.
    pub const fn detail(&self) -> Detail {
        self.detail
    }

    /// The index of the partition it concerns, when there is one.
    pub const fn index(&self) -> Option<usize> {
        self.index
    }

    /// For [`Detail::Overlap`], the index of the partition overlapped; the
    /// same as [`index`](Self::index) when the partition overlaps a
    /// structure of the table itself.
    pub const fn other(&self) -> Option<usize> {
        self.other
    }
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.detail, self.index, self.other) {
            (Detail::Overlap, Some(index), Some(other)) => {
                write!(f, "partition {index} would overlap partition {other}")
            }
            (Detail::OutOfBounds, Some(index), _) => {
                write!(f, "partition {index} lies outside the usable area")
            }
            (Detail::NoSuchPartition, Some(index), _) => write!(f, "no partition {index}"),
            (detail, _, _) => detail.fmt(f),
        }
    }
}

impl core::error::Error for TableError {}

impl<E> From<TableError> for Error<E> {
    fn from(err: TableError) -> Self {
        err.detail.error(err.kind)
    }
}

#[cfg(feature = "std")]
impl From<TableError> for std::io::Error {
    fn from(err: TableError) -> Self {
        std::io::Error::new(err.kind.into(), err)
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
