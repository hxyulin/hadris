use super::*;

io_transform! {

async fn resolve_parent<'p, D: FsDriver + ?Sized>(
    fs: &mut D,
    path: &'p str,
) -> FsResult<(NodeId, &'p Name), D::DeviceError> {
    let (parent, name) = VPath::new(path).split_file().ok_or(ErrorKind::InvalidInput)?;
    let name = Name::new(name)?;
    Ok((fs.resolve(parent.as_str()).await?, name))
}

async fn pinned_metadata<D: FsDriver + ?Sized>(
    fs: &mut D,
    node: NodeId,
) -> FsResult<Metadata, D::DeviceError> {
    let meta = fs.node_metadata(node).await;
    fs.forget(node);
    meta
}

pub(super) async fn open_node<D: FsDriver + ?Sized>(
    fs: &mut D,
    path: &str,
    opts: OpenOptions,
) -> FsResult<NodeId, D::DeviceError> {
    opts.validate().map_err(|err| Error::from(err.kind()))?;
    if opts.is_write() && !fs.capabilities().is_writable() {
        return Err(ErrorKind::ReadOnly.into());
    }
    let node = if opts.is_create() || opts.is_create_new() {
        let (dir, name) = resolve_parent(fs, path).await?;
        let node = match fs.lookup(dir, name).await {
            Ok(node) if opts.is_create_new() => {
                fs.forget(node);
                Err(ErrorKind::AlreadyExists.into())
            }
            Ok(node) => Ok(node),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                fs.create(dir, name, NewNode::File, &SetMetadata::new()).await
            }
            Err(err) => Err(err),
        };
        fs.forget(dir);
        node?
    } else {
        fs.resolve(path).await?
    };
    let checked = match fs.node_metadata(node).await {
        Ok(meta) if meta.file_type().is_dir() => Err(ErrorKind::IsADirectory.into()),
        Ok(meta) if meta.file_type().is_symlink() => Err(ErrorKind::Symlink.into()),
        Ok(meta) if opts.is_truncate() && meta.len() > 0 => fs.set_len(node, 0).await,
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    };
    match checked {
        Ok(()) => Ok(node),
        Err(err) => {
            fs.forget(node);
            Err(err)
        }
    }
}

async fn exists<D: FsDriver + ?Sized>(fs: &mut D, path: &str) -> FsResult<bool, D::DeviceError> {
    match fs.resolve(path).await {
        Ok(node) => {
            fs.forget(node);
            Ok(true)
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

async fn create_dir_all<D: FsDriver + ?Sized>(fs: &mut D, path: &str) -> FsResult<(), D::DeviceError> {
    let mut current: Option<NodeId> = None;
    let mut result = Ok(());
    for component in VPath::new(path).components() {
        let name = match component {
            Component::Normal(name) => match Name::new(name) {
                Ok(name) => name,
                Err(err) => {
                    result = Err(err.into());
                    break;
                }
            },
            Component::Root | Component::Current => continue,
            _ => {
                result = Err(ErrorKind::InvalidInput.into());
                break;
            }
        };
        let dir = current.unwrap_or(fs.root());
        let node = match fs.lookup(dir, name).await {
            Err(err) if err.kind() == ErrorKind::NotFound => {
                fs.create(dir, name, NewNode::Dir, &SetMetadata::new()).await
            }
            other => other,
        };
        if let Some(prev) = current.take() {
            fs.forget(prev);
        }
        let node = match node {
            Ok(node) => node,
            Err(err) => {
                result = Err(err);
                break;
            }
        };
        current = Some(node);
        match fs.node_metadata(node).await {
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => {
                result = Err(ErrorKind::NotADirectory.into());
                break;
            }
            Err(err) => {
                result = Err(err);
                break;
            }
        }
    }
    if let Some(node) = current {
        fs.forget(node);
    }
    result
}

#[cfg(feature = "alloc")]
async fn read_to_vec<D: FsDriver + ?Sized>(
    fs: &mut D,
    path: &str,
) -> FsResult<alloc::vec::Vec<u8>, D::DeviceError> {
    let mut file = OpenFile::open(fs, path, OpenOptions::read()).await?;
    let mut out = alloc::vec::Vec::new();
    let mut chunk = [0u8; 4096];
    let result = loop {
        match file.read(fs, &mut chunk).await {
            Ok(0) => break Ok(out),
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(err) => break Err(err),
        }
    };
    fs.forget(file.node());
    result
}

async fn write_file<D: FsDriver + ?Sized>(fs: &mut D, path: &str, data: &[u8]) -> FsResult<(), D::DeviceError> {
    let mut file = OpenFile::open(fs, path, OpenOptions::write().create().truncate()).await?;
    let mut rest = data;
    let mut result = Ok(());
    while !rest.is_empty() {
        match file.write(fs, rest).await {
            Ok(0) => {
                result = Err(ErrorKind::NoSpace.into());
                break;
            }
            Ok(n) => rest = &rest[n..],
            Err(err) => {
                result = Err(err);
                break;
            }
        }
    }
    file.close(fs, result).await
}

/// Removes `name` from `dir` after checking its type: a directory when
/// `want_dir`, anything else otherwise.
async fn remove_checked<D: FsDriver + ?Sized>(
    fs: &mut D,
    path: &str,
    want_dir: bool,
) -> FsResult<(), D::DeviceError> {
    let (dir, name) = resolve_parent(fs, path).await?;
    let result = match fs.lookup(dir, name).await {
        Ok(node) => {
            let meta = pinned_metadata(fs, node).await;
            match meta {
                Ok(meta) if meta.file_type().is_dir() != want_dir => Err(if want_dir {
                    ErrorKind::NotADirectory.into()
                } else {
                    ErrorKind::IsADirectory.into()
                }),
                Ok(_) => fs.remove(dir, name).await,
                Err(err) => Err(err),
            }
        }
        Err(err) => Err(err),
    };
    fs.forget(dir);
    result
}

/// Directories `remove_dir_all` descends before [`ErrorKind::LimitExceeded`].
const MAX_DEPTH: usize = 64;

/// Finds the name of `child` in `dir`.
async fn name_of<D: FsDriver + ?Sized>(
    fs: &mut D,
    dir: NodeId,
    child: NodeId,
    name: &mut NameBuf,
) -> FsResult<(), D::DeviceError> {
    let mut cursor = DirCursor::start();
    while let Some(entry) = fs.read_dir_entry(dir, &mut cursor, name).await? {
        if entry.node() == child {
            return Ok(());
        }
    }
    Err(ErrorKind::NotFound.into())
}

/// Empties the directory at `top`, depth first, with a fixed stack.
async fn empty_dir<D: FsDriver + ?Sized>(fs: &mut D, top: NodeId) -> FsResult<(), D::DeviceError> {
    let mut stack = [top; MAX_DEPTH];
    let mut depth = 1;
    let mut name = NameBuf::new();
    let result = loop {
        let dir = stack[depth - 1];
        let mut cursor = DirCursor::start();
        let entry = match fs.read_dir_entry(dir, &mut cursor, &mut name).await {
            Ok(entry) => entry,
            Err(err) => break Err(err),
        };
        match entry {
            Some(entry) if entry.file_type().is_dir() => {
                if depth == MAX_DEPTH {
                    break Err(ErrorKind::LimitExceeded.into());
                }
                let Some(child) = name.as_name() else {
                    break Err(ErrorKind::Corrupt.into());
                };
                match fs.lookup(dir, child).await {
                    Ok(node) => {
                        stack[depth] = node;
                        depth += 1;
                    }
                    Err(err) => break Err(err),
                }
            }
            Some(_) => {
                let Some(child) = name.as_name() else {
                    break Err(ErrorKind::Corrupt.into());
                };
                if let Err(err) = fs.remove(dir, child).await {
                    break Err(err);
                }
            }
            None if depth == 1 => break Ok(()),
            None => {
                depth -= 1;
                let parent = stack[depth - 1];
                let found = name_of(fs, parent, dir, &mut name).await;
                fs.forget(dir);
                let removed = match (found, name.as_name()) {
                    (Ok(()), Some(child)) => fs.remove(parent, child).await,
                    (Ok(()), None) => Err(ErrorKind::Corrupt.into()),
                    (Err(err), _) => Err(err),
                };
                if let Err(err) = removed {
                    break Err(err);
                }
            }
        }
    };
    for node in &stack[1..depth] {
        fs.forget(*node);
    }
    result
}

async fn remove_dir_all<D: FsDriver + ?Sized>(fs: &mut D, path: &str) -> FsResult<(), D::DeviceError> {
    let node = fs.resolve(path).await?;
    let meta = fs.node_metadata(node).await;
    let emptied = match meta {
        Ok(meta) if !meta.file_type().is_dir() => Err(ErrorKind::NotADirectory.into()),
        Ok(_) if node == fs.root() => Err(ErrorKind::InvalidInput.into()),
        Ok(_) => empty_dir(fs, node).await,
        Err(err) => Err(err),
    };
    fs.forget(node);
    emptied?;
    remove_checked(fs, path, true).await
}

async fn rename<D: FsDriver + ?Sized>(fs: &mut D, from: &str, to: &str) -> FsResult<(), D::DeviceError> {
    let (from_dir, from_name) = resolve_parent(fs, from).await?;
    let result = match resolve_parent(fs, to).await {
        Ok((to_dir, to_name)) => {
            let result = fs
                .rename(from_dir, from_name, to_dir, to_name, RenameFlags::empty())
                .await;
            fs.forget(to_dir);
            result
        }
        Err(err) => Err(err),
    };
    fs.forget(from_dir);
    result
}

/// Path helpers for the raw tier, on `&mut self`.
///
/// Implemented for every [`FsDriver`]. Paths resolve with the driver's
/// policy ([`Lexical`] unless wrapped in [`WithResolver`]).
pub trait DriverExt: FsDriver {
    /// Resolves paths with `resolver` instead of [`Lexical`].
    fn with_resolver<R: Resolver>(self, resolver: R) -> WithResolver<Self, R>
    where
        Self: Sized,
    {
        WithResolver::new(self, resolver)
    }

    /// Metadata of the node at `path`.
    async fn metadata(&mut self, path: &str) -> FsResult<Metadata, Self::DeviceError> {
        let node = self.resolve(path).await?;
        pinned_metadata(self, node).await
    }

    /// Whether `path` exists.
    async fn exists(&mut self, path: &str) -> FsResult<bool, Self::DeviceError> {
        exists(self, path).await
    }

    /// Opens `path`. The file borrows the driver until it is dropped; use
    /// [`OpenFile`] for several files at once without a [`Volume`].
    async fn open(&mut self, path: &str, opts: OpenOptions) -> FsResult<File<&mut Self>, Self::DeviceError> {
        File::open(self, path, opts).await
    }

    /// Opens the directory at `path`, borrowing the driver.
    async fn read_dir(&mut self, path: &str) -> FsResult<Dir<&mut Self>, Self::DeviceError> {
        Dir::open(self, path).await
    }

    /// Reads a whole file.
    #[cfg(feature = "alloc")]
    async fn read_to_vec(&mut self, path: &str) -> FsResult<alloc::vec::Vec<u8>, Self::DeviceError> {
        read_to_vec(self, path).await
    }

    /// Creates or truncates `path` and writes `data`.
    async fn write_file(&mut self, path: &str, data: &[u8]) -> FsResult<(), Self::DeviceError> {
        write_file(self, path, data).await
    }

    /// Creates `path` and every missing parent.
    async fn create_dir_all(&mut self, path: &str) -> FsResult<(), Self::DeviceError> {
        create_dir_all(self, path).await
    }

    /// Removes the file (or symlink) at `path`.
    async fn remove_file(&mut self, path: &str) -> FsResult<(), Self::DeviceError> {
        remove_checked(self, path, false).await
    }

    /// Removes the empty directory at `path`.
    async fn remove_dir(&mut self, path: &str) -> FsResult<(), Self::DeviceError> {
        remove_checked(self, path, true).await
    }

    /// Removes the directory at `path` and everything in it. Without
    /// allocation, so directories nested more than 64 deep fail with
    /// [`ErrorKind::LimitExceeded`] after removing what they can.
    async fn remove_dir_all(&mut self, path: &str) -> FsResult<(), Self::DeviceError> {
        remove_dir_all(self, path).await
    }

    /// Moves `from` to `to`, keeping the node's identity.
    async fn rename_path(&mut self, from: &str, to: &str) -> FsResult<(), Self::DeviceError> {
        rename(self, from, to).await
    }
}

impl<D: FsDriver + ?Sized> DriverExt for D {}

/// Path helpers for the shared and owned tiers, on `&self`.
///
/// Implemented for every [`FileSystem`]. The names do not overlap with the
/// node methods, so both traits can be in scope.
pub trait PathExt: FileSystem {
    /// Metadata of the node at `path`.
    async fn metadata(&self, path: &str) -> FsResult<Metadata, Self::DeviceError> {
        let mut fs = self;
        let node = FsDriver::resolve(&mut fs, path).await?;
        pinned_metadata(&mut fs, node).await
    }

    /// Whether `path` exists.
    async fn exists(&self, path: &str) -> FsResult<bool, Self::DeviceError> {
        exists(&mut &*self, path).await
    }

    /// Opens `path`. The file borrows `self`; use [`File::open`] with an
    /// `Arc` or `Rc` for an owned handle.
    async fn open(&self, path: &str, opts: OpenOptions) -> FsResult<File<&Self>, Self::DeviceError> {
        File::open(self, path, opts).await
    }

    /// Opens the directory at `path`.
    async fn read_dir(&self, path: &str) -> FsResult<Dir<&Self>, Self::DeviceError> {
        Dir::open(self, path).await
    }

    /// Reads a whole file.
    #[cfg(feature = "alloc")]
    async fn read_to_vec(&self, path: &str) -> FsResult<alloc::vec::Vec<u8>, Self::DeviceError> {
        read_to_vec(&mut &*self, path).await
    }

    /// Creates or truncates `path` and writes `data`.
    async fn write_file(&self, path: &str, data: &[u8]) -> FsResult<(), Self::DeviceError> {
        write_file(&mut &*self, path, data).await
    }

    /// Creates `path` and every missing parent.
    async fn create_dir_all(&self, path: &str) -> FsResult<(), Self::DeviceError> {
        create_dir_all(&mut &*self, path).await
    }

    /// Removes the file (or symlink) at `path`.
    async fn remove_file(&self, path: &str) -> FsResult<(), Self::DeviceError> {
        remove_checked(&mut &*self, path, false).await
    }

    /// Removes the empty directory at `path`.
    async fn remove_dir(&self, path: &str) -> FsResult<(), Self::DeviceError> {
        remove_checked(&mut &*self, path, true).await
    }

    /// Removes the directory at `path` and everything in it.
    async fn remove_dir_all(&self, path: &str) -> FsResult<(), Self::DeviceError> {
        remove_dir_all(&mut &*self, path).await
    }

    /// Moves `from` to `to`, keeping the node's identity.
    async fn rename_path(&self, from: &str, to: &str) -> FsResult<(), Self::DeviceError> {
        rename(&mut &*self, from, to).await
    }
}

impl<F: FileSystem + ?Sized> PathExt for F {}

}
