use core::fmt;

use crate::{DateTime, DeviceNumber, FileType};

/// POSIX permission bits: the `0o7777` part of `st_mode`, with the setuid,
/// setgid and sticky bits.
///
/// Formats that store none derive them: FAT and exFAT report `0o755` for a
/// directory and `0o644` for a file, without the write bits when the node
/// is read-only.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Permissions(u16);

impl Permissions {
    /// Every bit a `Permissions` can hold.
    pub const MASK: u16 = 0o7777;

    /// Creates permissions. Bits outside [`MASK`](Self::MASK), such as the
    /// file type bits of `st_mode`, are discarded.
    pub const fn new(mode: u32) -> Self {
        Self((mode & Self::MASK as u32) as u16)
    }

    /// Returns the permission bits.
    pub const fn bits(self) -> u32 {
        self.0 as u32
    }
}

impl fmt::Debug for Permissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Permissions({:#o})", self.0)
    }
}

/// The user and group that own a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Owner {
    uid: u32,
    gid: u32,
}

impl Owner {
    /// Creates an owner.
    pub const fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }

    /// Returns the user id.
    pub const fn uid(self) -> u32 {
        self.uid
    }

    /// Returns the group id.
    pub const fn gid(self) -> u32 {
        self.gid
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

impl Attributes {
    /// No attribute set.
    pub const NONE: Self = Self::empty();

    /// Returns `self` without the flags of `other`.
    pub const fn without(self, other: Self) -> Self {
        self.difference(other)
    }
}

/// Metadata of a filesystem node, as `stat` returns it.
///
/// Filesystems build it with [`new`](Self::new) and the `with_*` setters;
/// callers read it through getters. Fields a format does not store are
/// absent: `None` for times, owner and device, 0 for `allocated` and
/// `generation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Metadata {
    file_type: FileType,
    len: u64,
    allocated: u64,
    created: Option<DateTime>,
    modified: Option<DateTime>,
    accessed: Option<DateTime>,
    changed: Option<DateTime>,
    permissions: Permissions,
    owner: Option<Owner>,
    attributes: Attributes,
    nlink: u64,
    generation: u64,
    device: Option<DeviceNumber>,
}

impl Metadata {
    /// Creates metadata for a node of `file_type` with `permissions`, length
    /// 0, one link and nothing else.
    pub const fn new(file_type: FileType, permissions: Permissions) -> Self {
        Self {
            file_type,
            len: 0,
            allocated: 0,
            created: None,
            modified: None,
            accessed: None,
            changed: None,
            permissions,
            owner: None,
            attributes: Attributes::empty(),
            nlink: 1,
            generation: 0,
            device: None,
        }
    }

    /// Returns the node type.
    pub const fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the length in bytes: 0 for a directory, the target length for
    /// a symlink.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// Returns the bytes allocated on the device (`st_blocks` times 512).
    pub const fn allocated(&self) -> u64 {
        self.allocated
    }

    /// Returns the creation (birth) time.
    pub const fn created(&self) -> Option<DateTime> {
        self.created
    }

    /// Returns the last content modification time.
    pub const fn modified(&self) -> Option<DateTime> {
        self.modified
    }

    /// Returns the last access time.
    pub const fn accessed(&self) -> Option<DateTime> {
        self.accessed
    }

    /// Returns the last metadata change time.
    pub const fn changed(&self) -> Option<DateTime> {
        self.changed
    }

    /// Returns the POSIX permissions, stored or derived.
    pub const fn permissions(&self) -> Permissions {
        self.permissions
    }

    /// Returns the owner. `None` means the format stores no owner, which is
    /// not uid 0.
    pub const fn owner(&self) -> Option<Owner> {
        self.owner
    }

    /// Returns the DOS-style attributes.
    pub const fn attributes(&self) -> Attributes {
        self.attributes
    }

    /// Returns the number of hard links. 1 on a directory means the format
    /// does not count them (POSIX would report 2 plus the subdirectories).
    pub const fn nlink(&self) -> u64 {
        self.nlink
    }

    /// Returns the generation, which tells a reused [`NodeId`](crate::NodeId)
    /// from the node that had it before. 0 when the format keeps none.
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the device number of a character or block device.
    pub const fn device(&self) -> Option<DeviceNumber> {
        self.device
    }

    /// Sets the length in bytes.
    pub const fn with_len(self, len: u64) -> Self {
        Self { len, ..self }
    }

    /// Sets the bytes allocated on the device.
    pub const fn with_allocated(self, allocated: u64) -> Self {
        Self { allocated, ..self }
    }

    /// Sets the creation time.
    pub const fn with_created(self, time: DateTime) -> Self {
        Self {
            created: Some(time),
            ..self
        }
    }

    /// Sets the modification time.
    pub const fn with_modified(self, time: DateTime) -> Self {
        Self {
            modified: Some(time),
            ..self
        }
    }

    /// Sets the access time.
    pub const fn with_accessed(self, time: DateTime) -> Self {
        Self {
            accessed: Some(time),
            ..self
        }
    }

    /// Sets the metadata change time.
    pub const fn with_changed(self, time: DateTime) -> Self {
        Self {
            changed: Some(time),
            ..self
        }
    }

    /// Sets the owner.
    pub const fn with_owner(self, owner: Owner) -> Self {
        Self {
            owner: Some(owner),
            ..self
        }
    }

    /// Sets the DOS-style attributes.
    pub const fn with_attributes(self, attributes: Attributes) -> Self {
        Self { attributes, ..self }
    }

    /// Sets the number of hard links.
    pub const fn with_nlink(self, nlink: u64) -> Self {
        Self { nlink, ..self }
    }

    /// Sets the generation.
    pub const fn with_generation(self, generation: u64) -> Self {
        Self { generation, ..self }
    }

    /// Sets the device number.
    pub const fn with_device(self, device: DeviceNumber) -> Self {
        Self {
            device: Some(device),
            ..self
        }
    }
}

/// Attribute changes for `setattr`, and the initial attributes of `create`
/// and `mkdir`.
///
/// Every field is optional; unset fields are left unchanged, or take the
/// filesystem's default on create. `create` and `mkdir` ignore fields the
/// format cannot store, as `open(2)` ignores mode bits a filesystem lacks.
/// `setattr` succeeds when the value the format would report after storing
/// it equals what was asked, after rounding to the field's resolution and
/// deriving dependent bits, and fails with
/// [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported) otherwise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SetAttr {
    accessed: Option<DateTime>,
    modified: Option<DateTime>,
    created: Option<DateTime>,
    permissions: Option<Permissions>,
    owner: Option<Owner>,
    attributes: Option<Attributes>,
}

impl SetAttr {
    /// Creates an empty change set.
    pub const fn new() -> Self {
        Self {
            accessed: None,
            modified: None,
            created: None,
            permissions: None,
            owner: None,
            attributes: None,
        }
    }

    /// Sets the access time.
    pub const fn with_accessed(self, time: DateTime) -> Self {
        Self {
            accessed: Some(time),
            ..self
        }
    }

    /// Sets the modification time.
    pub const fn with_modified(self, time: DateTime) -> Self {
        Self {
            modified: Some(time),
            ..self
        }
    }

    /// Sets the creation time.
    pub const fn with_created(self, time: DateTime) -> Self {
        Self {
            created: Some(time),
            ..self
        }
    }

    /// Sets the permissions.
    pub const fn with_permissions(self, permissions: Permissions) -> Self {
        Self {
            permissions: Some(permissions),
            ..self
        }
    }

    /// Sets the owner.
    pub const fn with_owner(self, owner: Owner) -> Self {
        Self {
            owner: Some(owner),
            ..self
        }
    }

    /// Replaces the DOS attributes; read `stat` first to change one bit.
    pub const fn with_attributes(self, attributes: Attributes) -> Self {
        Self {
            attributes: Some(attributes),
            ..self
        }
    }

    /// Returns the access time to set.
    pub const fn accessed(&self) -> Option<DateTime> {
        self.accessed
    }

    /// Returns the modification time to set.
    pub const fn modified(&self) -> Option<DateTime> {
        self.modified
    }

    /// Returns the creation time to set.
    pub const fn created(&self) -> Option<DateTime> {
        self.created
    }

    /// Returns the permissions to set.
    pub const fn permissions(&self) -> Option<Permissions> {
        self.permissions
    }

    /// Returns the owner to set.
    pub const fn owner(&self) -> Option<Owner> {
        self.owner
    }

    /// Returns the attributes to set.
    pub const fn attributes(&self) -> Option<Attributes> {
        self.attributes
    }

    /// Returns whether the change set changes nothing.
    pub const fn is_empty(&self) -> bool {
        self.accessed.is_none()
            && self.modified.is_none()
            && self.created.is_none()
            && self.permissions.is_none()
            && self.owner.is_none()
            && self.attributes.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_mask_file_type_bits() {
        assert_eq!(Permissions::new(0o100644).bits(), 0o644);
        assert_eq!(Permissions::new(0o4755).bits(), 0o4755);
    }

    #[test]
    fn metadata_builder() {
        let meta = Metadata::new(FileType::File, Permissions::new(0o600))
            .with_len(42)
            .with_allocated(512)
            .with_owner(Owner::new(1000, 100))
            .with_nlink(2)
            .with_generation(7)
            .with_attributes(Attributes::HIDDEN | Attributes::ARCHIVE)
            .with_modified(DateTime::UNIX_EPOCH);
        assert_eq!(meta.file_type(), FileType::File);
        assert_eq!((meta.len(), meta.allocated()), (42, 512));
        assert_eq!(meta.permissions(), Permissions::new(0o600));
        assert_eq!(meta.owner(), Some(Owner::new(1000, 100)));
        assert_eq!((meta.nlink(), meta.generation()), (2, 7));
        assert!(meta.attributes().contains(Attributes::HIDDEN));
        assert_eq!(meta.modified(), Some(DateTime::UNIX_EPOCH));
        assert_eq!(meta.created(), None);
        assert_eq!(meta.device(), None);
    }

    #[test]
    fn attributes_without() {
        let attrs = Attributes::HIDDEN.union(Attributes::ARCHIVE);
        assert_eq!(attrs.without(Attributes::HIDDEN), Attributes::ARCHIVE);
        assert_eq!(Attributes::NONE, Attributes::empty());
    }

    #[test]
    fn set_attr_defaults_to_no_change() {
        assert!(SetAttr::default().is_empty());
        let change = SetAttr::new()
            .with_permissions(Permissions::new(0o644))
            .with_owner(Owner::new(0, 0));
        assert!(!change.is_empty());
        assert_eq!(change.permissions(), Some(Permissions::new(0o644)));
        assert_eq!(change.owner(), Some(Owner::new(0, 0)));
        assert_eq!(change.modified(), None);
    }
}
