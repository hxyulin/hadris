use super::ApfsFs;
use hadris_fs::{FsResult, Name, NodeId, RenameMode, Resolve, SetAttr};
use hadris_storage::async_::BlockDevice;

impl<D: BlockDevice> ApfsFs<D> {
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
