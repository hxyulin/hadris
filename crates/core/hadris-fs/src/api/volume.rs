use super::handle::{File, ReadDir};
use super::paths::{cancelled, components, resolve_parent, split_parent};
use super::super::lock::{Guard, Lock};
use super::*;
use crate::OpenOptions;
use alloc::collections::VecDeque;
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
    pending: spin::Mutex<VecDeque<Pending>>,
    resolve: Resolve,
}

/// A pin held across an `.await`. Dropped before [`forget`](Self::forget)
/// or [`keep`](Self::keep), as when its future is dropped, it queues the
/// release for the next lock, with a close when `open` is set. Without
/// `pinned` it guards an open only.
pub(super) struct Held<'a> {
    pending: &'a spin::Mutex<VecDeque<Pending>>,
    node: Option<NodeId>,
    open: bool,
    pinned: bool,
}

impl<'a> Held<'a> {
    fn new(pending: &'a spin::Mutex<VecDeque<Pending>>, node: NodeId) -> Self {
        Self { pending, node: Some(node), open: false, pinned: true }
    }

    pub(super) fn node(&self) -> NodeId {
        self.node.expect("released pin")
    }

    /// Hands the pin to the caller.
    pub(super) fn keep(mut self) -> NodeId {
        self.node.take().expect("released pin")
    }

    /// Records that the guarded open was closed, so a drop queues no close.
    pub(super) fn closed(&mut self) {
        self.open = false;
    }

    pub(super) fn forget<F: FileSystem + ?Sized>(mut self, fs: &mut F) {
        if let Some(node) = self.node.take()
            && self.pinned
        {
            fs.forget(node, 1);
        }
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Some(node) = self.node.take() {
            let mut pending = self.pending.lock();
            if self.open {
                pending.push_back(Pending::Close(node));
            }
            if self.pinned {
                pending.push_back(Pending::Forget(node));
            }
        }
    }
}

io_transform! {

/// A filesystem shared between handles and tasks, with paths and handles
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
/// ```rust,no_run
/// use hadris_fs::sync::{FileSystem, Volume};
/// use hadris_fs::{FsResult, OpenOptions};
/// # fn example<F: FileSystem>(fs: F) -> FsResult<(), F::DeviceError> {
/// let vol = Volume::new(fs);
/// let mut log = vol.open("/log.txt", OpenOptions::new().write().create().append())?;
/// log.write(b"hello\n")?;
/// log.close()?;
/// # Ok(())
/// # }
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
    pending: &'a spin::Mutex<VecDeque<Pending>>,
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
/// ignored, as in the `Drop` that queued them. A call leaves the queue as it
/// starts, so a dropped future leaves the rest queued.
async fn run_pending<F: FileSystem + ?Sized>(pending: &spin::Mutex<VecDeque<Pending>>, fs: &mut F) {
    loop {
        let Some(call) = pending.lock().pop_front() else {
            break;
        };
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
                pending: spin::Mutex::new(VecDeque::new()),
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
            pending.push_back(Pending::Close(node));
        }
        pending.push_back(Pending::Forget(node));
    }

    /// Guards the pin of `node` while a call on this volume awaits.
    pub(super) fn hold(&self, node: NodeId) -> Held<'_> {
        Held::new(&self.shared.pending, node)
    }

    send_only! {
    /// Guards an open of `node`, whose pin its caller keeps, while a call on
    /// this volume awaits.
    pub(super) fn hold_open(&self, node: NodeId) -> Held<'_> {
        Held { pending: &self.shared.pending, node: Some(node), open: true, pinned: false }
    }

    }

    /// Guards both the open and the pin of `node` while a call on this
    /// volume awaits.
    pub(super) fn hold_file(&self, node: NodeId) -> Held<'_> {
        Held { pending: &self.shared.pending, node: Some(node), open: true, pinned: true }
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
        let mut node = if options.is_create() || options.is_create_new() {
            let (dir, name) = resolve_parent(&mut *fs, path, how).await?;
            let dir = self.hold(dir);
            let found = match fs.lookup(dir.node(), name).await {
                Ok(node) if options.is_create_new() => {
                    fs.forget(node, 1);
                    Err(ErrorKind::AlreadyExists.into())
                }
                Err(err) if err.kind() == ErrorKind::NotFound => fs.create(dir.node(), name, &SetAttr::new()).await,
                other => other,
            };
            dir.forget(&mut *fs);
            let node = self.hold(found?);
            if how == Resolve::Follow && is_symlink(&mut *fs, node.node()).await {
                node.forget(&mut *fs);
                self.hold(fs.resolve(path, how).await?)
            } else {
                node
            }
        } else {
            self.hold(fs.resolve(path, how).await?)
        };
        let mode = if options.is_write() { OpenMode::Write } else { OpenMode::Read };
        if let Err(err) = fs.open(node.node(), mode).await {
            node.forget(&mut *fs);
            return Err(err);
        }
        node.open = true;
        if options.is_truncate() && !options.is_create_new() {
            let emptied = match fs.stat(node.node()).await {
                Ok(meta) if meta.len() > 0 => fs.truncate(node.node(), 0).await,
                Ok(_) => Ok(()),
                Err(err) => Err(err),
            };
            if let Err(err) = emptied {
                let _ = fs.close(node.node()).await;
                node.closed();
                node.forget(&mut *fs);
                return Err(err);
            }
        }
        let node = node.keep();
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
        let node = self.hold(fs.resolve(path, how).await?);
        let meta = fs.stat(node.node()).await;
        node.forget(&mut *fs);
        meta
    }

    /// Lists the directory at `path`.
    pub async fn read_dir(&self, path: impl AsRef<[u8]>) -> FsResult<ReadDir<F>, F::DeviceError> {
        let mut fs = self.lock().await;
        let node = self.hold(fs.resolve(path.as_ref(), self.shared.resolve).await?);
        match fs.stat(node.node()).await {
            Ok(meta) if meta.file_type().is_dir() => {
                let node = node.keep();
                drop(fs);
                Ok(ReadDir::new(self.clone(), node))
            }
            other => {
                node.forget(&mut *fs);
                Err(other.err().unwrap_or_else(|| ErrorKind::NotADirectory.into()))
            }
        }
    }

    /// The target of the symlink at `path`.
    pub async fn read_link(&self, path: impl AsRef<[u8]>) -> FsResult<Vec<u8>, F::DeviceError> {
        let mut fs = self.lock().await;
        let node = self.hold(fs.resolve(path.as_ref(), no_follow(self.shared.resolve)).await?);
        let target = read_link_of(&mut *fs, node.node()).await;
        node.forget(&mut *fs);
        target
    }

    /// Creates the directory at `path`.
    pub async fn create_dir(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
        let dir = self.hold(dir);
        let made = fs.mkdir(dir.node(), name, &SetAttr::new()).await;
        dir.forget(&mut *fs);
        fs.forget(made?, 1);
        Ok(())
    }

    /// Creates the directory at `path` and every missing parent.
    pub async fn create_dir_all(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        create_dir_all(&mut *fs, &self.shared.pending, path.as_ref(), self.shared.resolve).await
    }

    /// Removes the file or symlink at `path`.
    pub async fn remove_file(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
        let dir = self.hold(dir);
        let removed = fs.unlink(dir.node(), name).await;
        dir.forget(&mut *fs);
        removed
    }

    /// Removes the empty directory at `path`.
    pub async fn remove_dir(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
        let dir = self.hold(dir);
        let removed = fs.rmdir(dir.node(), name).await;
        dir.forget(&mut *fs);
        removed
    }

    /// Removes the directory at `path` and everything in it. A symlink at
    /// `path` fails with [`ErrorKind::NotADirectory`] and is not followed,
    /// and a path ending in `.` or `..` fails with
    /// [`ErrorKind::InvalidInput`] before anything is removed.
    pub async fn remove_dir_all(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let path = path.as_ref();
        let how = self.shared.resolve;
        split_parent(path)?;
        let mut fs = self.lock().await;
        let node = self.hold(fs.resolve(path, no_follow(how)).await?);
        let emptied = match fs.stat(node.node()).await {
            Ok(meta) if !meta.file_type().is_dir() => Err(ErrorKind::NotADirectory.into()),
            Ok(_) if node.node() == fs.root() => Err(ErrorKind::InvalidInput.into()),
            Ok(_) => empty_dir(&mut *fs, &self.shared.pending, node.node()).await,
            Err(err) => Err(err),
        };
        node.forget(&mut *fs);
        emptied?;
        let (dir, name) = resolve_parent(&mut *fs, path, how).await?;
        let dir = self.hold(dir);
        let removed = fs.rmdir(dir.node(), name).await;
        dir.forget(&mut *fs);
        removed
    }

    /// Moves `from` to `to`, replacing an existing `to` as
    /// `std::fs::rename` does. The node keeps its id.
    pub async fn rename(&self, from: impl AsRef<[u8]>, to: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        let how = self.shared.resolve;
        let mut fs = self.lock().await;
        let (from_dir, from_name) = resolve_parent(&mut *fs, from.as_ref(), how).await?;
        let from_dir = self.hold(from_dir);
        let moved = match resolve_parent(&mut *fs, to.as_ref(), how).await {
            Ok((to_dir, to_name)) => {
                let to_dir = self.hold(to_dir);
                let moved = fs
                    .rename(from_dir.node(), from_name, to_dir.node(), to_name, RenameMode::Replace)
                    .await;
                to_dir.forget(&mut *fs);
                moved
            }
            Err(err) => Err(err),
        };
        from_dir.forget(&mut *fs);
        moved
    }

    /// Changes the times, permissions, owner or attributes of the node at
    /// `path`.
    pub async fn set_attr(&self, path: impl AsRef<[u8]>, changes: &SetAttr) -> FsResult<(), F::DeviceError> {
        let mut fs = self.lock().await;
        let node = self.hold(fs.resolve(path.as_ref(), self.shared.resolve).await?);
        let set = fs.setattr(node.node(), changes).await;
        node.forget(&mut *fs);
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
async fn empty_dir<F: FileSystem + ?Sized>(
    fs: &mut F,
    pending: &spin::Mutex<VecDeque<Pending>>,
    top: NodeId,
) -> FsResult<(), F::DeviceError> {
    let mut stack: Vec<(Held<'_>, Vec<u8>)> = Vec::new();
    let result = loop {
        let dir = stack.last().map_or(top, |(node, _)| node.node());
        let entry = match fs.readdir(dir, DirCursor::START).await {
            Ok(entry) => entry,
            Err(err) => break Err(err),
        };
        match entry {
            Some(entry) if entry.file_type().is_dir() => match fs.lookup(dir, entry.name()).await {
                Ok(child) if child == top || stack.iter().any(|(node, _)| node.node() == child) => {
                    fs.forget(child, 1);
                    break Err(ErrorKind::Corrupt.into());
                }
                Ok(child) => stack.push((Held::new(pending, child), entry.name().as_bytes().to_vec())),
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
                let parent = stack.last().map_or(top, |(node, _)| node.node());
                let removed = fs.rmdir(parent, Name::new(&name)).await;
                node.forget(fs);
                if let Err(err) = removed {
                    break Err(err);
                }
            }
        }
    };
    for (node, _) in stack {
        node.forget(fs);
    }
    result
}

/// Creates the directory `path` and every missing parent, resolving `..`
/// as `how` does.
async fn create_dir_all<F: FileSystem + ?Sized>(
    fs: &mut F,
    pending: &spin::Mutex<VecDeque<Pending>>,
    path: &[u8],
    how: Resolve,
) -> FsResult<(), F::DeviceError> {
    let mut current = Held::new(pending, fs.root());
    let mut rest = components(path);
    let mut result = Ok(());
    while let Some(component) = rest.next() {
        if component == b"." {
            continue;
        }
        let next = if component == b".." {
            if how == Resolve::Lexical {
                continue;
            }
            fs.parent(current.node()).await
        } else if how == Resolve::Lexical && cancelled(rest.clone()) {
            continue;
        } else {
            let name = Name::new(component);
            match fs.lookup(current.node(), name).await {
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    fs.mkdir(current.node(), name, &SetAttr::new()).await
                }
                other => other,
            }
        };
        current.forget(fs);
        current = Held::new(pending, next?);
        match fs.stat(current.node()).await {
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
    current.forget(fs);
    result
}

}
