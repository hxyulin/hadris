use core::fmt;

use crate::{FileType, Metadata, Name, NameError, NodeId};

/// A resumable position in a directory listing.
///
/// The cursor is a plain value that callers may store and reuse. Its raw
/// form round-trips through [`from_raw`](Self::from_raw) and
/// [`into_raw`](Self::into_raw), so it can serve as a FUSE `readdir` offset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DirCursor(u64);

impl DirCursor {
    /// The position before the first entry. Its raw value is 0.
    pub const START: Self = Self(0);

    /// The largest raw value a driver may return, so that a cursor fits an
    /// `off_t` with room for the `.` and `..` entries a FUSE layer adds.
    pub const MAX_RAW: u64 = (1 << 63) - 16;

    /// Recreates a cursor from a value returned by [`into_raw`](Self::into_raw).
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw cursor value.
    pub const fn into_raw(self) -> u64 {
        self.0
    }

    /// Returns whether this cursor is at the start of the listing.
    pub const fn is_start(self) -> bool {
        self.0 == 0
    }
}

/// One directory listing entry: its name, node, metadata and the cursor
/// that continues the listing after it.
///
/// It owns its name inline, so listings need no allocation. The node is not
/// pinned.
#[derive(Clone, Copy)]
pub struct DirEntry {
    name: [u8; DirEntry::MAX_NAME],
    name_len: u16,
    node: NodeId,
    metadata: Metadata,
    next_cursor: DirCursor,
}

impl DirEntry {
    /// The longest name an entry holds, in bytes: 255 UTF-16 code units of
    /// a FAT, exFAT or NTFS name take at most 765 bytes of UTF-8.
    pub const MAX_NAME: usize = 768;

    /// Creates an entry. Fails when `name` is not a valid name or is longer
    /// than [`MAX_NAME`](Self::MAX_NAME).
    pub fn new(
        name: &Name,
        node: NodeId,
        metadata: Metadata,
        next_cursor: DirCursor,
    ) -> Result<Self, NameError> {
        name.check()?;
        let bytes = name.as_bytes();
        let mut stored = [0u8; Self::MAX_NAME];
        stored
            .get_mut(..bytes.len())
            .ok_or(NameError::TooLong)?
            .copy_from_slice(bytes);
        Ok(Self {
            name: stored,
            name_len: bytes.len() as u16,
            node,
            metadata,
            next_cursor,
        })
    }

    /// Returns the entry's name.
    pub fn name(&self) -> &Name {
        Name::from_bytes(&self.name[..usize::from(self.name_len)])
    }

    /// Returns the entry's node. It is not pinned.
    pub const fn node(&self) -> NodeId {
        self.node
    }

    /// Returns the entry's metadata.
    pub const fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the entry's file type.
    pub const fn file_type(&self) -> FileType {
        self.metadata.file_type()
    }

    /// Returns the cursor that continues the listing after this entry.
    pub const fn next_cursor(&self) -> DirCursor {
        self.next_cursor
    }
}

impl PartialEq for DirEntry {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
            && self.node == other.node
            && self.metadata == other.metadata
            && self.next_cursor == other.next_cursor
    }
}

impl Eq for DirEntry {}

impl fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirEntry")
            .field("name", &self.name())
            .field("node", &self.node)
            .field("metadata", &self.metadata)
            .field("next_cursor", &self.next_cursor)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Permissions;

    #[test]
    fn entry_holds_its_name() {
        let node = NodeId::new(7).unwrap();
        let meta = Metadata::new(FileType::File, Permissions::new(0o644)).with_len(3);
        let entry = DirEntry::new(Name::new("a.txt"), node, meta, DirCursor::from_raw(2)).unwrap();
        assert_eq!(entry.name().as_bytes(), b"a.txt");
        assert_eq!(entry.node(), node);
        assert_eq!(entry.metadata().len(), 3);
        assert_eq!(entry.file_type(), FileType::File);
        assert_eq!(entry.next_cursor().into_raw(), 2);
        let long = [b'x'; DirEntry::MAX_NAME];
        assert!(DirEntry::new(Name::new(&long), node, meta, DirCursor::START).is_ok());
        let longer = [b'x'; DirEntry::MAX_NAME + 1];
        assert_eq!(
            DirEntry::new(Name::new(&longer), node, meta, DirCursor::START),
            Err(NameError::TooLong)
        );
        assert_eq!(
            DirEntry::new(Name::new(".."), node, meta, DirCursor::START),
            Err(NameError::ParentDir)
        );
        assert!(DirCursor::START.is_start());
    }
}
