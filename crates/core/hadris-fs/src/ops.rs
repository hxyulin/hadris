use core::fmt;

use crate::{ErrorKind, FileType};

bitflags::bitflags! {
    /// Options for renaming a node.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct RenameFlags: u32 {
        /// Fail with [`ErrorKind::AlreadyExists`] instead of replacing the target.
        const NO_REPLACE = 1 << 0;
    }
}

/// What `remove` expects to find under the name it removes.
///
/// The driver checks the type it reads anyway, so `unlink` and `rmdir`
/// need no lookup first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RemoveKind {
    /// Anything but a directory, as `unlink`. A directory fails with
    /// [`ErrorKind::IsADirectory`].
    File,
    /// An empty directory, as `rmdir`. Anything else fails with
    /// [`ErrorKind::NotADirectory`].
    Dir,
    /// A file or an empty directory.
    Any,
}

impl RemoveKind {
    /// Checks a node of `file_type` against the expected kind.
    pub const fn check(self, file_type: FileType) -> Result<(), ErrorKind> {
        match (self, file_type.is_dir()) {
            (Self::File, true) => Err(ErrorKind::IsADirectory),
            (Self::Dir, false) => Err(ErrorKind::NotADirectory),
            _ => Ok(()),
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct OpenFlags: u8 {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const APPEND = 1 << 2;
        const TRUNCATE = 1 << 3;
        const CREATE = 1 << 4;
        const CREATE_NEW = 1 << 5;
    }
}

/// Why an [`OpenOptions`] combination is contradictory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpenOptionsError {
    /// Append, truncate or create was requested without write access.
    RequiresWrite,
    /// Append and truncate were both requested.
    AppendWithTruncate,
}

impl OpenOptionsError {
    /// Returns the matching error kind.
    pub const fn kind(self) -> ErrorKind {
        ErrorKind::InvalidInput
    }
}

impl From<OpenOptionsError> for ErrorKind {
    fn from(err: OpenOptionsError) -> Self {
        err.kind()
    }
}

impl fmt::Display for OpenOptionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RequiresWrite => "append, truncate and create require write access",
            Self::AppendWithTruncate => "append and truncate are mutually exclusive",
        })
    }
}

impl core::error::Error for OpenOptionsError {}

/// How to open a file.
///
/// Start from [`read`](Self::read), [`write`](Self::write) or
/// [`read_write`](Self::read_write) and add modifiers:
///
/// ```
/// use hadris_fs::OpenOptions;
///
/// let log = OpenOptions::write().create().append();
/// assert!(log.validate().is_ok());
/// assert!(OpenOptions::read().truncate().validate().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpenOptions(OpenFlags);

impl OpenOptions {
    /// Opens an existing file for reading.
    pub const fn read() -> Self {
        Self(OpenFlags::READ)
    }

    /// Opens an existing file for writing.
    pub const fn write() -> Self {
        Self(OpenFlags::WRITE)
    }

    /// Opens an existing file for reading and writing.
    pub const fn read_write() -> Self {
        Self(OpenFlags::READ.union(OpenFlags::WRITE))
    }

    /// Writes always go to the end of the file.
    pub const fn append(self) -> Self {
        Self(self.0.union(OpenFlags::APPEND))
    }

    /// Truncates the file to length 0 when opened.
    pub const fn truncate(self) -> Self {
        Self(self.0.union(OpenFlags::TRUNCATE))
    }

    /// Creates the file if it does not exist.
    pub const fn create(self) -> Self {
        Self(self.0.union(OpenFlags::CREATE))
    }

    /// Creates the file, failing with [`ErrorKind::AlreadyExists`] if it
    /// exists. Takes precedence over [`create`](Self::create) and
    /// [`truncate`](Self::truncate).
    pub const fn create_new(self) -> Self {
        Self(self.0.union(OpenFlags::CREATE_NEW))
    }

    /// Returns whether read access was requested.
    pub const fn is_read(&self) -> bool {
        self.0.contains(OpenFlags::READ)
    }

    /// Returns whether write access was requested.
    pub const fn is_write(&self) -> bool {
        self.0.contains(OpenFlags::WRITE)
    }

    /// Returns whether append mode was requested.
    pub const fn is_append(&self) -> bool {
        self.0.contains(OpenFlags::APPEND)
    }

    /// Returns whether truncation was requested.
    pub const fn is_truncate(&self) -> bool {
        self.0.contains(OpenFlags::TRUNCATE)
    }

    /// Returns whether creation was requested.
    pub const fn is_create(&self) -> bool {
        self.0.contains(OpenFlags::CREATE)
    }

    /// Returns whether exclusive creation was requested.
    pub const fn is_create_new(&self) -> bool {
        self.0.contains(OpenFlags::CREATE_NEW)
    }

    /// Rejects contradictory combinations.
    pub const fn validate(&self) -> Result<(), OpenOptionsError> {
        let needs_write = self.0.intersects(
            OpenFlags::APPEND
                .union(OpenFlags::TRUNCATE)
                .union(OpenFlags::CREATE)
                .union(OpenFlags::CREATE_NEW),
        );
        if needs_write && !self.is_write() {
            return Err(OpenOptionsError::RequiresWrite);
        }
        if self.is_append() && self.is_truncate() {
            return Err(OpenOptionsError::AppendWithTruncate);
        }
        Ok(())
    }
}

/// The kind of a device node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeviceKind {
    /// A character device.
    Char,
    /// A block device.
    Block,
}

/// A device's major and minor numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceNumber {
    major: u32,
    minor: u32,
}

impl DeviceNumber {
    /// Creates a device number.
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Returns the major number.
    pub const fn major(&self) -> u32 {
        self.major
    }

    /// Returns the minor number.
    pub const fn minor(&self) -> u32 {
        self.minor
    }
}

/// The kind of node to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NewNode<'a> {
    /// An empty regular file.
    File,
    /// An empty directory.
    Dir,
    /// A symbolic link to the given target. The target is a path, so it may
    /// contain `/`.
    Symlink(&'a [u8]),
    /// A device node.
    Device(DeviceKind, DeviceNumber),
}

impl NewNode<'_> {
    /// Returns the type of the node this creates.
    pub const fn file_type(&self) -> FileType {
        match self {
            Self::File => FileType::File,
            Self::Dir => FileType::Dir,
            Self::Symlink(_) => FileType::Symlink,
            Self::Device(DeviceKind::Char, _) => FileType::CharDevice,
            Self::Device(DeviceKind::Block, _) => FileType::BlockDevice,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_options_valid_combinations() {
        for opts in [
            OpenOptions::read(),
            OpenOptions::write(),
            OpenOptions::read_write(),
            OpenOptions::write().create().append(),
            OpenOptions::write().create().truncate(),
            OpenOptions::write().create_new(),
            OpenOptions::write().create().create_new(),
            OpenOptions::read_write().append(),
        ] {
            assert_eq!(opts.validate(), Ok(()), "{opts:?}");
        }
    }

    #[test]
    fn open_options_contradictions() {
        assert_eq!(
            OpenOptions::read().truncate().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            OpenOptions::read().append().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            OpenOptions::read().create().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            OpenOptions::read().create_new().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            OpenOptions::write().append().truncate().validate(),
            Err(OpenOptionsError::AppendWithTruncate)
        );
        assert_eq!(
            OpenOptionsError::RequiresWrite.kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn open_options_getters() {
        let opts = OpenOptions::read_write().create();
        assert!(opts.is_read() && opts.is_write() && opts.is_create());
        assert!(!opts.is_append() && !opts.is_truncate() && !opts.is_create_new());
    }

    #[test]
    fn remove_kind_checks_the_type() {
        assert_eq!(
            RemoveKind::File.check(FileType::Dir),
            Err(ErrorKind::IsADirectory)
        );
        assert_eq!(RemoveKind::File.check(FileType::Symlink), Ok(()));
        assert_eq!(
            RemoveKind::Dir.check(FileType::File),
            Err(ErrorKind::NotADirectory)
        );
        assert_eq!(RemoveKind::Any.check(FileType::Dir), Ok(()));
    }

    #[test]
    fn new_node_file_type() {
        let dev = NewNode::Device(DeviceKind::Char, DeviceNumber::new(5, 1));
        assert_eq!(dev.file_type(), FileType::CharDevice);
        assert_eq!(NewNode::Symlink(b"../a").file_type(), FileType::Symlink);
        assert_eq!(DeviceNumber::new(5, 1).minor(), 1);
    }
}
