use crate::{
    Capabilities, DirCursor, DirEntry, FsResult, FsStats, Metadata, Name, NodeId, OpenMode,
    RenameMode, Resolve, SetAttr,
};

#[allow(unused_imports)]
use hadris_io::async_ as io;
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[allow(unused_macros)]
macro_rules! sync_only {
    ($($item:tt)*) => {};
}

#[allow(unused_macros)]
macro_rules! async_only {
    ($($item:tt)*) => { $($item)* };
}

/// Runs a `Volume` call, marking the volume failed if a poll of it panics.
#[allow(unused_macros)]
macro_rules! watched {
    ($vol:expr, $body:block) => {
        $vol.watch(async move $body).await
    };
}

#[allow(clippy::duplicate_mod)]
#[path = "api/mod.rs"]
mod api;
#[cfg(feature = "unstable-contract")]
#[doc(hidden)]
pub use api::contract;
#[cfg(feature = "alloc")]
pub use api::{ContentReader, copy_tree};
#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
pub use api::{File, ReadDir, Volume, VolumeGuard, WalkDir, WalkDirEntry, read_tree};
pub use api::{FileSystem, Walk};

#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
#[allow(clippy::duplicate_mod)]
#[path = "async_lock.rs"]
mod lock;

/// The `Send` form of [`FileSystem`], generated from the same source.
#[path = ""]
mod send {
    #[allow(unused_imports)]
    use hadris_io::async_ as io;
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
    }

    #[allow(clippy::duplicate_mod)]
    #[path = "api/filesystem.rs"]
    pub(super) mod filesystem;
    #[allow(clippy::duplicate_mod, dead_code)]
    #[path = "api/paths.rs"]
    mod paths;
}

/// A [`FileSystem`] whose every future is `Send`, for code that is generic
/// over the filesystem and spawns its work on a multi-threaded executor.
///
/// A concrete filesystem over a `Send` device needs no bound: its
/// [`FileSystem`] futures are already `Send`. A generic `F: FileSystem`
/// cannot prove that, so generic code bounds on `SendFileSystem` instead.
/// Each format implements it for its `Send` devices, and [`Sendable`]
/// turns one back into a [`FileSystem`] whose futures are provably `Send`.
pub use send::filesystem::FileSystem as SendFileSystem;

/// A [`SendFileSystem`] used as a [`FileSystem`], whose futures stay
/// provably `Send` in generic code. [`SendVolume`] shares one.
#[derive(Debug)]
pub struct Sendable<F>(F);

impl<F> Sendable<F> {
    /// Wraps `fs`.
    pub fn new(fs: F) -> Self {
        Self(fs)
    }

    /// The wrapped filesystem.
    pub fn into_inner(self) -> F {
        self.0
    }
}

impl<F> core::ops::Deref for Sendable<F> {
    type Target = F;

    fn deref(&self) -> &F {
        &self.0
    }
}

impl<F> core::ops::DerefMut for Sendable<F> {
    fn deref_mut(&mut self) -> &mut F {
        &mut self.0
    }
}

impl<F: SendFileSystem> FileSystem for Sendable<F> {
    type DeviceError = F::DeviceError;

    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    fn root(&self) -> NodeId {
        self.0.root()
    }
    async fn statfs(&mut self) -> FsResult<FsStats, Self::DeviceError> {
        self.0.statfs().await
    }
    async fn label<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> FsResult<Option<&'b str>, Self::DeviceError> {
        self.0.label(buf).await
    }
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError> {
        self.0.lookup(dir, name).await
    }
    fn forget(&mut self, node: NodeId, count: u64) {
        self.0.forget(node, count)
    }
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
        self.0.parent(dir).await
    }
    async fn resolve(&mut self, path: &[u8], how: Resolve) -> FsResult<NodeId, Self::DeviceError> {
        self.0.resolve(path, how).await
    }
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError> {
        self.0.stat(node).await
    }
    async fn readdir(
        &mut self,
        dir: NodeId,
        from: DirCursor,
    ) -> FsResult<Option<DirEntry>, Self::DeviceError> {
        self.0.readdir(dir, from).await
    }
    async fn readlink<'b>(
        &mut self,
        node: NodeId,
        buf: &'b mut [u8],
    ) -> FsResult<&'b [u8], Self::DeviceError> {
        self.0.readlink(node, buf).await
    }
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), Self::DeviceError> {
        self.0.open(node, mode).await
    }
    async fn close(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        self.0.close(node).await
    }
    async fn read(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, Self::DeviceError> {
        self.0.read(node, offset, buf).await
    }
    async fn setattr(
        &mut self,
        node: NodeId,
        changes: &SetAttr,
    ) -> FsResult<(), Self::DeviceError> {
        self.0.setattr(node, changes).await
    }
    async fn write(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &[u8],
    ) -> FsResult<usize, Self::DeviceError> {
        self.0.write(node, offset, buf).await
    }
    async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
        self.0.truncate(node, len).await
    }
    async fn fsync(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        self.0.fsync(node).await
    }
    async fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        attrs: &SetAttr,
    ) -> FsResult<NodeId, Self::DeviceError> {
        self.0.create(dir, name, attrs).await
    }
    async fn mkdir(
        &mut self,
        dir: NodeId,
        name: &Name,
        attrs: &SetAttr,
    ) -> FsResult<NodeId, Self::DeviceError> {
        self.0.mkdir(dir, name, attrs).await
    }
    async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        self.0.unlink(dir, name).await
    }
    async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        self.0.rmdir(dir, name).await
    }
    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> FsResult<(), Self::DeviceError> {
        self.0.rename(from_dir, from, to_dir, to, mode).await
    }
    async fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
        self.0.sync().await
    }
}

/// A [`Volume`] over a [`SendFileSystem`], whose futures are `Send` in
/// code generic over the filesystem.
#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
pub type SendVolume<F> = Volume<Sendable<F>>;
