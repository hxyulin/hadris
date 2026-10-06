//! Path reads over the bare `FileSystem` trait, for the tests.

use hadris_fs::sync::FileSystem;
use hadris_fs::{DirCursor, DirEntry, FsResult, Metadata, OpenMode, Resolve};

pub trait PathOps: FileSystem {
    fn read_to_vec(&mut self, path: &str) -> FsResult<Vec<u8>, Self::DeviceError> {
        let node = self.resolve(path.as_bytes(), Resolve::Lexical)?;
        let data = read_node(self, node);
        self.forget(node, 1);
        data
    }

    fn metadata(&mut self, path: &str) -> FsResult<Metadata, Self::DeviceError> {
        let node = self.resolve(path.as_bytes(), Resolve::Lexical)?;
        let meta = self.stat(node);
        self.forget(node, 1);
        meta
    }

    fn entries(&mut self, path: &str) -> FsResult<Vec<DirEntry>, Self::DeviceError> {
        let dir = self.resolve(path.as_bytes(), Resolve::Lexical)?;
        let mut out = Vec::new();
        let mut cursor = DirCursor::START;
        let listed = loop {
            match self.readdir(dir, cursor) {
                Ok(Some(entry)) => {
                    cursor = entry.next_cursor();
                    out.push(entry);
                }
                Ok(None) => break Ok(out),
                Err(err) => break Err(err),
            }
        };
        self.forget(dir, 1);
        listed
    }
}

impl<F: FileSystem + ?Sized> PathOps for F {}

fn read_node<F: FileSystem + ?Sized>(
    fs: &mut F,
    node: hadris_fs::NodeId,
) -> FsResult<Vec<u8>, F::DeviceError> {
    fs.open(node, OpenMode::Read)?;
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    let read = loop {
        match fs.read(node, out.len() as u64, &mut chunk) {
            Ok(0) => break Ok(()),
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(err) => break Err(err),
        }
    };
    let closed = fs.close(node);
    read?;
    closed?;
    Ok(out)
}
