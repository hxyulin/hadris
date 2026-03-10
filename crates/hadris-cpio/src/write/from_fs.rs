use std::path::Path;

use alloc::sync::Arc;
use alloc::vec::Vec;

use super::file_tree::{FileNode, FileTree};

#[cfg(unix)]
fn meta_uid(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    meta.uid()
}
#[cfg(not(unix))]
fn meta_uid(_meta: &std::fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn meta_gid(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    meta.gid()
}
#[cfg(not(unix))]
fn meta_gid(_meta: &std::fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
fn meta_mtime(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime() as u32
}
#[cfg(not(unix))]
fn meta_mtime(meta: &std::fs::Metadata) -> u32 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

#[cfg(unix)]
fn meta_permissions(meta: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    meta.mode() & 0o7777
}
#[cfg(not(unix))]
fn meta_permissions(meta: &std::fs::Metadata) -> u32 {
    if meta.is_dir() {
        0o755
    } else if meta.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

/// Error returned by [`FileTree::from_fs`].
#[derive(Debug)]
pub enum FromFsError {
    /// An I/O error occurred while scanning the directory tree.
    Io(std::io::Error),
}

impl core::fmt::Display for FromFsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "filesystem scan I/O error: {e}"),
        }
    }
}

impl std::error::Error for FromFsError {}

impl From<std::io::Error> for FromFsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl FileTree {
    /// Recursively scan a directory and build a FileTree.
    ///
    /// Handles regular files, directories, and symlinks.
    /// Skips device nodes, FIFOs, and sockets.
    /// Entries are sorted by name for deterministic output.
    pub fn from_fs(root_path: &Path) -> core::result::Result<Self, FromFsError> {
        let mut root = Vec::new();
        scan_dir(root_path, &mut root)?;
        root.sort_by(|a, b| a.node_name().cmp(b.node_name()));
        Ok(FileTree { root })
    }
}

fn scan_dir(dir: &Path, out: &mut Vec<FileNode>) -> core::result::Result<(), FromFsError> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let ft = entry.file_type()?;

        if ft.is_symlink() {
            let target = std::fs::read_link(&path)?;
            let target_str = target.to_string_lossy().into_owned();
            let meta = std::fs::symlink_metadata(&path)?;
            out.push(FileNode::Symlink {
                name: Arc::new(name),
                target: target_str,
                permissions: meta_permissions(&meta),
                uid: meta_uid(&meta),
                gid: meta_gid(&meta),
                mtime: meta_mtime(&meta),
            });
        } else if ft.is_dir() {
            let meta = std::fs::metadata(&path)?;
            let mut children = Vec::new();
            scan_dir(&path, &mut children)?;
            children.sort_by(|a, b| a.node_name().cmp(b.node_name()));
            out.push(FileNode::Directory {
                name: Arc::new(name),
                children,
                permissions: meta_permissions(&meta),
                uid: meta_uid(&meta),
                gid: meta_gid(&meta),
                mtime: meta_mtime(&meta),
            });
        } else if ft.is_file() {
            let meta = std::fs::metadata(&path)?;
            let contents = std::fs::read(&path)?;
            out.push(FileNode::File {
                name: Arc::new(name),
                contents,
                permissions: meta_permissions(&meta),
                uid: meta_uid(&meta),
                gid: meta_gid(&meta),
                mtime: meta_mtime(&meta),
            });
        }
        // Skip device nodes, FIFOs, sockets
    }

    Ok(())
}
