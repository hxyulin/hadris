#![allow(dead_code)]

//! Path helpers for the tests: whole-file reads and writes on a
//! `FileSystem` or a `Volume`, with paths resolved lexically.

/// The parent path and last name of `path`.
fn split(path: &str) -> (&str, &str) {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(at) => (&path[..at], &path[at + 1..]),
        None => ("", path),
    }
}

pub mod sync {
    use hadris_fs::sync::{FileSystem, Volume};
    use hadris_fs::{
        ErrorKind, FsResult, Metadata, Name, NodeId, OpenMode, OpenOptions, Resolve, SetAttr,
    };

    pub trait FsPaths: FileSystem {
        /// The node at `path`, pinned.
        fn resolve_path(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
            self.resolve(path.as_bytes(), Resolve::Lexical)
        }

        fn metadata(&mut self, path: &str) -> FsResult<Metadata, Self::DeviceError> {
            let node = self.resolve_path(path)?;
            let meta = self.stat(node);
            self.forget(node, 1);
            meta
        }

        fn exists(&mut self, path: &str) -> FsResult<bool, Self::DeviceError> {
            match self.resolve_path(path) {
                Ok(node) => {
                    self.forget(node, 1);
                    Ok(true)
                }
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
                Err(err) => Err(err),
            }
        }

        fn read_to_vec(&mut self, path: &str) -> FsResult<Vec<u8>, Self::DeviceError> {
            let node = self.resolve_path(path)?;
            let read = read_node(self, node);
            self.forget(node, 1);
            read
        }

        /// Creates or truncates the file at `path` and writes `data`.
        fn write_file(&mut self, path: &str, data: &[u8]) -> FsResult<(), Self::DeviceError> {
            let (parent, name) = super::split(path);
            let dir = self.resolve_path(parent)?;
            let node = match self.lookup(dir, Name::new(name)) {
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    self.create(dir, Name::new(name), &SetAttr::new())
                }
                other => other,
            };
            self.forget(dir, 1);
            let node = node?;
            let written = write_node(self, node, data);
            self.forget(node, 1);
            written
        }

        /// Removes `name` from `dir`, as `unlink` for a file and `rmdir`
        /// for a directory.
        fn remove_any(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
            match self.unlink(dir, name) {
                Err(err) if err.kind() == ErrorKind::IsADirectory => self.rmdir(dir, name),
                other => other,
            }
        }

        /// The volume label as text.
        fn label_text(&mut self) -> FsResult<Option<String>, Self::DeviceError> {
            let mut buf = [0u8; 64];
            Ok(self.label(&mut buf)?.map(str::to_owned))
        }
    }

    impl<F: FileSystem + ?Sized> FsPaths for F {}

    fn read_node<F: FileSystem + ?Sized>(
        fs: &mut F,
        node: NodeId,
    ) -> FsResult<Vec<u8>, F::DeviceError> {
        fs.open(node, OpenMode::Read)?;
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        let read = loop {
            match fs.read(node, out.len() as u64, &mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(err) => break Err(err),
            }
        };
        let closed = fs.close(node);
        read.and(closed).map(|()| out)
    }

    fn write_node<F: FileSystem + ?Sized>(
        fs: &mut F,
        node: NodeId,
        data: &[u8],
    ) -> FsResult<(), F::DeviceError> {
        fs.open(node, OpenMode::Write)?;
        let mut done = 0;
        let written = match fs.truncate(node, 0) {
            Ok(()) => loop {
                if done == data.len() {
                    break Ok(());
                }
                match fs.write(node, done as u64, &data[done..]) {
                    Ok(0) => break Err(ErrorKind::NoSpace.into()),
                    Ok(n) => done += n,
                    Err(err) => break Err(err),
                }
            },
            Err(err) => Err(err),
        };
        let closed = fs.close(node);
        written.and(closed)
    }

    pub trait VolumePaths {
        type Error;

        fn read_to_vec(&self, path: &str) -> FsResult<Vec<u8>, Self::Error>;
        fn write_file(&self, path: &str, data: &[u8]) -> FsResult<(), Self::Error>;
        fn exists(&self, path: &str) -> FsResult<bool, Self::Error>;
    }

    impl<F: FileSystem> VolumePaths for Volume<F> {
        type Error = F::DeviceError;

        fn read_to_vec(&self, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
            let mut file = self.open(path, OpenOptions::new().read())?;
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                match file.read(&mut buf)? {
                    0 => break,
                    n => out.extend_from_slice(&buf[..n]),
                }
            }
            file.close()?;
            Ok(out)
        }

        fn write_file(&self, path: &str, data: &[u8]) -> FsResult<(), F::DeviceError> {
            let options = OpenOptions::new().write().create().truncate();
            let mut file = self.open(path, options)?;
            let mut data = data;
            while !data.is_empty() {
                match file.write(data)? {
                    0 => return Err(ErrorKind::NoSpace.into()),
                    n => data = &data[n..],
                }
            }
            file.close()
        }

        fn exists(&self, path: &str) -> FsResult<bool, F::DeviceError> {
            match self.metadata(path) {
                Ok(_) => Ok(true),
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
                Err(err) => Err(err),
            }
        }
    }
}

pub mod r#async {
    use hadris_fs::r#async::{FileSystem, Volume};
    use hadris_fs::{
        ErrorKind, FsResult, Metadata, Name, NodeId, OpenMode, OpenOptions, Resolve, SetAttr,
    };

    #[allow(async_fn_in_trait)]
    pub trait FsPaths: FileSystem {
        /// The node at `path`, pinned.
        async fn resolve_path(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
            self.resolve(path.as_bytes(), Resolve::Lexical).await
        }

        async fn metadata(&mut self, path: &str) -> FsResult<Metadata, Self::DeviceError> {
            let node = self.resolve_path(path).await?;
            let meta = self.stat(node).await;
            self.forget(node, 1);
            meta
        }

        async fn exists(&mut self, path: &str) -> FsResult<bool, Self::DeviceError> {
            match self.resolve_path(path).await {
                Ok(node) => {
                    self.forget(node, 1);
                    Ok(true)
                }
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
                Err(err) => Err(err),
            }
        }

        async fn read_to_vec(&mut self, path: &str) -> FsResult<Vec<u8>, Self::DeviceError> {
            let node = self.resolve_path(path).await?;
            let read = read_node(self, node).await;
            self.forget(node, 1);
            read
        }

        /// Creates or truncates the file at `path` and writes `data`.
        async fn write_file(&mut self, path: &str, data: &[u8]) -> FsResult<(), Self::DeviceError> {
            let (parent, name) = super::split(path);
            let dir = self.resolve_path(parent).await?;
            let node = match self.lookup(dir, Name::new(name)).await {
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    self.create(dir, Name::new(name), &SetAttr::new()).await
                }
                other => other,
            };
            self.forget(dir, 1);
            let node = node?;
            let written = write_node(self, node, data).await;
            self.forget(node, 1);
            written
        }

        /// Removes `name` from `dir`, as `unlink` for a file and `rmdir`
        /// for a directory.
        async fn remove_any(
            &mut self,
            dir: NodeId,
            name: &Name,
        ) -> FsResult<(), Self::DeviceError> {
            match self.unlink(dir, name).await {
                Err(err) if err.kind() == ErrorKind::IsADirectory => self.rmdir(dir, name).await,
                other => other,
            }
        }

        /// The volume label as text.
        async fn label_text(&mut self) -> FsResult<Option<String>, Self::DeviceError> {
            let mut buf = [0u8; 64];
            Ok(self.label(&mut buf).await?.map(str::to_owned))
        }
    }

    impl<F: FileSystem + ?Sized> FsPaths for F {}

    async fn read_node<F: FileSystem + ?Sized>(
        fs: &mut F,
        node: NodeId,
    ) -> FsResult<Vec<u8>, F::DeviceError> {
        fs.open(node, OpenMode::Read).await?;
        let mut out = Vec::new();
        let mut buf = [0u8; 8192];
        let read = loop {
            match fs.read(node, out.len() as u64, &mut buf).await {
                Ok(0) => break Ok(()),
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(err) => break Err(err),
            }
        };
        let closed = fs.close(node).await;
        read.and(closed).map(|()| out)
    }

    async fn write_node<F: FileSystem + ?Sized>(
        fs: &mut F,
        node: NodeId,
        data: &[u8],
    ) -> FsResult<(), F::DeviceError> {
        fs.open(node, OpenMode::Write).await?;
        let mut done = 0;
        let written = match fs.truncate(node, 0).await {
            Ok(()) => loop {
                if done == data.len() {
                    break Ok(());
                }
                match fs.write(node, done as u64, &data[done..]).await {
                    Ok(0) => break Err(ErrorKind::NoSpace.into()),
                    Ok(n) => done += n,
                    Err(err) => break Err(err),
                }
            },
            Err(err) => Err(err),
        };
        let closed = fs.close(node).await;
        written.and(closed)
    }

    #[allow(async_fn_in_trait)]
    pub trait VolumePaths {
        type Error;

        async fn read_to_vec(&self, path: &str) -> FsResult<Vec<u8>, Self::Error>;
        async fn write_file(&self, path: &str, data: &[u8]) -> FsResult<(), Self::Error>;
        async fn exists(&self, path: &str) -> FsResult<bool, Self::Error>;
    }

    impl<F: FileSystem> VolumePaths for Volume<F> {
        type Error = F::DeviceError;

        async fn read_to_vec(&self, path: &str) -> FsResult<Vec<u8>, F::DeviceError> {
            let mut file = self.open(path, OpenOptions::new().read()).await?;
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                match file.read(&mut buf).await? {
                    0 => break,
                    n => out.extend_from_slice(&buf[..n]),
                }
            }
            file.close().await?;
            Ok(out)
        }

        async fn write_file(&self, path: &str, data: &[u8]) -> FsResult<(), F::DeviceError> {
            let options = OpenOptions::new().write().create().truncate();
            let mut file = self.open(path, options).await?;
            let mut data = data;
            while !data.is_empty() {
                match file.write(data).await? {
                    0 => return Err(ErrorKind::NoSpace.into()),
                    n => data = &data[n..],
                }
            }
            file.close().await
        }

        async fn exists(&self, path: &str) -> FsResult<bool, F::DeviceError> {
            match self.metadata(path).await {
                Ok(_) => Ok(true),
                Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
                Err(err) => Err(err),
            }
        }
    }
}
