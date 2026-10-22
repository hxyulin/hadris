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
pub use api::*;

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

    use crate::{
        Capabilities, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats, Metadata, Name,
        NodeId, OpenMode, RenameMode, Resolve, SetAttr,
    };

    #[allow(clippy::duplicate_mod)]
    #[path = "api/filesystem.rs"]
    pub(super) mod filesystem;
    use filesystem::FileSystem;
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

    fn capabilities(&self) -> crate::Capabilities {
        self.0.capabilities()
    }
    fn root(&self) -> crate::NodeId {
        self.0.root()
    }
    async fn statfs(&mut self) -> crate::FsResult<crate::FsStats, Self::DeviceError> {
        self.0.statfs().await
    }
    async fn label<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> crate::FsResult<Option<&'b str>, Self::DeviceError> {
        self.0.label(buf).await
    }
    async fn lookup(
        &mut self,
        dir: crate::NodeId,
        name: &crate::Name,
    ) -> crate::FsResult<crate::NodeId, Self::DeviceError> {
        self.0.lookup(dir, name).await
    }
    fn forget(&mut self, node: crate::NodeId, count: u64) {
        self.0.forget(node, count)
    }
    async fn parent(
        &mut self,
        dir: crate::NodeId,
    ) -> crate::FsResult<crate::NodeId, Self::DeviceError> {
        self.0.parent(dir).await
    }
    async fn resolve(
        &mut self,
        path: &[u8],
        how: crate::Resolve,
    ) -> crate::FsResult<crate::NodeId, Self::DeviceError> {
        self.0.resolve(path, how).await
    }
    async fn stat(
        &mut self,
        node: crate::NodeId,
    ) -> crate::FsResult<crate::Metadata, Self::DeviceError> {
        self.0.stat(node).await
    }
    async fn readdir(
        &mut self,
        dir: crate::NodeId,
        from: crate::DirCursor,
    ) -> crate::FsResult<Option<crate::DirEntry>, Self::DeviceError> {
        self.0.readdir(dir, from).await
    }
    async fn readlink<'b>(
        &mut self,
        node: crate::NodeId,
        buf: &'b mut [u8],
    ) -> crate::FsResult<&'b [u8], Self::DeviceError> {
        self.0.readlink(node, buf).await
    }
    async fn open(
        &mut self,
        node: crate::NodeId,
        mode: crate::OpenMode,
    ) -> crate::FsResult<(), Self::DeviceError> {
        self.0.open(node, mode).await
    }
    async fn close(&mut self, node: crate::NodeId) -> crate::FsResult<(), Self::DeviceError> {
        self.0.close(node).await
    }
    async fn read(
        &mut self,
        node: crate::NodeId,
        offset: u64,
        buf: &mut [u8],
    ) -> crate::FsResult<usize, Self::DeviceError> {
        self.0.read(node, offset, buf).await
    }
    async fn setattr(
        &mut self,
        node: crate::NodeId,
        changes: &crate::SetAttr,
    ) -> crate::FsResult<(), Self::DeviceError> {
        self.0.setattr(node, changes).await
    }
    async fn write(
        &mut self,
        node: crate::NodeId,
        offset: u64,
        buf: &[u8],
    ) -> crate::FsResult<usize, Self::DeviceError> {
        self.0.write(node, offset, buf).await
    }
    async fn truncate(
        &mut self,
        node: crate::NodeId,
        len: u64,
    ) -> crate::FsResult<(), Self::DeviceError> {
        self.0.truncate(node, len).await
    }
    async fn fsync(&mut self, node: crate::NodeId) -> crate::FsResult<(), Self::DeviceError> {
        self.0.fsync(node).await
    }
    async fn create(
        &mut self,
        dir: crate::NodeId,
        name: &crate::Name,
        attrs: &crate::SetAttr,
    ) -> crate::FsResult<crate::NodeId, Self::DeviceError> {
        self.0.create(dir, name, attrs).await
    }
    async fn mkdir(
        &mut self,
        dir: crate::NodeId,
        name: &crate::Name,
        attrs: &crate::SetAttr,
    ) -> crate::FsResult<crate::NodeId, Self::DeviceError> {
        self.0.mkdir(dir, name, attrs).await
    }
    async fn unlink(
        &mut self,
        dir: crate::NodeId,
        name: &crate::Name,
    ) -> crate::FsResult<(), Self::DeviceError> {
        self.0.unlink(dir, name).await
    }
    async fn rmdir(
        &mut self,
        dir: crate::NodeId,
        name: &crate::Name,
    ) -> crate::FsResult<(), Self::DeviceError> {
        self.0.rmdir(dir, name).await
    }
    async fn rename(
        &mut self,
        from_dir: crate::NodeId,
        from: &crate::Name,
        to_dir: crate::NodeId,
        to: &crate::Name,
        mode: crate::RenameMode,
    ) -> crate::FsResult<(), Self::DeviceError> {
        self.0.rename(from_dir, from, to_dir, to, mode).await
    }
    async fn sync(&mut self) -> crate::FsResult<(), Self::DeviceError> {
        self.0.sync().await
    }
}

/// A [`Volume`] over a [`SendFileSystem`], whose futures are `Send` in
/// code generic over the filesystem.
#[cfg(all(feature = "alloc", target_has_atomic = "ptr"))]
pub type SendVolume<F> = Volume<Sendable<F>>;
