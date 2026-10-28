use super::super::lock::{Guard, Lock};
use super::filesystem::FileSystem;
use super::handle::{File, ReadDir, WalkDir};
use super::paths::{cancelled, components, resolve_parent, split_parent, write_all_at};
use crate::{
    DirCursor, DirEntry, ErrorKind, FsResult, Metadata, Name, NodeId, OpenMode, OpenOptions,
    RenameMode, Resolve, SetAttr,
};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

/// A call a dropped handle could not make, waiting for the next lock.
#[derive(Debug, Clone, Copy)]
pub(super) enum Pending {
    Close(NodeId),
    Forget(NodeId),
}

/// `how` for the last component of `symlink_metadata`, `read_link` and
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
    failed: AtomicBool,
}

/// Marks a volume failed when dropped, as by an unwind out of a poll.
#[allow(dead_code)]
struct Unwinding<'a>(&'a AtomicBool);

impl Drop for Unwinding<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
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
        Self {
            pending,
            node: Some(node),
            open: false,
            pinned: true,
        }
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
/// # Panics during a call
///
/// A panic while the filesystem is locked, in a driver, a device or the
/// caller's own code holding a [`VolumeGuard`], may leave the driver
/// mid-change, so the volume fails for good. Every later path or handle
/// call returns [`Detail::VolumeFailed`](crate::Detail::VolumeFailed) with
/// kind [`ErrorKind::Io`], without reaching the driver, and calls that
/// dropped handles queued are discarded. [`is_failed`](Self::is_failed)
/// tells, and [`into_inner`](Self::into_inner) still gives the filesystem
/// back, whose state may then be inconsistent.
///
/// The sync volume detects every such panic by the poisoning of its
/// `std::sync::Mutex`. The async volume detects a panic out of a poll of its
/// own calls and handle calls, also without `std`, and with `std` any panic
/// that drops a [`VolumeGuard`] while unwinding.
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
    #[allow(dead_code)]
    failed: &'a AtomicBool,
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

impl<F: FileSystem> core::fmt::Debug for VolumeGuard<'_, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VolumeGuard").finish_non_exhaustive()
    }
}

impl<F: FileSystem> core::ops::DerefMut for VolumeGuard<'_, F> {
    fn deref_mut(&mut self) -> &mut F {
        &mut self.fs
    }
}

impl<F: FileSystem> Drop for VolumeGuard<'_, F> {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        if std::thread::panicking() {
            self.failed.store(true, Ordering::Release);
        }
        sync_only! {
            if !self.failed.load(Ordering::Acquire) {
                run_pending(self.pending, &mut *self.fs);
            }
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
                failed: AtomicBool::new(false),
            }),
        }
    }

    /// Locks the filesystem for node calls and format-specific methods.
    ///
    /// Dropping a [`File`] or [`ReadDir`] of this volume while holding the
    /// guard queues its close and does not deadlock. Calling a path method
    /// on this volume while holding the guard deadlocks, as with any mutex.
    ///
    /// # Errors
    ///
    /// [`Detail::VolumeFailed`](crate::Detail::VolumeFailed) when the volume
    /// [failed](Self::is_failed). Take the filesystem back with
    /// [`into_inner`](Self::into_inner) to inspect or discard it.
    ///
    /// # Errors
    ///
    /// Returns an I/O error with [`Detail::VolumeFailed`](crate::Detail::VolumeFailed)
    /// if a previous panic left the volume failed.
    pub async fn lock(&self) -> FsResult<VolumeGuard<'_, F>, F::DeviceError> {
        watched!(self, { self.enter().await })
    }

    /// Whether a panic during a call failed the volume, so every later
    /// call returns [`Detail::VolumeFailed`](crate::Detail::VolumeFailed).
    pub fn is_failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire) || self.shared.fs.poisoned()
    }

    /// Locks the filesystem for a call, or fails when the volume failed.
    pub(super) async fn enter(&self) -> FsResult<VolumeGuard<'_, F>, F::DeviceError> {
        if self.is_failed() {
            return Err(crate::Detail::VolumeFailed.error());
        }
        let Some(mut fs) = self.shared.fs.lock().await else {
            return Err(crate::Detail::VolumeFailed.error());
        };
        if self.shared.failed.load(Ordering::Acquire) {
            return Err(crate::Detail::VolumeFailed.error());
        }
        run_pending(&self.shared.pending, &mut *fs).await;
        Ok(VolumeGuard { fs, pending: &self.shared.pending, failed: &self.shared.failed })
    }

    /// Returns the filesystem, after the calls dropped handles queued, or
    /// the volume while clones or handles of it exist.
    ///
    /// A [failed](Self::is_failed) volume gives its filesystem back without
    /// making the queued calls. Its state may be inconsistent, so check it
    /// or discard it rather than writing through it.
    ///
    /// # Errors
    ///
    /// Returns the volume itself while other clones or handles still exist.
    pub async fn into_inner(self) -> Result<F, Self> {
        match Arc::try_unwrap(self.shared) {
            Ok(shared) => {
                let failed = shared.failed.load(Ordering::Acquire) || shared.fs.poisoned();
                let mut fs = shared.fs.into_inner();
                if !failed {
                    run_pending(&shared.pending, &mut fs).await;
                }
                Ok(fs)
            }
            Err(shared) => Err(Self { shared }),
        }
    }

    async_only! {
    /// Polls `call`, marking the volume failed if a poll of it unwinds. A
    /// call dropped between polls, as by a cancelled future, is not a
    /// failure.
    pub(super) async fn watch<T>(&self, call: impl core::future::Future<Output = T>) -> T {
        let mut call = core::pin::pin!(call);
        core::future::poll_fn(|cx| {
            let armed = Unwinding(&self.shared.failed);
            let polled = call.as_mut().poll(cx);
            core::mem::forget(armed);
            polled
        })
        .await
    }
    }

    /// Makes the call a dropped handle owes on `node`: forgets it, after
    /// closing it when `close` is set. Runs now when the lock is free and
    /// the call needs no `.await`, and queues it for the next lock
    /// otherwise. A failed volume drops the call.
    pub(super) fn release(&self, node: NodeId, close: bool) {
        if self.is_failed() {
            return;
        }
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

    /// Guards an open of `node`, whose pin its caller keeps, while a call on
    /// this volume awaits.
    pub(super) fn hold_open(&self, node: NodeId) -> Held<'_> {
        Held { pending: &self.shared.pending, node: Some(node), open: true, pinned: false }
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
    ///
    /// # Errors
    ///
    /// Returns an error for invalid options, path resolution failures, or
    /// refused access. Creating an existing file with `create_new` returns
    /// [`ErrorKind::AlreadyExists`]. Filesystem and device errors propagate;
    /// creation or truncation may have happened before a later failure.
    pub async fn open(&self, path: impl AsRef<[u8]>, options: OpenOptions) -> FsResult<File<F>, F::DeviceError> {
        watched!(self, {
            options.validate()?;
            let how = self.shared.resolve;
            let path = path.as_ref();
            let mut fs = self.enter().await?;
            if options.is_write() && !fs.capabilities().writable() {
                return Err(ErrorKind::ReadOnly.into());
            }
            let node = if options.is_create() || options.is_create_new() {
                let (dir, name) = resolve_parent(&mut *fs, path, how).await?;
                let dir = self.hold(dir);
                let found = match fs.lookup(dir.node(), name).await {
                    Ok(node) if options.is_create_new() => {
                        fs.forget(node, 1);
                        Err(ErrorKind::AlreadyExists.into())
                    }
                    Err(err) if err.kind() == ErrorKind::NotFound => fs.create(dir.node(), name, options.attr()).await,
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
            let node = open_held(&mut *fs, node, options).await?;
            drop(fs);
            Ok(File::new(self.clone(), node, options))
        })
    }

    /// Opens the file `entry` names in the directory `dir`, as `readdir`
    /// listed it, without resolving a path. `dir` is pinned, as the
    /// directory of a [`ReadDir`] or the root is, or was listed by this
    /// volume and has not changed since, as the
    /// [`dir`](super::WalkDirEntry::dir) of a [`WalkDir`] entry.
    ///
    /// The file is pinned by looking `entry`'s name up in `dir`, so the
    /// lookup rules of the format apply; a name that no longer leads to
    /// `entry`'s node, as after the directory changed, fails with
    /// [`ErrorKind::NotFound`]. A symlink is not followed and fails with
    /// [`ErrorKind::Symlink`]. `create` changes nothing, `create_new` fails
    /// with [`ErrorKind::AlreadyExists`], and the other options apply as for
    /// [`open`](Self::open).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`] if the entry no longer names the same
    /// node, or [`ErrorKind::AlreadyExists`] when `create_new` is set.
    /// Invalid options, refused access, and filesystem or device failures
    /// also return errors.
    pub async fn open_node(
        &self,
        dir: NodeId,
        entry: &DirEntry,
        options: OpenOptions,
    ) -> FsResult<File<F>, F::DeviceError> {
        watched!(self, {
            options.validate()?;
            if options.is_create_new() {
                return Err(ErrorKind::AlreadyExists.into());
            }
            let mut fs = self.enter().await?;
            if options.is_write() && !fs.capabilities().writable() {
                return Err(ErrorKind::ReadOnly.into());
            }
            let node = self.hold(fs.lookup(dir, entry.name()).await?);
            if node.node() != entry.node() {
                node.forget(&mut *fs);
                return Err(ErrorKind::NotFound.into());
            }
            let node = open_held(&mut *fs, node, options).await?;
            drop(fs);
            Ok(File::new(self.clone(), node, options))
        })
    }

    /// Reads the whole file at `path`, as `std::fs::read`.
    ///
    /// # Errors
    ///
    /// Propagates errors from opening, reading, or closing the file.
    /// Bytes already read are discarded if a later operation fails.
    pub async fn read(&self, path: impl AsRef<[u8]>) -> FsResult<Vec<u8>, F::DeviceError> {
        watched!(self, {
            let file = self.open(path, OpenOptions::new().read()).await?;
            let mut fs = self.enter().await?;
            let read = read_all(&mut *fs, file.node()).await;
            drop(fs);
            let closed = file.close().await;
            let data = read?;
            closed?;
            Ok(data)
        })
    }

    /// Writes `contents` as the whole file at `path`, creating it or
    /// truncating it first, as `std::fs::write`.
    ///
    /// # Errors
    ///
    /// Propagates errors from opening, writing, or closing the file.
    /// Failure may leave the file created, truncated, or partly written.
    pub async fn write(&self, path: impl AsRef<[u8]>, contents: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let file = self.open(path, OpenOptions::new().write().create().truncate()).await?;
            let mut fs = self.enter().await?;
            let written = write_all_at(&mut *fs, file.node(), 0, contents.as_ref()).await;
            drop(fs);
            let closed = file.close().await;
            written?;
            closed
        })
    }

    /// Whether `path` names a node, following a final symlink when the
    /// volume's policy does, as `std::fs::exists`: `Ok(false)` when it is
    /// not found, and other errors as they are.
    ///
    /// # Errors
    ///
    /// Returns `false` for [`ErrorKind::NotFound`]. Other errors from path
    /// resolution or metadata access are propagated.
    pub async fn exists(&self, path: impl AsRef<[u8]>) -> FsResult<bool, F::DeviceError> {
        match self.metadata(path).await {
            Ok(_) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Walks everything below the directory at `path`, depth first, each
    /// directory before its children, yielding every entry with its path.
    ///
    /// A directory that leads back to one of its ancestors, which only a
    /// corrupt volume holds, ends the walk with [`ErrorKind::Corrupt`].
    /// Each step locks the volume once; see [`WalkDir`] for changes made
    /// during the walk.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotADirectory`] if the path is not a directory.
    /// Propagates path resolution and metadata errors. Later traversal
    /// errors are returned by [`WalkDir::next_entry`].
    pub async fn walk(&self, path: impl AsRef<[u8]>) -> FsResult<WalkDir<F>, F::DeviceError> {
        watched!(self, {
            let path = path.as_ref();
            let mut fs = self.enter().await?;
            let node = self.hold(fs.resolve(path, self.shared.resolve).await?);
            match fs.stat(node.node()).await {
                Ok(meta) if meta.file_type().is_dir() => {
                    let node = node.keep();
                    drop(fs);
                    Ok(WalkDir::new(self.clone(), node, path))
                }
                other => {
                    node.forget(&mut *fs);
                    Err(other.err().unwrap_or_else(|| ErrorKind::NotADirectory.into()))
                }
            }
        })
    }

    /// Metadata of the node at `path`, following a final symlink when the
    /// volume's policy does.
    ///
    /// # Errors
    ///
    /// Propagates errors from path resolution and [`FileSystem::stat`].
    pub async fn metadata(&self, path: impl AsRef<[u8]>) -> FsResult<Metadata, F::DeviceError> {
        self.stat_path(path.as_ref(), self.shared.resolve).await
    }

    /// Metadata of the node at `path`, never following a final symlink.
    ///
    /// # Errors
    ///
    /// Propagates errors from path resolution and [`FileSystem::stat`].
    pub async fn symlink_metadata(&self, path: impl AsRef<[u8]>) -> FsResult<Metadata, F::DeviceError> {
        self.stat_path(path.as_ref(), no_follow(self.shared.resolve)).await
    }

    async fn stat_path(&self, path: &[u8], how: Resolve) -> FsResult<Metadata, F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
            let node = self.hold(fs.resolve(path, how).await?);
            let meta = fs.stat(node.node()).await;
            node.forget(&mut *fs);
            meta
        })
    }

    /// Lists the directory at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotADirectory`] if the path is not a directory.
    /// Propagates path resolution and metadata errors. Entry-reading errors
    /// are returned by [`ReadDir::next_entry`].
    pub async fn read_dir(&self, path: impl AsRef<[u8]>) -> FsResult<ReadDir<F>, F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
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
        })
    }

    /// The target of the symlink at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be resolved, is not a symlink,
    /// or its target cannot be read within the supported buffer size.
    pub async fn read_link(&self, path: impl AsRef<[u8]>) -> FsResult<Vec<u8>, F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
            let node = self.hold(fs.resolve(path.as_ref(), no_follow(self.shared.resolve)).await?);
            let target = read_link_of(&mut *fs, node.node()).await;
            node.forget(&mut *fs);
            target
        })
    }

    /// Creates the directory at `path`.
    ///
    /// # Errors
    ///
    /// Propagates path resolution and [`FileSystem::mkdir`] errors, including
    /// [`ErrorKind::AlreadyExists`] when the final name already exists.
    pub async fn create_dir(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
            let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
            let dir = self.hold(dir);
            let made = fs.mkdir(dir.node(), name, &SetAttr::new()).await;
            dir.forget(&mut *fs);
            fs.forget(made?, 1);
            Ok(())
        })
    }

    /// Creates the directory at `path` and every missing parent.
    ///
    /// # Errors
    ///
    /// Returns an error if a component is not a directory or a missing
    /// directory cannot be created. Earlier directories remain on failure.
    pub async fn create_dir_all(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
            create_dir_all(&mut *fs, &self.shared.pending, path.as_ref(), self.shared.resolve).await
        })
    }

    /// Removes the file or symlink at `path`.
    ///
    /// # Errors
    ///
    /// Propagates path resolution and [`FileSystem::unlink`] errors, including
    /// [`ErrorKind::IsADirectory`] and [`ErrorKind::Busy`].
    pub async fn remove_file(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
            let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
            let dir = self.hold(dir);
            let removed = fs.unlink(dir.node(), name).await;
            dir.forget(&mut *fs);
            removed
        })
    }

    /// Removes the empty directory at `path`.
    ///
    /// # Errors
    ///
    /// Propagates path resolution and [`FileSystem::rmdir`] errors, including
    /// [`ErrorKind::NotADirectory`] and [`ErrorKind::DirectoryNotEmpty`].
    pub async fn remove_dir(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
            let (dir, name) = resolve_parent(&mut *fs, path.as_ref(), self.shared.resolve).await?;
            let dir = self.hold(dir);
            let removed = fs.rmdir(dir.node(), name).await;
            dir.forget(&mut *fs);
            removed
        })
    }

    /// Removes the directory at `path` and everything in it. A symlink at
    /// `path` fails with [`ErrorKind::NotADirectory`] and is not followed,
    /// and a path ending in `.` or `..` fails with
    /// [`ErrorKind::InvalidInput`] before anything is removed.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-directory, the root directory, an invalid
    /// final component, or a failed traversal or removal. Earlier removals
    /// are not rolled back.
    pub async fn remove_dir_all(&self, path: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let path = path.as_ref();
            let how = self.shared.resolve;
            split_parent(path)?;
            let mut fs = self.enter().await?;
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
        })
    }

    /// Moves `from` to `to`, replacing an existing `to` as
    /// `std::fs::rename` does. The node keeps its id.
    ///
    /// # Errors
    ///
    /// Propagates path resolution and [`FileSystem::rename`] errors.
    /// An I/O failure may leave partial changes.
    pub async fn rename(&self, from: impl AsRef<[u8]>, to: impl AsRef<[u8]>) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let how = self.shared.resolve;
            let mut fs = self.enter().await?;
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
        })
    }

    /// Changes the times, permissions, owner or attributes of the node at
    /// `path`.
    ///
    /// # Errors
    ///
    /// Propagates path resolution and [`FileSystem::setattr`] errors,
    /// including read-only mounts and attributes the format cannot store.
    pub async fn set_attr(&self, path: impl AsRef<[u8]>, changes: &SetAttr) -> FsResult<(), F::DeviceError> {
        watched!(self, {
            let mut fs = self.enter().await?;
            let node = self.hold(fs.resolve(path.as_ref(), self.shared.resolve).await?);
            let set = fs.setattr(node.node(), changes).await;
            node.forget(&mut *fs);
            set
        })
    }
}

/// Opens the pinned `node` for `options`, emptying it for `truncate`, and
/// hands its pin to the caller. On an error the pin is forgotten.
async fn open_held<F: FileSystem + ?Sized>(
    fs: &mut F,
    mut node: Held<'_>,
    options: OpenOptions,
) -> FsResult<NodeId, F::DeviceError> {
    let mode = if options.is_write() { OpenMode::Write } else { OpenMode::Read };
    if let Err(err) = fs.open(node.node(), mode).await {
        node.forget(fs);
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
            node.forget(fs);
            return Err(err);
        }
    }
    Ok(node.keep())
}

/// The most `read_all` reserves up front from a file's reported length.
const READ_HINT: usize = 1 << 20;

/// Reads all of the open file `node` until a read returns 0. The reported
/// length is only a hint, so a damaged image that claims a huge file grows
/// the buffer as data arrives; a failed allocation is
/// [`ErrorKind::LimitExceeded`].
async fn read_all<F: FileSystem + ?Sized>(fs: &mut F, node: NodeId) -> FsResult<Vec<u8>, F::DeviceError> {
    let hint = usize::try_from(fs.stat(node).await?.len()).unwrap_or(usize::MAX);
    let mut data = Vec::new();
    data.try_reserve(hint.clamp(1, READ_HINT)).map_err(|_| ErrorKind::LimitExceeded)?;
    loop {
        if data.len() == data.capacity() {
            data.try_reserve(data.len().max(4096)).map_err(|_| ErrorKind::LimitExceeded)?;
        }
        let filled = data.len();
        data.resize(data.capacity(), 0);
        let n = fs.read(node, filled as u64, &mut data[filled..]).await?;
        data.truncate(filled + n);
        if n == 0 {
            return Ok(data);
        }
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

/// Empties the directory `top`, depth first. Each directory's listing
/// continues from the cursor after the entry last removed, which stays
/// valid across removes, and is read once more from the start when it ends
/// in case the driver moved an entry before the cursor.
async fn empty_dir<F: FileSystem + ?Sized>(
    fs: &mut F,
    pending: &spin::Mutex<VecDeque<Pending>>,
    top: NodeId,
) -> FsResult<(), F::DeviceError> {
    let mut stack: Vec<(Held<'_>, Vec<u8>)> = Vec::new();
    let mut cursors = alloc::vec![DirCursor::START];
    let result = loop {
        let dir = stack.last().map_or(top, |(node, _)| node.node());
        let at = cursors.last_mut().expect("one cursor per listed directory");
        let entry = match fs.readdir(dir, *at).await {
            Ok(entry) => entry,
            Err(err) => break Err(err),
        };
        if let Some(entry) = &entry {
            *at = entry.next_cursor();
        } else if !at.is_start() {
            *at = DirCursor::START;
            continue;
        }
        match entry {
            Some(entry) if entry.file_type().is_dir() => match fs.lookup(dir, entry.name()).await {
                Ok(child) if child == top || stack.iter().any(|(node, _)| node.node() == child) => {
                    fs.forget(child, 1);
                    break Err(ErrorKind::Corrupt.into());
                }
                Ok(child) => {
                    stack.push((Held::new(pending, child), entry.name().as_bytes().to_vec()));
                    cursors.push(DirCursor::START);
                }
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
                cursors.pop();
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
