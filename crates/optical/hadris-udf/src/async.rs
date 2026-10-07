#[allow(unused_macros)]
macro_rules! io_transform {
    ($($item:tt)*) => { $($item)* };
}

#[cfg(feature = "alloc")]
use hadris_fs::local as fs;
#[cfg(feature = "alloc")]
use hadris_iso::async_ as iso;
use hadris_storage::async_ as storage;

use hadris_fs::local::FileSystem;

#[path = "read.rs"]
mod read;
pub use read::UdfFs;
#[cfg(feature = "alloc")]
#[path = "write.rs"]
mod write;
#[cfg(feature = "alloc")]
pub use write::{write, write_bridge};

use hadris_fs::{
    Capabilities, DirCursor, DirEntry, FsResult, FsStats, Metadata, Name, NodeId, OpenMode,
    RenameMode, Resolve, SetAttr,
};

impl<D: storage::BlockDevice> UdfFs<D> {
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
    /// Delegates to [`hadris_fs::local::FileSystem::resolve`].
    pub fn resolve(
        &mut self,
        path: &[u8],
        how: Resolve,
    ) -> impl core::future::Future<Output = FsResult<NodeId, D::Error>> {
        hadris_fs::local::FileSystem::resolve(self, path, how)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::setattr`].
    pub fn setattr(
        &mut self,
        node: NodeId,
        changes: &SetAttr,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::setattr(self, node, changes)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::write`].
    pub fn write(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &[u8],
    ) -> impl core::future::Future<Output = FsResult<usize, D::Error>> {
        hadris_fs::local::FileSystem::write(self, node, offset, buf)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::truncate`].
    pub fn truncate(
        &mut self,
        node: NodeId,
        len: u64,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::truncate(self, node, len)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::fsync`].
    pub fn fsync(
        &mut self,
        node: NodeId,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::fsync(self, node)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::create`].
    pub fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        attrs: &SetAttr,
    ) -> impl core::future::Future<Output = FsResult<NodeId, D::Error>> {
        hadris_fs::local::FileSystem::create(self, dir, name, attrs)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::mkdir`].
    pub fn mkdir(
        &mut self,
        dir: NodeId,
        name: &Name,
        attrs: &SetAttr,
    ) -> impl core::future::Future<Output = FsResult<NodeId, D::Error>> {
        hadris_fs::local::FileSystem::mkdir(self, dir, name, attrs)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::unlink`].
    pub fn unlink(
        &mut self,
        dir: NodeId,
        name: &Name,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::unlink(self, dir, name)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::rmdir`].
    pub fn rmdir(
        &mut self,
        dir: NodeId,
        name: &Name,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::rmdir(self, dir, name)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::rename`].
    pub fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::rename(self, from_dir, from, to_dir, to, mode)
    }
    /// Delegates to [`hadris_fs::local::FileSystem::sync`].
    pub fn sync(&mut self) -> impl core::future::Future<Output = FsResult<(), D::Error>> {
        hadris_fs::local::FileSystem::sync(self)
    }
}

#[cfg(feature = "async")]
impl<D: hadris_storage::async_::SendBlockDevice> hadris_fs::r#async::FileSystem for UdfFs<D> {
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
