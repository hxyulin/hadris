use core::fmt;

/// The category of an error, shared by every Hadris crate.
///
/// Callers match on the kind. New failure modes add a message, a location
/// or a detail code to [`Error`](crate::Error), not new kinds. Each kind maps
/// to one [`Errno`], so a VFS or FUSE layer can translate it without looking
/// at the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The device failed; [`Error::device_error`](crate::Error::device_error)
    /// returns its error.
    Io,
    /// The bytes are not this format at all.
    NotRecognized,
    /// The bytes are this format, but violate its specification.
    Corrupt,
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
    /// A read-only mount or device, or a mount that turned read-only.
    ReadOnly,
    /// Bad options, names or arguments from the caller, including a block
    /// request outside a device.
    InvalidInput,
    /// The format cannot store or do this, or it is not implemented.
    Unsupported,
    /// A value does not fit an on-disk field or a caller's buffer, a path is
    /// too deep, or a table (such as a node table) is full.
    LimitExceeded,
    /// A name is longer than the filesystem or the buffer accepts
    /// (`ENAMETOOLONG`).
    NameTooLong,
    /// A file would grow past the format's size limit (`EFBIG`).
    FileTooLarge,
    /// Too many symbolic links, or a symbolic link where a file or directory
    /// was needed (`ELOOP`).
    Symlink,
    /// Unknown or forgotten node identifier.
    InvalidHandle,
    /// The resource is in use, for example an open node that would be
    /// removed.
    Busy,
}

impl ErrorKind {
    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::Io => "I/O error",
            Self::NotRecognized => "not a recognized format",
            Self::Corrupt => "corrupt filesystem data",
            Self::NotFound => "not found",
            Self::AlreadyExists => "already exists",
            Self::NotADirectory => "not a directory",
            Self::IsADirectory => "is a directory",
            Self::DirectoryNotEmpty => "directory not empty",
            Self::NoSpace => "no space left on volume",
            Self::ReadOnly => "read-only",
            Self::InvalidInput => "invalid input",
            Self::Unsupported => "unsupported operation",
            Self::LimitExceeded => "value exceeds a limit",
            Self::NameTooLong => "name too long",
            Self::FileTooLarge => "file too large",
            Self::Symlink => "too many symbolic links, or a symbolic link where none is allowed",
            Self::InvalidHandle => "invalid node handle",
            Self::Busy => "resource busy",
        }
    }

    /// The errno this kind maps to. [`Unsupported`](Self::Unsupported) is
    /// [`Errno::EOPNOTSUPP`], never `ENOSYS`, which FUSE reads as "never call
    /// this again".
    ///
    /// ```rust
    /// use hadris_io::{Errno, ErrorKind};
    ///
    /// assert_eq!(ErrorKind::ReadOnly.errno(), Errno::EROFS);
    /// assert_eq!(ErrorKind::Unsupported.errno().linux(), 95);
    /// ```
    pub const fn errno(self) -> Errno {
        match self {
            Self::Io => Errno::EIO,
            Self::NotRecognized | Self::InvalidInput => Errno::EINVAL,
            Self::Corrupt => Errno::EUCLEAN,
            Self::NotFound => Errno::ENOENT,
            Self::AlreadyExists => Errno::EEXIST,
            Self::NotADirectory => Errno::ENOTDIR,
            Self::IsADirectory => Errno::EISDIR,
            Self::DirectoryNotEmpty => Errno::ENOTEMPTY,
            Self::NoSpace => Errno::ENOSPC,
            Self::ReadOnly => Errno::EROFS,
            Self::Unsupported => Errno::EOPNOTSUPP,
            Self::LimitExceeded => Errno::EOVERFLOW,
            Self::NameTooLong => Errno::ENAMETOOLONG,
            Self::FileTooLarge => Errno::EFBIG,
            Self::Symlink => Errno::ELOOP,
            Self::InvalidHandle => Errno::ESTALE,
            Self::Busy => Errno::EBUSY,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl core::error::Error for ErrorKind {}

#[cfg(feature = "std")]
impl From<ErrorKind> for std::io::ErrorKind {
    fn from(kind: ErrorKind) -> Self {
        use std::io::ErrorKind as Io;
        match kind {
            ErrorKind::NotFound => Io::NotFound,
            ErrorKind::AlreadyExists => Io::AlreadyExists,
            ErrorKind::NotADirectory => Io::NotADirectory,
            ErrorKind::IsADirectory => Io::IsADirectory,
            ErrorKind::DirectoryNotEmpty => Io::DirectoryNotEmpty,
            ErrorKind::NoSpace => Io::StorageFull,
            ErrorKind::ReadOnly => Io::ReadOnlyFilesystem,
            ErrorKind::InvalidInput | ErrorKind::InvalidHandle => Io::InvalidInput,
            ErrorKind::NotRecognized | ErrorKind::Corrupt => Io::InvalidData,
            ErrorKind::Unsupported => Io::Unsupported,
            ErrorKind::Busy => Io::ResourceBusy,
            ErrorKind::NameTooLong => Io::InvalidFilename,
            ErrorKind::FileTooLarge => Io::FileTooLarge,
            ErrorKind::Io | ErrorKind::LimitExceeded | ErrorKind::Symlink => Io::Other,
        }
    }
}

/// A POSIX error number, named symbolically.
///
/// [`ErrorKind::errno`] gives one per kind. The numbers differ between
/// systems, so each system has its own accessor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[allow(clippy::upper_case_acronyms)]
pub enum Errno {
    /// Input/output error.
    EIO,
    /// Invalid argument.
    EINVAL,
    /// Structure needs cleaning: Linux filesystems report corruption with it
    /// (`EFSCORRUPTED`).
    EUCLEAN,
    /// No such file or directory.
    ENOENT,
    /// File exists.
    EEXIST,
    /// Not a directory.
    ENOTDIR,
    /// Is a directory.
    EISDIR,
    /// Directory not empty.
    ENOTEMPTY,
    /// No space left on device.
    ENOSPC,
    /// Read-only file system.
    EROFS,
    /// Operation not supported.
    EOPNOTSUPP,
    /// Value too large for its data type.
    EOVERFLOW,
    /// File name too long.
    ENAMETOOLONG,
    /// File too large.
    EFBIG,
    /// Too many levels of symbolic links.
    ELOOP,
    /// Stale file handle.
    ESTALE,
    /// Device or resource busy.
    EBUSY,
}

impl Errno {
    /// The number on Linux with the generic numbering (x86, Arm, RISC-V).
    pub const fn linux(self) -> i32 {
        match self {
            Self::EIO => 5,
            Self::EINVAL => 22,
            Self::EUCLEAN => 117,
            Self::ENOENT => 2,
            Self::EEXIST => 17,
            Self::ENOTDIR => 20,
            Self::EISDIR => 21,
            Self::ENOTEMPTY => 39,
            Self::ENOSPC => 28,
            Self::EROFS => 30,
            Self::EOPNOTSUPP => 95,
            Self::EOVERFLOW => 75,
            Self::ENAMETOOLONG => 36,
            Self::EFBIG => 27,
            Self::ELOOP => 40,
            Self::ESTALE => 116,
            Self::EBUSY => 16,
        }
    }

    /// The symbolic name, such as `"EROFS"`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::EIO => "EIO",
            Self::EINVAL => "EINVAL",
            Self::EUCLEAN => "EUCLEAN",
            Self::ENOENT => "ENOENT",
            Self::EEXIST => "EEXIST",
            Self::ENOTDIR => "ENOTDIR",
            Self::EISDIR => "EISDIR",
            Self::ENOTEMPTY => "ENOTEMPTY",
            Self::ENOSPC => "ENOSPC",
            Self::EROFS => "EROFS",
            Self::EOPNOTSUPP => "EOPNOTSUPP",
            Self::EOVERFLOW => "EOVERFLOW",
            Self::ENAMETOOLONG => "ENAMETOOLONG",
            Self::EFBIG => "EFBIG",
            Self::ELOOP => "ELOOP",
            Self::ESTALE => "ESTALE",
            Self::EBUSY => "EBUSY",
        }
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
