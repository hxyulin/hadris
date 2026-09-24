use super::handle::{File, ReadDir};
use super::paths::{create_dir_all, resolve_parent};
use super::super::lock::{Guard, Lock};
use super::*;
use crate::OpenOptions;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// A call a dropped handle could not make, waiting for the next lock.
#[derive(Debug, Clone, Copy)]
pub(super) enum Pending {
    Close(NodeId),
    Forget(NodeId),
}

/// `how` for the last component of ``symlink_metadata`, `read_link` and
/// `remove_dir_all`: a
/// symlink there is returned, not followed.
const fn no_follow(how: Resolve) -> Resolve {
    match how {
        Resolve::Follow => Resolve::NoFollow,
        other => other,
    }
}

pub(super) struct Shared<F> {
    fs: Lock<F>,
    pending: spin::Mutex<Vec<Pending>>,
    resolve: Resolve,
}

io_transform! {

/// A filesystem shared between threads and tasks, with paths and handles
/// named after `std::fs`.
///
/// Clones are cheap and share the volume. Each call locks the filesystem
/// once, and a path resolves under that one lock hold, so calls on one
/// volume serialize. The sync `Volume` locks a `std::sync::Mutex` and needs
/// `std`; the async one locks an async mutex and needs only `alloc`.
///
/// Paths are `/`-separated bytes from the root and resolve with the
/// [`Resolve`] policy given to [`with_resolve`](Self::with_resolve),
/// [`Resolve::Lexical`] for [`new`](Self::new).
///
/// ```rust,ignore
/// let vol = Volume::new(fs);
/// let mut log = vol.open("/log.txt", OpenOptions::new().write().create().append())?;
/// log.write(b"hello\n")?;
/// log.close()?;
/// ```
pub struct Volume<F> {
    shared: Arc<Shared<F>>,
}

/// The filesystem of a [`Volume`], locked by [`Volume::lock`].
///
/// Dereferences to the filesystem, for node calls and format-specific
/// methods.
pub struct VolumeGuard<'a, F: FileSystem> {
    fs: Guard<'a, F>,
    #[allow(dead_code)]
    pending: &'a spin::Mutex<Vec<Pending>>,
}

impl<F> Clone for Volume<F> {
    fn clone(&self) -> Self {
        Self { shared: Arc::clone(&self.shared) }
    }
}

impl<F> core::fmt::Debug for Volume<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Volume").field("resolve", &self.shared.resolve).finish_non_exhaustive()
    }
}

impl<F: FileSystem> core::ops::Deref for VolumeGuard<'_, F> {
    type Target = F;

    fn deref(&self) -> &F {
        &self.fs
    }
}

impl<F: FileSystem> core::ops::DerefMut for VolumeGuard<'_, F> {
    fn deref_mut(&mut self) -> &mut F {
        &mut self.fs
    }
}

impl<F: FileSystem> Drop for VolumeGuard<'_, F> {
    fn drop(&mut self) {
        sync_only! {
            run_pending(self.pending, &mut *self.fs);
        }
    }
}

/// Runs the calls dropped handles queued, in order. Close errors are
/// ignored, as in the `Drop` that queued them.
async fn run_pending<F: FileSystem + ?Sized>(pending: &spin::Mutex<Vec<Pending>>, fs: &mut F) {
    let queued = core::mem::take(&mut *pending.lock());
    for call in queued {
        match call {
            Pending::Close(node) => {
                let _ = fs.close(node).await;
            }
            Pending::Forget(node) => fs.forget(node, 1),
        }
    }
}

impl<F: FileSystem> Volume<F> {
    /// Shares `fs`. Paths resolve lexically.
    pub fn new(fs: F) -> Self {
        Self::with_resolve(fs, Resolve::Lexical)
    }

    /// Shares `fs`, resolving every path with `resolve`.
    pub fn with_resolve(fs: F, resolve: Resolve) -> Self {
        Self {
            shared: Arc::new(Shared {
                fs: Lock::new(fs),
                pending: spin::Mutex::new(Vec::new()),
                resolve,
            }),
        }
    }

    /// Locks the filesystem for node calls and format-specific methods.
    ///
    /// Dropping a [`File`] or [`ReadDir`] of this volume while holding the
    /// guard queues its close and does not deadlock. Calling a path method
    /// on this volume while holding the guard deadlocks, as with any mutex.
    pub async fn lock(&self) -> VolumeGuard<'_, F> {
        let mut fs = self.shared.fs.lock().await;
        run_pending(&self.shared.pending, &mut *fs).await;
        VolumeGuard { fs, pending: &self.shared.pending }
    }

    /// Returns the filesystem, after the calls dropped handles queued, or
    /// the volume while clones or handles of it exist.
    pub async fn into_inner(self) -> Result<F, Self> {
        match Arc::try_unwrap(self.shared) {
            Ok(shared) => {
                let mut fs = shared.fs.into_inner();
                run_pending(&shared.pending, &mut fs).await;
                Ok(fs)
            }
            Err(shared) => Err(Self { shared }),
        }
    }

    /// Makes the call a dropped handle owes on `node`: forgets it, after
    /// closing it when `close` is set. Runs now when the lock is free and
    /// the call needs no `.await`, and queues it for the next lock
    /// otherwise.
    pub(super) fn release(&self, node: NodeId, close: bool) {
        sync_only! {
            if let Some(mut fs) = self.shared.fs.try_lock() {
                run_pending(&self.shared.pending, &mut *fs);
                if close {
                    let _ = fs.close(node);
                }
                fs.forget(node, 1);
                return;
            }
        }
        async_only! {
            if !close
                && let Some(mut fs) = self.shared.fs.try_lock()
                && self.shared.pending.lock().is_empty()
            {
                fs.forget(node, 1);
                return;
            }
        }
        let mut pending = self.shared.pending.lock();
        if close {
            pending.push(Pending::Close(node));
        }
        pending.push(Pending::Forget(node));
    }

    /// Opens the file at `path` with `options`.
    ///
    /// A write open fails with [`ErrorKind::ReadOnly`] before anything is
    /// created or truncated, a directory with [`ErrorKind::IsADirectory`],
    /// and a symlink the volume's policy does not follow with
    /// [`ErrorKind::Symlink`].
    pub async fn open(&self, path: impl AsRef<[u8]>, options: OpenOptions) -> FsResult<File<F>, F::DeviceError> {
        options.validate()?;
        let how = self.shared.resolve;
        let path = path.as_ref();
        let mut fs = self.lock().await;
        if options.is_write() && !fs.capabilities().writable() {
            return Err(ErrorKind::ReadOnly.into());
        }
        let node = if options.is_create() || options.is_create_new() {
            let (dir, name) = resolve_parent(&mut *fs, path, how).await?;
            let found = match fs.lookup(dir, name).await {
                Ok(node) if options.is_create_new() => {
                    fs.forget(node, 1);
                    Err(ErrorKind::AlreadyExists.into())
                }
                Err(err) if err.kind() == ErrorKind::NotFound => fs.create(dir, name, &SetAttr::new()).await,
                other => other,
            };
            fs.forget(dir, 1);
            let node = found?;
            if how == Resolve::Follow && is_symlink(&mut *fs, node).await {
                fs.forget(node, 1);
                fs.resolve(path, how).await?
            } else {
                node
            }
        } else {
            fs.resolve(path, how).await?
        };
        let mode = if options.is_write() { OpenMode::Write } else { OpenMode::Read };
        if let Err(err) = fs.open(node, mode).await {
            fs.forget(node, 1);
            return Err(err);
        }
        if options.is_truncate() && !options.is_create_new() {
            let emptied = match fs.stat(node).await {
                Ok(meta) if meta.len() > 0 => fs.truncate(node, 0).await,
                Ok(_) => Ok(()),
                Err(err) => Err(err),
            };
            if let Err(err) = emptied {
                let _ = fs.close(node).await;
                fs.forget(node, 1);
                return Err(err);
            }
        }
        drop(fs);
        Ok(File::new(self.clone(), node, options))
    }

    /// Metadata of the node at `path`, following a final symlink when the
    /// volume's policy does.
    pub async fn metadata(&self, path: impl AsRef<[u8]>) -> FsResult<Metadata, F::DeviceError> {
        self.stat_path(path.as_ref(), self.shared.resolve).await
    }

    /// Metadata of the node at `path`, never following a final symlink.
    pub async fn symlink_metadata(&self, path: impl AsRef<[u8]>) -> FsResult<Metadata, F::DeviceError> {
        self.stat_path(path.as_ref(), no_follow(self.shared.resolve)).await
    }

    async fn stat_path(&self, path: &[u8], how: Resolve) -> FsResult<Metadata, F::DeviceError> {
        let mut fs = self.lock().await;
        let node = fs.resolve(path, how).await?;
        let meta = fs.stat(node).await;
        fs.forget(node, 1);
        meta
    }

    /// Lists the directory at `path`.
    pub async fn read_dir(&self, path: impl AsRef<[u8]>) -> FsResult<ReadDir<F>, F::DeviceError> {
        let mut fs = self.lock().await;
        let node = fs.resolve(path.as_ref(), self.shared.resolve).await?;
        match fs.stat(node).await {
            Ok(meta) if meta.file_type().is_dir() => {
                drop(fs);
                Ok(ReadDir::new(self.clone(), node))
            }
            other => {
                fs.forget(node, 1);
                Err(other.err().unwrap_or_else(|| ErrorKind::NotADirectory.into()))
            }
        }
    }

    /// The target of the symlink at `path`.
    pub async fn read_link(&self, path: impl AsRef<[u8]>) -> FsResult<Vec<u8>, F::DeviceError> {
        let mut fs = self.lock().await;
        let node = fs.resolve(path.as_ref(), no_follow(self.shared.resolve)).await?;
        let target = read_link_of(&mut *fs, node).await;
        fs.forget(node, 1);
        target
    }

    /// Creates the directory at `path`.
    pub async fn create_dir(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
        let made = fs.mkdir(dir, name, &SetAttr::new()).await;
        fs.forget(dir, 1);
        fs.forget(made?, 1);
        Ok(())
    }

    /// Creates the directory at `path` and every missing parent.
    pub async fn create_dir_all(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        create_dir_all(&mut *fs, path.as_ref(), self.shared.resolve).await
    }

    /// Removes the file or symlink at `path`.
    pub async fn remove_file(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
        let removed = fs.unlink(dir, name).await;
        fs.forget(dir, 1);
        removed
    }

    /// Removes the empty directory at `path`.
    pub async fn remove_dir(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
        let removed = fs.rmdir(dir, name).await;
        fs.forget(dir, 1);
        removed
    }

    /// Removes the directory at `path` and everything in it. A symlink at
    /// `path` fails with [`ErrorKind::NotADirectory`] and is not followed.
    pub async fn remove_dir_all(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let path = path.as_ref();
        let how = self.shared.resolve;
        let mut fs = self.lock().await;
        let node = fs.resolve(path, no_follow(how)).await?;
        let emptied = match fs.stat(node).await {
            Ok(meta) if !meta.file_type().is_dir() => Err(ErrorKind::NotADirectory.into()),
            Ok(_) if node == fs.root() => Err(ErrorKind::InvalidInput.into()),
            Ok(_) => empty_dir(&mut *fs, node).await,
            Err(err) => Err(err),
        };
        fs.forget(node, 1);
        emptied?;
        let (dir, name) = resolve_parent(&mut *fs, path, how).await?;
        let removed = fs.rmdir(dir, name).await;
        fs.forget(dir, 1);
        removed
    }

    /// Moves `from` to `to`, replacing an existing `to` as
    /// `std::fs::rename` does. The node keeps its id.
    pub async fn rename(&self, from: impl AsRef<[u8]>, to: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let how = self.shared.resolve;
        let mut fs = self.lock().await;
        let (from_dir, from_name) = resolve_parent(&mut *fs, from.as_ref(), how).await?;
        let moved = match resolve_parent(&mut *fs, to.as_ref(), how).await {
            Ok((to_dir, to_name)) => {
                let moved = fs.rename(from_dir, from_name, to_dir, to_name, RenameMode::Replace).await;
                fs.forget(to_dir, 1);
                moved
            }
            Err(err) => Err(err),
        };
        fs.forget(from_dir, 1);
        moved
    }

    /// Changes the times, permissions, owner or attributes of the node at
    /// `path`.
    pub async fn set_attr(&self, path: impl AsRef<[u8]>, changes: &SetAttr) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let node = fs.resolve(path.as_ref(), self.shared.resolve).await?;
        let set = fs.setattr(node, changes).await;
        fs.forget(node, 1);
        set
    }
}

async fn is_symlink<F: FileSystem + ?Sized>(fs: &mut F, node: NodeId) -> bool {
    matches!(fs.stat(node).await, Ok(meta) if meta.file_type().is_symlink())
}

/// Reads the target of the symlink `node` into a buffer of its length.
async fn read_link_of<F: FileSystem + ?Sized>(fs: &mut F, node: NodeId) -> FsResult<Vec<u8>, F::DeviceError> {
    let meta = fs.stat(node).await?;
    if !meta.file_type().is_symlink() {
        return Err(ErrorKind::InvalidInput.into());
    }
    let len = usize::try_from(meta.len()).map_err(|_| ErrorKind::LimitExceeded)?;
    let mut buf = alloc::vec![0u8; len.max(1)];
    let n = fs.readlink(node, &mut buf).await?.len();
    buf.truncate(n);
    Ok(buf)
}

/// Empties the directory `top`, depth first.
async fn empty_dir<F: FileSystem + ?Sized>(fs: &mut F, top: NodeId) -> FsResult<(), F::DeviceError> {
    let mut stack: Vec<(NodeId, Vec<u8>)> = Vec::new();
    let result = loop {
        let dir = stack.last().map_or(top, |(node, _)| *node);
        let entry = match fs.readdir(dir, DirCursor::START).await {
            Ok(entry) => entry,
            Err(err) => break Err(err),
        };
        match entry {
            Some(entry) if entry.file_type().is_dir() => match fs.lookup(dir, entry.name()).await {
                Ok(child) if child == top || stack.iter().any(|(node, _)| *node == child) => {
                    fs.forget(child, 1);
                    break Err(ErrorKind::Corrupt.into());
                }
                Ok(child) => stack.push((child, entry.name().as_bytes().to_vec())),
                Err(err) => break Err(err),
            },
            Some(entry) => {
                if let Err(err) = fs.unlink(dir, entry.name()).await {
                    break Err(err);
                }
            }
            None => {
                let Some((node, name)) = stack.pop() else {
                    break Ok(());
                };
                let parent = stack.last().map_or(top, |(node, _)| *node);
                let removed = fs.rmdir(parent, Name::new(&name)).await;
                fs.forget(node, 1);
                if let Err(err) = removed {
                    break Err(err);
                }
            }
        }
    };
    for (node, _) in stack {
        fs.forget(node, 1);
    }
    result
}

}
