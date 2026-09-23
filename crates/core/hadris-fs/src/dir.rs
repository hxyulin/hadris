use crate::{FileType, NodeId};

/// A resumable position in a directory listing.
///
/// The cursor is a plain value that callers may store and reuse. Its raw
/// form round-trips through [`from_raw`](Self::from_raw) and
/// [`into_raw`](Self::into_raw), so it can serve as a FUSE `readdir` offset.
/// The raw value of [`start`](Self::start) is `0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DirCursor(u64);

impl DirCursor {
    /// Returns a cursor positioned before the first entry.
    pub const fn start() -> Self {
        Self(0)
    }

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

/// One directory entry. The name is written to a caller-provided buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DirEntry {
    node: NodeId,
    file_type: FileType,
    name_len: usize,
}

impl DirEntry {
    /// Creates an entry.
    pub const fn new(node: NodeId, file_type: FileType, name_len: usize) -> Self {
        Self {
            node,
            file_type,
            name_len,
        }
    }

    /// Returns the entry's node. It is not pinned.
    pub const fn node(&self) -> NodeId {
        self.node
    }

    /// Returns the entry's file type.
    pub const fn file_type(&self) -> FileType {
        self.file_type
    }

    /// Returns the length in bytes of the entry's name.
    pub const fn name_len(&self) -> usize {
        self.name_len
    }
}
