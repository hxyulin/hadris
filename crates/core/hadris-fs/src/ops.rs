use core::fmt;

use crate::ErrorKind;

/// How the node-level `open` opens a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpenMode {
    /// For reading.
    Read,
    /// For reading and writing. Fails with [`ErrorKind::ReadOnly`] on a
    /// read-only mount.
    Write,
}

/// What `rename` does when the target name exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RenameMode {
    /// Replace the target, as `rename(2)` does. A directory replaces only an
    /// empty directory, and a file only a file.
    #[default]
    Replace,
    /// Fail with [`ErrorKind::AlreadyExists`], as `RENAME_NOREPLACE` does.
    NoReplace,
}

/// How a path becomes a node.
///
/// A value, never a cargo feature, so two volumes in one program can
/// resolve differently.
///
/// | | `Lexical` | `Follow` | `NoFollow` |
/// |---|---|---|---|
/// | `..` | Removes the previous component of the text | Goes to the real parent | Same as `Follow` |
/// | `/a/missing/../b` | `/b` | `NotFound` | `NotFound` |
/// | Trailing `/` on a file | Ignored | `NotADirectory` | `NotADirectory` |
/// | Symlinks mid-path | `NotADirectory` | Followed, 40 at most | Followed |
/// | Symlink as the last component | Returned | Followed | Returned |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Resolve {
    /// `..` removes the previous component of the path text, and symlinks
    /// are never followed. Works on every filesystem with one `lookup` per
    /// component.
    #[default]
    Lexical,
    /// POSIX: every component must exist, `..` is the real parent, and
    /// symlinks are followed, 40 at most ([`ErrorKind::Symlink`] after,
    /// like `ELOOP`).
    Follow,
    /// POSIX, but a symlink in the last component is returned, as `lstat`
    /// and `O_NOFOLLOW` see it.
    NoFollow,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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
    /// Neither read nor write access was requested.
    NoAccess,
}

impl OpenOptionsError {
    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::RequiresWrite => "append, truncate and create require write access",
            Self::AppendWithTruncate => "append and truncate are mutually exclusive",
            Self::NoAccess => "neither read nor write access was requested",
        }
    }

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
        f.write_str(self.description())
    }
}

impl core::error::Error for OpenOptionsError {}

/// How to open a file by path.
///
/// Start from [`new`](Self::new), which opens nothing, and add access and
/// modifiers:
///
/// ```
/// use hadris_fs::OpenOptions;
///
/// let log = OpenOptions::new().write().create().append();
/// assert!(log.validate().is_ok());
/// assert!(OpenOptions::new().read().truncate().validate().is_err());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OpenOptions(OpenFlags);

impl OpenOptions {
    /// Options that request nothing yet.
    pub const fn new() -> Self {
        Self(OpenFlags::empty())
    }

    /// Requests read access.
    pub const fn read(self) -> Self {
        Self(self.0.union(OpenFlags::READ))
    }

    /// Requests write access.
    pub const fn write(self) -> Self {
        Self(self.0.union(OpenFlags::WRITE))
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

    /// Rejects contradictory combinations and options that request no
    /// access.
    pub const fn validate(&self) -> Result<(), OpenOptionsError> {
        if !self.is_read() && !self.is_write() {
            return Err(OpenOptionsError::NoAccess);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_options_valid_combinations() {
        let new = OpenOptions::new;
        for opts in [
            new().read(),
            new().write(),
            new().read().write(),
            new().write().create().append(),
            new().write().create().truncate(),
            new().write().create_new(),
            new().write().create().create_new(),
            new().read().write().append(),
        ] {
            assert_eq!(opts.validate(), Ok(()), "{opts:?}");
        }
    }

    #[test]
    fn open_options_contradictions() {
        let new = OpenOptions::new;
        assert_eq!(new().validate(), Err(OpenOptionsError::NoAccess));
        assert_eq!(
            new().read().truncate().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            new().read().append().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            new().read().create().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            new().read().create_new().validate(),
            Err(OpenOptionsError::RequiresWrite)
        );
        assert_eq!(
            new().write().append().truncate().validate(),
            Err(OpenOptionsError::AppendWithTruncate)
        );
        assert_eq!(
            OpenOptionsError::RequiresWrite.kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn open_options_getters() {
        let opts = OpenOptions::new().read().write().create();
        assert!(opts.is_read() && opts.is_write() && opts.is_create());
        assert!(!opts.is_append() && !opts.is_truncate() && !opts.is_create_new());
    }

    #[test]
    fn defaults() {
        assert_eq!(Resolve::default(), Resolve::Lexical);
        assert_eq!(RenameMode::default(), RenameMode::Replace);
        assert_eq!(DeviceNumber::new(5, 1).minor(), 1);
    }
}
