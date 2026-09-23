use crate::{FileType, Name, NameBuf, NodeId};

/// A resumable position in a directory listing.
///
/// The cursor is a plain value that callers may store and reuse. Its raw
/// form round-trips through [`from_raw`](Self::from_raw) and
/// [`into_raw`](Self::into_raw), so it can serve as a FUSE `readdir` offset.
/// The raw value of [`start`](Self::start) is `0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct DirCursor(u64);

impl DirCursor {
    /// The largest raw value a driver may return, so that a cursor fits an
    /// `off_t` with room for the `.` and `..` entries a FUSE layer adds.
    pub const MAX_RAW: u64 = (1 << 63) - 16;

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

/// A directory entry together with its name, as yielded by a directory
/// handle.
#[derive(Debug, Clone)]
pub struct DirItem {
    name: NameBuf,
    entry: DirEntry,
}

impl DirItem {
    /// Pairs an entry with the name its filesystem wrote. `None` if the
    /// buffer holds no name.
    pub fn new(name: NameBuf, entry: DirEntry) -> Option<Self> {
        (!name.is_empty()).then_some(Self { name, entry })
    }

    /// The entry's name.
    pub fn name(&self) -> &Name {
        const UNREACHABLE: &Name = match Name::from_bytes(b"?") {
            Ok(name) => name,
            Err(_) => panic!(),
        };
        self.name.as_name().unwrap_or(UNREACHABLE)
    }

    /// The name's bytes.
    pub fn name_bytes(&self) -> &[u8] {
        self.name.as_bytes()
    }

    /// The name as UTF-8, if it is.
    pub fn name_str(&self) -> Option<&str> {
        core::str::from_utf8(self.name.as_bytes()).ok()
    }

    /// Node, type and name length. The node is not pinned.
    pub fn entry(&self) -> DirEntry {
        self.entry
    }

    /// The entry's file type.
    pub fn file_type(&self) -> FileType {
        self.entry.file_type()
    }
}
