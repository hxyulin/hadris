#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

use hadris_fs::local::FileSystem;
use hadris_storage::async_ as storage;

#[allow(clippy::duplicate_mod)]
#[path = "image.rs"]
pub(crate) mod image;
pub use image::IsoFs;

use hadris_fs::{
    Capabilities, DirCursor, DirEntry, FsResult, FsStats, Metadata, Name, NodeId, OpenMode,
};

impl<D: storage::BlockDevice> IsoFs<D> {
    /// Delegates to [`hadris_fs::local::FileSystem::capabilities`].
    pub fn capabilities(&self) -> Capabilities {
        hadris_fs::local::FileSystem::capabilities(self)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::root`].
    pub fn root(&self) -> NodeId {
        hadris_fs::local::FileSystem::root(self)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::statfs`].
    pub fn statfs(&mut self) -> impl core::future::Future<Output = FsResult<FsStats, D::Error>> {
        hadris_fs::local::FileSystem::statfs(self)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::label`].
    pub fn label<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> impl core::future::Future<Output = FsResult<Option<&'b str>, D::Error>> {
        hadris_fs::local::FileSystem::label(self, buf)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::lookup`].
    pub fn lookup(
        &mut self,
        dir: NodeId,
        name: &Name,
    ) -> impl core::future::Future<Output = FsResult<NodeId, D::Error>> {
        hadris_fs::local::FileSystem::lookup(self, dir, name)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::forget`].
    pub fn forget(&mut self, node: NodeId, count: u64) {
        hadris_fs::local::FileSystem::forget(self, node, count)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::parent`].
    pub fn parent(
        &mut self,
        dir: NodeId,
    ) -> impl core::future::Future<Output = FsResult<NodeId, D::Error>> {
        hadris_fs::local::FileSystem::parent(self, dir)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::stat`].
    pub fn stat(
        &mut self,
        node: NodeId,
    ) -> impl core::future::Future<Output = FsResult<Metadata, D::Error>> {
        hadris_fs::local::FileSystem::stat(self, node)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::readdir`].
    pub fn readdir(
        &mut self,
        dir: NodeId,
        from: DirCursor,
    ) -> impl core::future::Future<Output = FsResult<Option<DirEntry>, D::Error>> {
        hadris_fs::local::FileSystem::readdir(self, dir, from)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::readlink`].
    pub fn readlink<'b>(
        &mut self,
        node: NodeId,
        buf: &'b mut [u8],
    ) -> impl core::future::Future<Output = FsResult<&'b [u8], D::Error>> {
        hadris_fs::local::FileSystem::readlink(self, node, buf)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::open`].
    pub fn open(
        &mut self,
        node: NodeId,
        mode: OpenMode,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::open(self, node, mode)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::close`].
    pub fn close(
        &mut self,
        node: NodeId,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::close(self, node)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::read`].
    pub fn read(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &mut [u8],
    ) -> impl core::future::Future<Output = FsResult<usize, D::Error>> {
        hadris_fs::local::FileSystem::read(self, node, offset, buf)
    }
}

#[cfg(feature = "async")]
impl<D: hadris_storage::async_::SendBlockDevice> hadris_fs::r#async::FileSystem for IsoFs<D> {
    type DeviceError = D::Error;
    fn capabilities(&self) -> Capabilities {
        hadris_fs::local::FileSystem::capabilities(self)
    }
    fn root(&self) -> NodeId {
        hadris_fs::local::FileSystem::root(self)
    }
    fn statfs(&mut self) -> impl core::future::Future<Output = FsResult<FsStats, D::Error>> + Send {
        hadris_fs::local::FileSystem::statfs(self)
    }
    fn label<'b>(
        &mut self,
        buf: &'b mut [u8],
    ) -> impl core::future::Future<Output = FsResult<Option<&'b str>, D::Error>> + Send {
        hadris_fs::local::FileSystem::label(self, buf)
    }
    fn lookup(
        &mut self,
        dir: NodeId,
        name: &Name,
    ) -> impl core::future::Future<Output = FsResult<NodeId, D::Error>> + Send {
        hadris_fs::local::FileSystem::lookup(self, dir, name)
    }
    fn forget(&mut self, node: NodeId, count: u64) {
        hadris_fs::local::FileSystem::forget(self, node, count)
    }
    fn parent(
        &mut self,
        dir: NodeId,
    ) -> impl core::future::Future<Output = FsResult<NodeId, D::Error>> + Send {
        hadris_fs::local::FileSystem::parent(self, dir)
    }
    fn stat(
        &mut self,
        node: NodeId,
    ) -> impl core::future::Future<Output = FsResult<Metadata, D::Error>> + Send {
        hadris_fs::local::FileSystem::stat(self, node)
    }
    fn readdir(
        &mut self,
        dir: NodeId,
        from: DirCursor,
    ) -> impl core::future::Future<Output = FsResult<Option<DirEntry>, D::Error>> + Send {
        hadris_fs::local::FileSystem::readdir(self, dir, from)
    }
    fn readlink<'b>(
        &mut self,
        node: NodeId,
        buf: &'b mut [u8],
    ) -> impl core::future::Future<Output = FsResult<&'b [u8], D::Error>> + Send {
        hadris_fs::local::FileSystem::readlink(self, node, buf)
    }
    fn open(
        &mut self,
        node: NodeId,
        mode: OpenMode,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> + Send {
        hadris_fs::local::FileSystem::open(self, node, mode)
    }
    fn close(
        &mut self,
        node: NodeId,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> + Send {
        hadris_fs::local::FileSystem::close(self, node)
    }
    fn read(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &mut [u8],
    ) -> impl core::future::Future<Output = FsResult<usize, D::Error>> + Send {
        hadris_fs::local::FileSystem::read(self, node, offset, buf)
    }
}

#[cfg(all(feature = "alloc", feature = "async"))]
pub use crate::r#async::{Session, write};
