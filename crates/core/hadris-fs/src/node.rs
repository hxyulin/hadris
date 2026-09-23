/// Opaque identifier of a node within one filesystem.
///
/// The value is stable for as long as the node is pinned (between a lookup
/// and the matching forget). It maps directly to FUSE `ino` values and
/// kernel inode numbers. Filesystems choose the encoding; callers must not
/// interpret it. No filesystem hands out 0, which FUSE reserves; the root
/// may have any other value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u64);

impl NodeId {
    /// Creates an identifier from its raw value.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Returns the raw value.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The type of a filesystem node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileType {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link.
    Symlink,
    /// A character device.
    CharDevice,
    /// A block device.
    BlockDevice,
    /// A named pipe.
    Fifo,
    /// A Unix domain socket.
    Socket,
}

impl FileType {
    /// Returns whether this is a regular file.
    pub const fn is_file(self) -> bool {
        matches!(self, Self::File)
    }

    /// Returns whether this is a directory.
    pub const fn is_dir(self) -> bool {
        matches!(self, Self::Dir)
    }

    /// Returns whether this is a symbolic link.
    pub const fn is_symlink(self) -> bool {
        matches!(self, Self::Symlink)
    }
}
