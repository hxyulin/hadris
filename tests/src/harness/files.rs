//! Whole-file reads and listings over the bare `hadris-fs` `FileSystem`
//! trait, for the Hadris adapters.

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, DirEntry, FsResult, NodeId, OpenMode, Resolve};

/// The contents of the pinned file `node`.
pub fn read_node<F: FileSystem>(fs: &mut F, node: NodeId) -> FsResult<Vec<u8>, F::DeviceError> {
    fs.open(node, OpenMode::Read)?;
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    let read = loop {
        match fs.read(node, out.len() as u64, &mut chunk) {
            Ok(0) => break Ok(()),
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(error) => break Err(error),
        }
    };
    let closed = fs.close(node);
    read?;
    closed?;
    Ok(out)
}

/// The contents of the file at `path`, resolved lexically.
pub fn read_path<F: FileSystem>(fs: &mut F, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
    let node = fs.resolve(path.as_bytes(), Resolve::Lexical)?;
    let data = read_node(fs, node);
    fs.forget(node, 1);
    data
}

/// Every entry of the directory `dir`, in listing order.
pub fn entries<F: FileSystem>(fs: &mut F, dir: NodeId) -> FsResult<Vec<DirEntry>, F::DeviceError> {
    let mut out = Vec::new();
    let mut cursor = DirCursor::START;
    while let Some(entry) = fs.readdir(dir, cursor)? {
        cursor = entry.next_cursor();
        out.push(entry);
    }
    Ok(out)
}
