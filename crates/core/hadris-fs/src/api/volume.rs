use super::*;

io_transform! {

/// A driver behind a lock, shared through `&self`.
///
/// Opt-in: nothing in the driver tier needs it. Each [`FileSystem`] call
/// takes the lock once and makes one driver call; a path resolves under one
/// lock hold. The constructor picks the lock (`Volume::new`, `Volume::spin`,
/// `Volume::local`, or [`with_lock`](Self::with_lock) for any [`LockKind`]),
/// so there is no default lock type. Code that stores a volume names one
/// alias:
///
/// ```ignore
/// type Disk = Volume<FatFs<std::fs::File>, StdMutex>;
/// ```
///
/// Handles dropped while another call holds the lock queue their `forget`
/// for the next lock holder, without allocating. The queue holds 16 distinct
/// nodes; when it is full, `forget` waits for the lock.
pub struct Volume<F: FsDriver, K: LockKind> {
    driver: K::Lock<F>,
    pending: spin::Mutex<ForgetQueue>,
    caps: spin::Mutex<Capabilities>,
    root: NodeId,
}

impl<F: FsDriver, K: LockKind> Volume<F, K> {
    /// Shares `driver` behind lock `K`.
    pub fn with_lock(driver: F) -> Self {
        let root = driver.root();
        let caps = spin::Mutex::new(driver.capabilities());
        Self {
            driver: Lock::new(driver),
            pending: spin::Mutex::new(ForgetQueue::new()),
            caps,
            root,
        }
    }

    /// Locks the driver for several calls, or for format-specific methods.
    ///
    /// Calling a [`FileSystem`] method on this volume while holding the
    /// guard deadlocks, or panics with a `Local` lock. Dropping handles while
    /// holding it is fine.
    pub async fn lock(&self) -> impl core::ops::DerefMut<Target = F> + '_ {
        let mut guard = self.driver.lock().await;
        self.drain(&mut guard);
        guard
    }

    /// Borrows the driver without locking.
    pub fn get_mut(&mut self) -> &mut F {
        self.drain_mut();
        self.driver.get_mut()
    }

    /// Returns the driver, after applying queued forgets.
    pub fn into_inner(self) -> F {
        let mut this = core::mem::ManuallyDrop::new(self);
        this.drain_mut();
        // SAFETY: `this` is never used or dropped again, so the lock is moved
        // out exactly once. The other fields have no drop glue.
        let driver = unsafe { core::ptr::read(&this.driver) };
        driver.into_inner()
    }

    fn drain(&self, driver: &mut F) {
        *self.caps.lock() = driver.capabilities();
        let mut pending = self.pending.lock();
        if !pending.is_empty() {
            pending.drain(|node| driver.forget(node));
        }
    }

    fn drain_mut(&mut self) {
        let driver = self.driver.get_mut();
        self.pending.get_mut().drain(|node| driver.forget(node));
    }
}

impl<F: FsDriver, K: LockKind> core::fmt::Debug for Volume<F, K> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Volume").field("root", &self.root).finish_non_exhaustive()
    }
}

impl<F: FsDriver, K: LockKind> Drop for Volume<F, K> {
    fn drop(&mut self) {
        self.drain_mut();
    }
}

impl<F: FsDriver, K: LockKind> FileSystem for Volume<F, K> {
    type DeviceError = F::DeviceError;

    /// The driver's capabilities, or the value seen by the last lock holder
    /// when another call holds the lock.
    fn capabilities(&self) -> Capabilities {
        match self.driver.try_lock() {
            Some(mut driver) => {
                self.drain(&mut driver);
                driver.capabilities()
            }
            None => *self.caps.lock(),
        }
    }

    fn root(&self) -> NodeId {
        self.root
    }

    async fn lookup(&self, dir: NodeId, name: &Name) -> FsResult<NodeId, F::DeviceError> {
        self.lock().await.lookup(dir, name).await
    }

    async fn node_metadata(&self, node: NodeId) -> FsResult<Metadata, F::DeviceError> {
        self.lock().await.node_metadata(node).await
    }

    async fn read_dir_entry(
        &self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, F::DeviceError> {
        self.lock().await.read_dir_entry(dir, cursor, name).await
    }

    async fn read_at(&self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, F::DeviceError> {
        self.lock().await.read_at(node, offset, buf).await
    }

    async fn stats(&self) -> FsResult<FsStats, F::DeviceError> {
        self.lock().await.stats().await
    }

    /// Forgets now if the lock is free, else queues the node for the next
    /// lock holder. If the queue is full it waits for the lock, which never
    /// comes if this thread, or a suspended task on it, holds the lock.
    fn forget(&self, node: NodeId) {
        loop {
            if let Some(mut driver) = self.driver.try_lock() {
                self.drain(&mut driver);
                driver.forget(node);
                return;
            }
            if self.pending.lock().push(node) {
                return;
            }
            core::hint::spin_loop();
        }
    }

    async fn parent(&self, dir: NodeId) -> FsResult<NodeId, F::DeviceError> {
        self.lock().await.parent(dir).await
    }

    async fn read_link(&self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, F::DeviceError> {
        self.lock().await.read_link(link, buf).await
    }

    /// Resolves under one lock hold, with the driver's policy.
    async fn resolve(&self, path: &str) -> FsResult<NodeId, F::DeviceError> {
        self.lock().await.resolve(path).await
    }

    async fn create(
        &self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        meta: &SetMetadata,
    ) -> FsResult<NodeId, F::DeviceError> {
        self.lock().await.create(dir, name, kind, meta).await
    }

    async fn remove(&self, dir: NodeId, name: &Name) -> FsResult<(), F::DeviceError> {
        self.lock().await.remove(dir, name).await
    }

    async fn rename(
        &self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), F::DeviceError> {
        self.lock().await.rename(from_dir, from, to_dir, to, flags).await
    }

    async fn write_at(&self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, F::DeviceError> {
        self.lock().await.write_at(node, offset, buf).await
    }

    async fn set_len(&self, node: NodeId, len: u64) -> FsResult<(), F::DeviceError> {
        self.lock().await.set_len(node, len).await
    }

    async fn set_metadata(&self, node: NodeId, changes: &SetMetadata) -> FsResult<(), F::DeviceError> {
        self.lock().await.set_metadata(node, changes).await
    }

    async fn sync_node(&self, node: NodeId) -> FsResult<(), F::DeviceError> {
        self.lock().await.sync_node(node).await
    }

    async fn sync(&self) -> FsResult<(), F::DeviceError> {
        self.lock().await.sync().await
    }
}

}
