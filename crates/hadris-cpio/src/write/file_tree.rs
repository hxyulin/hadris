use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::mode::FileType;

/// An in-memory file tree representing the contents of a CPIO archive.
///
/// Build a tree programmatically using [`FileNode`] constructors and [`add`](FileTree::add),
/// or scan a directory with [`FileTree::from_fs`] (requires `std`).
pub struct FileTree {
    /// The top-level entries in the archive.
    pub root: Vec<FileNode>,
}

impl FileTree {
    /// Create an empty file tree.
    pub fn new() -> Self {
        Self { root: Vec::new() }
    }

    /// Append a node to the top level of the tree.
    pub fn add(&mut self, node: FileNode) {
        self.root.push(node);
    }
}

impl Default for FileTree {
    fn default() -> Self {
        Self::new()
    }
}

/// A single node in the file tree, representing one archive entry.
///
/// Use the convenience constructors ([`file`](FileNode::file), [`dir`](FileNode::dir),
/// [`symlink`](FileNode::symlink), etc.) for quick construction with default ownership,
/// or the `_with_owner` variants for full control.
pub enum FileNode {
    /// A regular file with in-memory contents.
    File {
        name: Arc<String>,
        contents: Vec<u8>,
        permissions: u32,
        uid: u32,
        gid: u32,
        mtime: u32,
    },
    /// A directory containing child nodes.
    Directory {
        name: Arc<String>,
        children: Vec<FileNode>,
        permissions: u32,
        uid: u32,
        gid: u32,
        mtime: u32,
    },
    /// A symbolic link. The file data is the target path.
    Symlink {
        name: Arc<String>,
        target: String,
        permissions: u32,
        uid: u32,
        gid: u32,
        mtime: u32,
    },
    /// A hard link to another entry in the tree. The `link_target` must be
    /// the archive path of a [`File`](FileNode::File) that appears earlier.
    HardLink {
        name: Arc<String>,
        link_target: String,
    },
    /// A block or character device node.
    DeviceNode {
        name: Arc<String>,
        device_type: FileType,
        major: u32,
        minor: u32,
        permissions: u32,
        uid: u32,
        gid: u32,
        mtime: u32,
    },
    /// A named pipe (FIFO).
    Fifo {
        name: Arc<String>,
        permissions: u32,
        uid: u32,
        gid: u32,
        mtime: u32,
    },
}

impl FileNode {
    /// Create a regular file node with uid/gid 0 and mtime 0.
    pub fn file(name: &str, contents: Vec<u8>, permissions: u32) -> Self {
        FileNode::File {
            name: Arc::new(String::from(name)),
            contents,
            permissions,
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }

    /// Create a regular file node with explicit ownership and mtime.
    pub fn file_with_owner(
        name: &str,
        contents: Vec<u8>,
        permissions: u32,
        uid: u32,
        gid: u32,
        mtime: u32,
    ) -> Self {
        FileNode::File {
            name: Arc::new(String::from(name)),
            contents,
            permissions,
            uid,
            gid,
            mtime,
        }
    }

    /// Create a directory node with uid/gid 0 and mtime 0.
    pub fn dir(name: &str, children: Vec<FileNode>, permissions: u32) -> Self {
        FileNode::Directory {
            name: Arc::new(String::from(name)),
            children,
            permissions,
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }

    /// Create a directory node with explicit ownership and mtime.
    pub fn dir_with_owner(
        name: &str,
        children: Vec<FileNode>,
        permissions: u32,
        uid: u32,
        gid: u32,
        mtime: u32,
    ) -> Self {
        FileNode::Directory {
            name: Arc::new(String::from(name)),
            children,
            permissions,
            uid,
            gid,
            mtime,
        }
    }

    /// Create a symbolic link with permissions `0o777` and uid/gid 0.
    pub fn symlink(name: &str, target: &str) -> Self {
        FileNode::Symlink {
            name: Arc::new(String::from(name)),
            target: String::from(target),
            permissions: 0o777,
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }

    /// Create a hard link to another entry's archive path.
    pub fn hard_link(name: &str, link_target: &str) -> Self {
        FileNode::HardLink {
            name: Arc::new(String::from(name)),
            link_target: String::from(link_target),
        }
    }

    /// Create a block or character device node.
    pub fn device(name: &str, device_type: FileType, major: u32, minor: u32, permissions: u32) -> Self {
        FileNode::DeviceNode {
            name: Arc::new(String::from(name)),
            device_type,
            major,
            minor,
            permissions,
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }

    /// Create a named pipe (FIFO) node.
    pub fn fifo(name: &str, permissions: u32) -> Self {
        FileNode::Fifo {
            name: Arc::new(String::from(name)),
            permissions,
            uid: 0,
            gid: 0,
            mtime: 0,
        }
    }

    /// Returns this node's filename (the last path component).
    pub fn node_name(&self) -> &str {
        match self {
            FileNode::File { name, .. }
            | FileNode::Directory { name, .. }
            | FileNode::Symlink { name, .. }
            | FileNode::HardLink { name, .. }
            | FileNode::DeviceNode { name, .. }
            | FileNode::Fifo { name, .. } => name,
        }
    }
}
