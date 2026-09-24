#![allow(dead_code)]

//! Whole-file helpers over the bare `FileSystem` trait, in each mode.

pub mod sync {
    use hadris_fs::sync::FileSystem;
    use hadris_fs::{ErrorKind, FsResult, Name, OpenMode, Resolve, SetAttr};

    /// Creates or replaces the file at `path` with `data`.
    pub fn put<F: FileSystem>(fs: &mut F, path: &str, data: &[u8]) -> FsResult<(), F::DeviceError> {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let dir = fs.resolve(dir.as_bytes(), Resolve::Lexical)?;
        let node = match fs.lookup(dir, Name::new(name)) {
            Err(err) if err.kind() == ErrorKind::NotFound => {
                fs.create(dir, Name::new(name), &SetAttr::new())
            }
            other => other,
        };
        fs.forget(dir, 1);
        let node = node?;
        let mut written = fs.open(node, OpenMode::Write);
        if written.is_ok() {
            written = fs.truncate(node, 0);
            let mut offset = 0;
            while written.is_ok() && offset < data.len() {
                match fs.write(node, offset as u64, &data[offset..]) {
                    Ok(n) => offset += n,
                    Err(err) => written = Err(err),
                }
            }
            let closed = fs.close(node);
            written = written.and(closed);
        }
        fs.forget(node, 1);
        written
    }

    /// The contents of the file at `path`.
    pub fn get<F: FileSystem>(fs: &mut F, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
        let node = fs.resolve(path.as_bytes(), Resolve::Lexical)?;
        let mut out = Vec::new();
        let mut read = fs.open(node, OpenMode::Read);
        if read.is_ok() {
            let mut chunk = [0u8; 4096];
            loop {
                match fs.read(node, out.len() as u64, &mut chunk) {
                    Ok(0) => break,
                    Ok(n) => out.extend_from_slice(&chunk[..n]),
                    Err(err) => {
                        read = Err(err);
                        break;
                    }
                }
            }
            let closed = fs.close(node);
            read = read.and(closed);
        }
        fs.forget(node, 1);
        read.map(|()| out)
    }
}

#[cfg(feature = "async")]
pub mod asynch {
    use hadris_fs::r#async::FileSystem;
    use hadris_fs::{ErrorKind, FsResult, Name, OpenMode, Resolve, SetAttr};

    /// Creates or replaces the file at `path` with `data`.
    pub async fn put<F: FileSystem>(
        fs: &mut F,
        path: &str,
        data: &[u8],
    ) -> FsResult<(), F::DeviceError> {
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let dir = fs.resolve(dir.as_bytes(), Resolve::Lexical).await?;
        let node = match fs.lookup(dir, Name::new(name)).await {
            Err(err) if err.kind() == ErrorKind::NotFound => {
                fs.create(dir, Name::new(name), &SetAttr::new()).await
            }
            other => other,
        };
        fs.forget(dir, 1);
        let node = node?;
        let mut written = fs.open(node, OpenMode::Write).await;
        if written.is_ok() {
            written = fs.truncate(node, 0).await;
            let mut offset = 0;
            while written.is_ok() && offset < data.len() {
                match fs.write(node, offset as u64, &data[offset..]).await {
                    Ok(n) => offset += n,
                    Err(err) => written = Err(err),
                }
            }
            let closed = fs.close(node).await;
            written = written.and(closed);
        }
        fs.forget(node, 1);
        written
    }

    /// The contents of the file at `path`.
    pub async fn get<F: FileSystem>(fs: &mut F, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
        let node = fs.resolve(path.as_bytes(), Resolve::Lexical).await?;
        let mut out = Vec::new();
        let mut read = fs.open(node, OpenMode::Read).await;
        if read.is_ok() {
            let mut chunk = [0u8; 4096];
            loop {
                match fs.read(node, out.len() as u64, &mut chunk).await {
                    Ok(0) => break,
                    Ok(n) => out.extend_from_slice(&chunk[..n]),
                    Err(err) => {
                        read = Err(err);
                        break;
                    }
                }
            }
            let closed = fs.close(node).await;
            read = read.and(closed);
        }
        fs.forget(node, 1);
        read.map(|()| out)
    }
}
