use core::fmt;

use hadris_fs::ErrorKind;

/// What exactly went wrong, beyond the [`ErrorKind`].
///
/// Callers match on the kind; the detail tells a tool which structure or
/// partition to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Detail {
    /// Block 0 lacks the `55 AA` boot signature, so the disk has no
    /// partition table.
    NoTable,
    /// An MBR or EBR entry is invalid: a boot indicator other than `0x00`
    /// or `0x80`, or a second extended partition.
    MbrEntry,
    /// The chain of extended boot records loops, leaves the extended
    /// partition, or is too long.
    EbrChain,
    /// A GPT header field is invalid: signature, revision, size, location or
    /// usable range.
    GptHeader,
    /// A GPT header fails its CRC.
    GptHeaderCrc,
    /// The GPT entry size, count or array location is invalid.
    GptEntries,
    /// A GPT entry array fails its CRC.
    GptEntriesCrc,
    /// The block size is not a power of two of at least 512 bytes, or does
    /// not match the table's.
    BlockSize,
    /// The partition would overlap partition `other` (or, for `other ==
    /// index`, a structure of the table itself).
    Overlap {
        /// Index of the partition being placed.
        index: usize,
        /// Index of the partition it would overlap.
        other: usize,
    },
    /// The partition lies outside the usable area of the disk or the
    /// extended partition.
    OutOfBounds {
        /// Index of the partition.
        index: usize,
    },
    /// Every slot of the table is in use.
    TableFull,
    /// No partition has this index.
    NoSuchPartition {
        /// The index asked for.
        index: usize,
    },
    /// A logical partition needs an extended partition, or a second
    /// extended partition was requested.
    Extended,
    /// The extended partition still holds logical partitions.
    ExtendedInUse,
    /// A partition name is too long or contains a NUL.
    Name,
    /// A hybrid MBR mirror is invalid.
    Mirror,
    /// The disk is too small for the table or the layout.
    DiskTooSmall,
    /// A value does not fit its 32-bit MBR field.
    FieldOverflow,
    /// The partition kind does not belong to this table (a GUID in an MBR,
    /// a type code in a GPT), or is the unused kind.
    Kind,
    /// A flag the table cannot store.
    Flags,
    /// A partition size is zero, or `Size::Remaining` is not last.
    Size,
    /// Boot code longer than 446 bytes.
    Bootstrap,
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
            Self::Overlap { .. } => "partitions overlap",
            Self::OutOfBounds { .. } => "partition outside the usable area",
            Self::TableFull => "partition table is full",
            Self::NoSuchPartition { .. } => "no such partition",
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

impl fmt::Display for Detail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Overlap { index, other } => {
                write!(f, "partition {index} would overlap partition {other}")
            }
            Self::OutOfBounds { index } => {
                write!(f, "partition {index} lies outside the usable area")
            }
            Self::NoSuchPartition { index } => write!(f, "no partition {index}"),
            other => f.write_str(other.description()),
        }
    }
}

/// Error of a table operation that touches no device: an edit, a layout,
/// a name.
///
/// Converts with `?` into [`Error<E>`] and [`hadris_fs::Error<E>`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableError {
    kind: ErrorKind,
    detail: Detail,
}

impl TableError {
    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self { kind, detail }
    }

    pub(crate) const fn invalid(detail: Detail) -> Self {
        Self::new(ErrorKind::InvalidInput, detail)
    }

    /// What went wrong.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which structure or partition it concerns.
    pub const fn detail(&self) -> Detail {
        self.detail
    }
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.detail.fmt(f)
    }
}

impl core::error::Error for TableError {}

impl<E> From<TableError> for hadris_fs::Error<E> {
    fn from(err: TableError) -> Self {
        err.kind.into()
    }
}

#[cfg(feature = "std")]
impl From<TableError> for std::io::Error {
    fn from(err: TableError) -> Self {
        std::io::Error::new(err.kind.into(), err)
    }
}

/// Error of reading, writing or opening a partition table on a device with
/// error `E`.
///
/// Like [`hadris_fs::Error`], it keeps the device's own error without
/// allocation. Callers match on [`kind`](Self::kind): [`ErrorKind::Io`]
/// when the device failed, [`ErrorKind::Corrupt`] when the disk data breaks
/// the specification, [`ErrorKind::NotFound`] when block 0 holds no table.
/// [`detail`](Self::detail) names the structure. Two errors are equal when
/// their kinds and device errors are.
///
/// `?` converts it into [`hadris_fs::Error<E>`], and with `std` into
/// [`std::io::Error`], returning an `io::Error` device error as itself.
#[derive(Debug, Clone)]
pub struct Error<E> {
    kind: ErrorKind,
    detail: Option<Detail>,
    device: Option<E>,
}

impl<E> Error<E> {
    pub(crate) const fn device(err: E) -> Self {
        Self {
            kind: ErrorKind::Io,
            detail: None,
            device: Some(err),
        }
    }

    pub(crate) const fn new(kind: ErrorKind, detail: Detail) -> Self {
        Self {
            kind,
            detail: Some(detail),
            device: None,
        }
    }

    pub(crate) const fn corrupt(detail: Detail) -> Self {
        Self::new(ErrorKind::Corrupt, detail)
    }

    /// What went wrong. [`ErrorKind::Io`] when the device failed.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Which structure or partition it concerns, when known.
    pub const fn detail(&self) -> Option<Detail> {
        self.detail
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
            detail: self.detail,
            device: self.device.map(f),
        }
    }
}

impl<E: PartialEq> PartialEq for Error<E> {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.device == other.device
    }
}

impl<E: Eq> Eq for Error<E> {}

impl<E> From<TableError> for Error<E> {
    fn from(err: TableError) -> Self {
        Self::new(err.kind, err.detail)
    }
}

impl<E> From<hadris_storage::WriteError<E>> for Error<E> {
    fn from(err: hadris_storage::WriteError<E>) -> Self {
        match err {
            hadris_storage::WriteError::Device(err) => Self::device(err),
            _ => Self {
                kind: ErrorKind::ReadOnly,
                detail: None,
                device: None,
            },
        }
    }
}

impl<E> From<Error<E>> for hadris_fs::Error<E> {
    fn from(err: Error<E>) -> Self {
        match err.device {
            Some(device) => hadris_fs::Error::from_device(device),
            None => err.kind.into(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.device, self.detail) {
            (Some(err), _) => write!(f, "device error: {err}"),
            (None, Some(detail)) => detail.fmt(f),
            (None, None) => self.kind.fmt(f),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.device.as_ref().map(|err| err as _)
    }
}

/// A device error that is a `std::io::Error` comes back as itself.
#[cfg(feature = "std")]
impl<E: core::error::Error + Send + Sync + 'static> From<Error<E>> for std::io::Error {
    fn from(err: Error<E>) -> Self {
        match (err.device, err.detail) {
            (Some(device), _) => hadris_io::into_std_error(device),
            (None, Some(detail)) => TableError::new(err.kind, detail).into(),
            (None, None) => std::io::Error::new(err.kind.into(), err.kind),
        }
    }
}
