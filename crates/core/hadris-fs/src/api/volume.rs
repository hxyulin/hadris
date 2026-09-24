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
/// Handles dropped while another call holds the lock queue their
/// `close_node` and `forget` for the next lock holder. With `alloc` the
/// queue grows as needed. Without it, the queue holds 16 distinct nodes,
/// and closing or forgetting a 17th while the lock is held panics, since
/// waiting for the lock would never end if this thread or task holds it.
pub struct Volume<F: FsDriver, K: LockKind> {
    driver: K::Lock<F>,
    pending: K::Lock<ForgetQueue>,
    caps: K::Lock<Capabilities>,
    root: NodeId,
}

impl<F: FsDriver, K: LockKind> Volume<F, K> {
    /// Shares `driver` behind lock `K`.
    pub fn with_lock(driver: F) -> Self {
        let root = driver.root();
        let caps = Lock::new(driver.capabilities());
        Self {
            driver: Lock::new(driver),
            pending: Lock::new(ForgetQueue::new()),
            caps,
            root,
        }
    }

    /// Locks the driver for several calls, or for format-specific methods.
    ///
    /// Calling a [`FileSystem`] method on this volume while holding the
    /// guard deadlocks, or panics with a `Local` lock. Dropping handles while
    /// holding it is fine; without `alloc`, for handles to at most 16
    /// distinct nodes (see [`Volume`]).
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
        // SAFETY: `this` is never used or dropped again, so each field is
        // moved out exactly once.
        let (driver, pending, caps) = unsafe {
            (
                core::ptr::read(&this.driver),
                core::ptr::read(&this.pending),
                core::ptr::read(&this.caps),
            )
        };
        drop((pending, caps));
        driver.into_inner()
    }

    /// Locks the queue, which is held only briefly and never across a
    /// driver call.
    fn queue(&self) -> impl core::ops::DerefMut<Target = ForgetQueue> + '_ {
        loop {
            if let Some(queue) = self.pending.try_lock() {
                return queue;
            }
            core::hint::spin_loop();
        }
    }

    fn drain(&self, driver: &mut F) {
        let caps = driver.capabilities();
        loop {
            if let Some(mut slot) = self.caps.try_lock() {
                *slot = caps;
                break;
            }
            core::hint::spin_loop();
        }
        let mut batch = self.queue().take();
        run_queued(&mut batch, driver);
    }

    fn drain_mut(&mut self) {
        let mut batch = self.pending.get_mut().take();
        run_queued(&mut batch, self.driver.get_mut());
    }

    /// Runs `call` on the driver now if the lock is free, else queues it
    /// with `queue` for the next lock holder.
    fn deferred(&self, node: NodeId, call: impl Fn(&mut F, NodeId), queue: impl Fn(&mut ForgetQueue, NodeId) -> bool) {
        if let Some(mut driver) = self.driver.try_lock() {
            self.drain(&mut driver);
            call(&mut driver, node);
            return;
        }
        if queue(&mut self.queue(), node) {
            return;
        }
        if let Some(mut driver) = self.driver.try_lock() {
            self.drain(&mut driver);
            call(&mut driver, node);
            return;
        }
        panic!(
            "hadris-fs Volume: a handle was dropped while the driver lock was held and 16 other \
             nodes were already waiting; without the `alloc` feature the queue is full. Drop \
             the `Volume::lock()` guard before dropping handles, or enable `alloc`"
        );
    }
}

/// Runs a batch of queued calls in order. Publishes are queued only in the
/// blocking API, where they can run here; their errors are ignored, as in
/// the `Drop` that queued them.
fn run_queued<F: FsDriver + ?Sized>(batch: &mut ForgetQueue, driver: &mut F) {
    batch.drain(|op, node| match op {
        Op::Publish => {
            sync_only! {
                let _ = driver.publish_node(node);
            }
        }
        Op::Close => driver.close_node(node),
        Op::Forget => driver.forget(node),
    });
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
        if let Some(mut driver) = self.driver.try_lock() {
            self.drain(&mut driver);
            return driver.capabilities();
        }
        loop {
            if let Some(caps) = self.caps.try_lock() {
                return *caps;
            }
            core::hint::spin_loop();
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
    /// lock holder. Panics if the queue is full, which needs a build
    /// without `alloc` (see [`Volume`]).
    fn forget(&self, node: NodeId) {
        self.deferred(node, |driver, node| driver.forget(node), ForgetQueue::push);
    }

    async fn open_node(&self, node: NodeId) -> FsResult<(), F::DeviceError> {
        self.lock().await.open_node(node).await
    }

    /// Closes now if the lock is free, else queues the close as
    /// [`forget`](Self::forget) does.
    fn close_node(&self, node: NodeId) {
        self.deferred(node, |driver, node| driver.close_node(node), ForgetQueue::push_close);
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

    async fn remove(&self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), F::DeviceError> {
        self.lock().await.remove(dir, name, kind).await
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

    /// Publishes under the lock. In the blocking API, when the lock is held
    /// and a close of `node` is queued, as when a written [`File`] is
    /// dropped while [`lock`](Volume::lock) is held, the publish joins the
    /// queue and this returns `Ok(())` without waiting.
    async fn publish_node(&self, node: NodeId) -> FsResult<(), F::DeviceError> {
        sync_only! {
            if self.driver.try_lock().is_none() && self.queue().push_publish(node) {
                return Ok(());
            }
        }
        self.lock().await.publish_node(node).await
    }

    async fn sync(&self) -> FsResult<(), F::DeviceError> {
        self.lock().await.sync().await
    }
}

}
