use core::fmt;

use crate::{FileTimes, FileType};

/// POSIX permission bits: the `0o7777` part of `st_mode`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Mode(u32);

impl Mode {
    /// Every bit a `Mode` can hold.
    pub const MASK: u32 = 0o7777;

    /// Creates a mode. Bits outside [`MASK`](Self::MASK) are discarded.
    pub const fn new(bits: u32) -> Self {
        Self(bits & Self::MASK)
    }

    /// Returns the permission bits.
    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mode({:#o})", self.0)
    }
}

bitflags::bitflags! {
    /// DOS-style attribute flags shared by FAT, exFAT, NTFS and ISO hidden files.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct Attributes: u32 {
        /// The node may not be written.
        const READ_ONLY = 1 << 0;
        /// The node is hidden from normal listings.
        const HIDDEN = 1 << 1;
        /// The node belongs to the operating system.
        const SYSTEM = 1 << 2;
        /// The node changed since the last backup.
        const ARCHIVE = 1 << 5;
    }
}

/// Metadata of a filesystem node.
///
/// Filesystems build it with [`new`](Self::new) and the `with_*` setters;
/// callers read it through getters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Metadata {
    file_type: FileType,
    len: u64,
    times: FileTimes,
    permissions: Option<Mode>,
    owner: Option<(u32, u32)>,
    nlink: u64,
    attributes: Attributes,
}

impl Metadata {
    /// Creates metadata for a node of `file_type` with length 0, one link,
    /// no times, no permissions, no owner and no attributes.
    pub const fn new(file_type: FileType) -> Self {
        Self {
            file_type,
            len: 0,
            times: FileTimes::new(),
            permissions: None,
            owner: None,
            nlink: 1,
            attributes: Attributes::empty(),
        }
    }

    /// Returns the node type.
    pub const fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the length in bytes.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Returns the node's timestamps.
    pub const fn times(&self) -> FileTimes {
        self.times
    }

    /// Returns the POSIX permissions, if the format stores them.
    pub const fn permissions(&self) -> Option<Mode> {
        self.permissions
    }

    /// Returns the `(uid, gid)` owner, if the format stores one.
    pub const fn owner(&self) -> Option<(u32, u32)> {
        self.owner
    }

    /// Returns the number of hard links. 1 on a directory means the format
    /// does not count them (POSIX would report 2 plus the subdirectories).
    pub const fn nlink(&self) -> u64 {
        self.nlink
    }

    /// Returns the DOS-style attributes.
    pub const fn attributes(&self) -> Attributes {
        self.attributes
    }

    /// Sets the node type.
    pub const fn with_file_type(self, file_type: FileType) -> Self {
        Self { file_type, ..self }
    }

    /// Sets the length in bytes.
    pub const fn with_len(self, len: u64) -> Self {
        Self { len, ..self }
    }

    /// Sets the timestamps.
    pub const fn with_times(self, times: FileTimes) -> Self {
        Self { times, ..self }
    }

    /// Sets the POSIX permissions.
    pub fn with_permissions(self, permissions: impl Into<Option<Mode>>) -> Self {
        Self {
            permissions: permissions.into(),
            ..self
        }
    }

    /// Sets the `(uid, gid)` owner.
    pub fn with_owner(self, owner: impl Into<Option<(u32, u32)>>) -> Self {
        Self {
            owner: owner.into(),
            ..self
        }
    }

    /// Sets the number of hard links.
    pub const fn with_nlink(self, nlink: u64) -> Self {
        Self { nlink, ..self }
    }

    /// Sets the DOS-style attributes.
    pub const fn with_attributes(self, attributes: Attributes) -> Self {
        Self { attributes, ..self }
    }
}

/// Metadata changes for `set_metadata` and for creating nodes.
///
/// Every field is optional; unset fields are left unchanged, or take the
/// filesystem's default on create. Filesystems ignore fields they cannot
/// store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SetMetadata {
    times: FileTimes,
    mode: Option<Mode>,
    uid: Option<u32>,
    gid: Option<u32>,
    attributes: Option<Attributes>,
}

impl SetMetadata {
    /// Creates an empty change set.
    pub const fn new() -> Self {
        Self {
            times: FileTimes::new(),
            mode: None,
            uid: None,
            gid: None,
            attributes: None,
        }
    }

    /// Returns the times to set. Unset times are left unchanged.
    pub const fn times(&self) -> FileTimes {
        self.times
    }

    /// Returns the permissions to set.
    pub const fn mode(&self) -> Option<Mode> {
        self.mode
    }

    /// Returns the owner user ID to set.
    pub const fn uid(&self) -> Option<u32> {
        self.uid
    }

    /// Returns the owner group ID to set.
    pub const fn gid(&self) -> Option<u32> {
        self.gid
    }

    /// Returns the attributes to set.
    pub const fn attributes(&self) -> Option<Attributes> {
        self.attributes
    }

    /// Sets the times to change.
    pub const fn with_times(self, times: FileTimes) -> Self {
        Self { times, ..self }
    }

    /// Sets the permissions.
    pub fn with_mode(self, mode: impl Into<Option<Mode>>) -> Self {
        Self {
            mode: mode.into(),
            ..self
        }
    }

    /// Sets the owner user ID.
    pub fn with_uid(self, uid: impl Into<Option<u32>>) -> Self {
        Self {
            uid: uid.into(),
            ..self
        }
    }

    /// Sets the owner group ID.
    pub fn with_gid(self, gid: impl Into<Option<u32>>) -> Self {
        Self {
            gid: gid.into(),
            ..self
        }
    }

    /// Sets the attributes.
    pub fn with_attributes(self, attributes: impl Into<Option<Attributes>>) -> Self {
        Self {
            attributes: attributes.into(),
            ..self
        }
    }

    /// Returns whether the change set changes nothing.
    pub const fn is_empty(&self) -> bool {
        self.times.is_empty()
            && self.mode.is_none()
            && self.uid.is_none()
            && self.gid.is_none()
            && self.attributes.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DateTime;

    #[test]
    fn mode_masks_file_type_bits() {
        assert_eq!(Mode::new(0o100644).bits(), 0o644);
        assert_eq!(Mode::new(0o4755).bits(), 0o4755);
    }

    #[test]
    fn metadata_builder() {
        let meta = Metadata::new(FileType::File)
            .with_len(42)
            .with_permissions(Mode::new(0o600))
            .with_owner((1000, 100))
            .with_nlink(2)
            .with_attributes(Attributes::HIDDEN | Attributes::ARCHIVE)
            .with_times(FileTimes::new().with_modified(DateTime::UNIX_EPOCH));
        assert_eq!(meta.file_type(), FileType::File);
        assert_eq!(meta.len(), 42);
        assert_eq!(meta.permissions(), Some(Mode::new(0o600)));
        assert_eq!(meta.owner(), Some((1000, 100)));
        assert_eq!(meta.nlink(), 2);
        assert!(meta.attributes().contains(Attributes::HIDDEN));
        assert_eq!(meta.times().modified(), Some(DateTime::UNIX_EPOCH));
    }

    #[test]
    fn set_metadata_defaults_to_no_change() {
        assert!(SetMetadata::default().is_empty());
        let change = SetMetadata::new().with_mode(Mode::new(0o644)).with_uid(0);
        assert!(!change.is_empty());
        assert_eq!(change.mode(), Some(Mode::new(0o644)));
        assert_eq!(change.uid(), Some(0));
        assert_eq!(change.gid(), None);
    }
}
