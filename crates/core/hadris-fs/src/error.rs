use core::fmt;

/// The category of a filesystem error, shared by every Hadris crate.
///
/// Callers match on the kind. New failure modes add context to crate errors,
/// not new kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The underlying device failed.
    Io,
    /// The named node does not exist.
    NotFound,
    /// A node with that name already exists.
    AlreadyExists,
    /// A directory was expected.
    NotADirectory,
    /// A non-directory was expected.
    IsADirectory,
    /// The directory still has entries.
    DirectoryNotEmpty,
    /// The volume has no space left.
    NoSpace,
    /// The volume or node is read-only.
    ReadOnly,
    /// Bad options, names or arguments from the caller.
    InvalidInput,
    /// Disk data violates the specification.
    Corrupt,
    /// Valid but not implemented, or not in the filesystem's capabilities.
    Unsupported,
    /// A value does not fit the on-disk field.
    LimitExceeded,
    /// Unknown or forgotten node identifier.
    InvalidHandle,
    /// The resource is in use, for example by a second writer.
    Busy,
}

impl ErrorKind {
    const fn description(self) -> &'static str {
        match self {
            Self::Io => "I/O error",
            Self::NotFound => "not found",
            Self::AlreadyExists => "already exists",
            Self::NotADirectory => "not a directory",
            Self::IsADirectory => "is a directory",
            Self::DirectoryNotEmpty => "directory not empty",
            Self::NoSpace => "no space left on volume",
            Self::ReadOnly => "read-only",
            Self::InvalidInput => "invalid input",
            Self::Corrupt => "corrupt filesystem data",
            Self::Unsupported => "unsupported operation",
            Self::LimitExceeded => "value exceeds an on-disk limit",
            Self::InvalidHandle => "invalid node handle",
            Self::Busy => "resource busy",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl core::error::Error for ErrorKind {}
