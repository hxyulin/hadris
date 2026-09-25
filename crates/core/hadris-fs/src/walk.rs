use crate::{DirCursor, DirEntry, NodeId};

/// One level of a walk stack that the caller lends to `Walk::with_stack`:
/// a directory being listed and where its listing stands. A stack of `n`
/// frames walks `n` levels deep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkFrame(pub(crate) Option<(NodeId, DirCursor)>);

impl WalkFrame {
    /// A frame holding no directory.
    pub const EMPTY: Self = Self(None);
}

/// One entry of a walk: the directory entry and its depth, 1 for the
/// children of the directory the walk started from. A stack of names cut
/// to `depth - 1` before each push builds the entry's path.
#[derive(Clone, Copy)]
pub struct WalkEntry {
    entry: DirEntry,
    depth: u32,
}

impl WalkEntry {
    #[cfg(any(feature = "sync", feature = "async"))]
    pub(crate) fn new(entry: DirEntry, depth: u32) -> Self {
        Self { entry, depth }
    }

    /// The directory entry.
    pub fn entry(&self) -> &DirEntry {
        &self.entry
    }

    /// The depth, 1 for the children of the starting directory.
    pub fn depth(&self) -> u32 {
        self.depth
    }
}

impl core::fmt::Debug for WalkEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WalkEntry")
            .field("entry", &self.entry)
            .field("depth", &self.depth)
            .finish()
    }
}
