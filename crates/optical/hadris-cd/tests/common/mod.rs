#![allow(dead_code)]

/// Path reads for tests on the bare tier, as the removed `DriverExt` gave.
pub trait Paths: hadris_fs::sync::FileSystem {
    fn resolve_path(
        &mut self,
        path: &str,
    ) -> hadris_fs::FsResult<hadris_fs::NodeId, Self::DeviceError> {
        self.resolve(path.as_bytes(), hadris_fs::Resolve::Lexical)
    }

    fn metadata(
        &mut self,
        path: &str,
    ) -> hadris_fs::FsResult<hadris_fs::Metadata, Self::DeviceError> {
        let node = self.resolve_path(path)?;
        let meta = self.stat(node);
        self.forget(node, 1);
        meta
    }

    fn exists(&mut self, path: &str) -> hadris_fs::FsResult<bool, Self::DeviceError> {
        match self.resolve_path(path) {
            Ok(node) => {
                self.forget(node, 1);
                Ok(true)
            }
            Err(err) if err.kind() == hadris_fs::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn read_to_vec(&mut self, path: &str) -> hadris_fs::FsResult<Vec<u8>, Self::DeviceError> {
        let node = self.resolve_path(path)?;
        let data = read_node(self, node);
        self.forget(node, 1);
        data
    }

    fn names(&mut self, path: &str) -> hadris_fs::FsResult<Vec<String>, Self::DeviceError> {
        let dir = self.resolve_path(path)?;
        let mut names = Vec::new();
        let mut cursor = hadris_fs::DirCursor::START;
        let listed = loop {
            match self.readdir(dir, cursor) {
                Ok(Some(entry)) => {
                    cursor = entry.next_cursor();
                    names.push(String::from_utf8_lossy(entry.name().as_bytes()).into_owned());
                }
                Ok(None) => break Ok(names),
                Err(err) => break Err(err),
            }
        };
        self.forget(dir, 1);
        listed
    }
}

impl<F: hadris_fs::sync::FileSystem + ?Sized> Paths for F {}

/// Opens `node`, reads it whole and closes it.
pub fn read_node<F: hadris_fs::sync::FileSystem + ?Sized>(
    fs: &mut F,
    node: hadris_fs::NodeId,
) -> hadris_fs::FsResult<Vec<u8>, F::DeviceError> {
    fs.open(node, hadris_fs::OpenMode::Read)?;
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
