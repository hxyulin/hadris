use super::*;

io_transform! {

/// The node API that format crates implement: exclusive access through
/// `&mut self`, no locks.
///
/// Write methods default to [`ErrorKind::ReadOnly`] and `parent`/`read_link`
/// to [`ErrorKind::Unsupported`], so a read-only format implements only the
/// read half. Most formats write each method once as an inherent method and
/// generate this impl with [`impl_fs_driver!`](crate::impl_fs_driver).
///
/// Contract:
/// - `lookup`, `create` and `parent` pin the node they return; `forget`
///   unpins it. A pinned `NodeId` stays valid, even across `rename`.
/// - A pin never blocks a removal. `open_node` marks a pinned node as open
///   and `close_node` ends that; `remove` and a replacing `rename` fail with
///   [`ErrorKind::Busy`] only when they would remove the last name of an
///   open node. A pinned node that is removed keeps its id until its last
///   `forget`, and every method but `forget` and `close_node` answers
///   [`ErrorKind::NotFound`] for it.
/// - `.` and `..` never appear in `read_dir_entry` output, and `lookup`
///   rejects them.
/// - A failed operation leaves the filesystem unchanged, in memory and on
///   disk.
pub trait FsDriver {
    /// The device's own error, returned inside [`Error`].
    type DeviceError: core::error::Error + Send + Sync + 'static;

    /// What this filesystem supports, for this mount.
    fn capabilities(&self) -> Capabilities;

    /// The root directory. Always pinned.
    fn root(&self) -> NodeId;

    /// Finds `name` in `dir` and pins the result.
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError>;

    /// Metadata of a pinned node, or of an entry just returned by
    /// `read_dir_entry`.
    async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError>;

    /// Writes the entry after `cursor` into `name` and advances `cursor`.
    /// `None` at the end. Entries are not pinned.
    async fn read_dir_entry(
        &mut self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, Self::DeviceError>;

    /// Reads from a file at `offset`. Returns 0 at or past the end.
    async fn read_at(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, Self::DeviceError>;

    /// Space usage.
    async fn stats(&mut self) -> FsResult<FsStats, Self::DeviceError>;

    /// Unpins a node. Never fails.
    fn forget(&mut self, node: NodeId);

    /// Marks a pinned node as open, so that removing its last name fails
    /// with [`ErrorKind::Busy`] until the matching
    /// [`close_node`](Self::close_node). The caller keeps its pin while the
    /// node is open. The default does nothing, which suits drivers that
    /// cannot remove nodes.
    async fn open_node(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        let _ = node;
        Ok(())
    }

    /// Ends one [`open_node`](Self::open_node). Never fails and never
    /// blocks, so `Drop` can call it.
    fn close_node(&mut self, node: NodeId) {
        let _ = node;
    }

    /// The directory containing `dir`, pinned. The root is its own parent.
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
        let _ = dir;
        Err(ErrorKind::Unsupported.into())
    }

    /// Writes a symlink's target into `buf` and returns its length.
    /// [`ErrorKind::LimitExceeded`] if `buf` is too small.
    async fn read_link(&mut self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
        let _ = (link, buf);
        Err(ErrorKind::Unsupported.into())
    }

    /// Resolves `path` from the root and pins the result: [`Lexical`] unless
    /// the driver is wrapped in [`WithResolver`].
    async fn resolve(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
        Lexical.resolve(self, path).await
    }

    /// Creates `name` in `dir` and pins the new node.
    async fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        meta: &SetMetadata,
    ) -> FsResult<NodeId, Self::DeviceError> {
        let _ = (dir, name, kind, meta);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Removes `name` from `dir`, which must be of `kind`: a directory fails
    /// with [`ErrorKind::IsADirectory`] for [`RemoveKind::File`], anything
    /// but a directory with [`ErrorKind::NotADirectory`] for
    /// [`RemoveKind::Dir`]. A directory must be empty.
    ///
    /// Fails with [`ErrorKind::Busy`] when `name` is the last name of an
    /// open node. A node that is only pinned is removed, and its id answers
    /// [`ErrorKind::NotFound`] until its last `forget`.
    async fn remove(&mut self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), Self::DeviceError> {
        let _ = (dir, name, kind);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`. The moved node keeps
    /// its `NodeId`. Replacing an existing `to` removes it as
    /// [`remove`](Self::remove) would, so an open target fails with
    /// [`ErrorKind::Busy`].
    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), Self::DeviceError> {
        let _ = (from_dir, from, to_dir, to, flags);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Writes to a file at `offset`, growing it as needed.
    async fn write_at(
        &mut self,
        node: NodeId,
        offset: u64,
        buf: &[u8],
    ) -> FsResult<usize, Self::DeviceError> {
        let _ = (node, offset, buf);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Truncates or extends a file.
    async fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
        let _ = (node, len);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Changes times, mode, owner or attributes. Fields the format cannot
    /// store are ignored.
    async fn set_metadata(
        &mut self,
        node: NodeId,
        changes: &SetMetadata,
    ) -> FsResult<(), Self::DeviceError> {
        let _ = (node, changes);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Makes one node durable: writes its pending data and metadata, then
    /// flushes the device. This is `fsync`.
    async fn sync_node(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        let _ = node;
        Ok(())
    }

    /// Writes one node's pending metadata (a size or time kept in memory)
    /// to the device without flushing it, so that another mount of the
    /// device would see it once the device's own cache is written. This is
    /// what closing a file needs. The default calls
    /// [`sync_node`](Self::sync_node).
    async fn publish_node(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        self.sync_node(node).await
    }

    /// Writes every piece of cached metadata and flushes the device.
    async fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
        Ok(())
    }
}

/// What shared code programs against: the [`FsDriver`] methods on `&self`.
///
/// [`Volume`] implements it for every driver. A format that wants finer
/// locking than one lock per call can implement it directly. Because `&F` is
/// an [`FsDriver`] whenever `F` is a `FileSystem`, every path helper is
/// written once and serves both.
pub trait FileSystem {
    /// The device's own error, returned inside [`Error`].
    type DeviceError: core::error::Error + Send + Sync + 'static;

    /// What this filesystem supports, for this mount.
    fn capabilities(&self) -> Capabilities;

    /// The root directory. Always pinned.
    fn root(&self) -> NodeId;

    /// Finds `name` in `dir` and pins the result.
    async fn lookup(&self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError>;

    /// Metadata of a pinned node, or of an entry just returned by
    /// `read_dir_entry`.
    async fn node_metadata(&self, node: NodeId) -> FsResult<Metadata, Self::DeviceError>;

    /// Writes the entry after `cursor` into `name` and advances `cursor`.
    async fn read_dir_entry(
        &self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, Self::DeviceError>;

    /// Reads from a file at `offset`.
    async fn read_at(
        &self,
        node: NodeId,
        offset: u64,
        buf: &mut [u8],
    ) -> FsResult<usize, Self::DeviceError>;

    /// Space usage.
    async fn stats(&self) -> FsResult<FsStats, Self::DeviceError>;

    /// Unpins a node. Never blocks, so `Drop` can call it in every mode.
    fn forget(&self, node: NodeId);

    /// Marks a pinned node as open, so that removing its last name fails
    /// with [`ErrorKind::Busy`] until the matching
    /// [`close_node`](Self::close_node).
    async fn open_node(&self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        let _ = node;
        Ok(())
    }

    /// Ends one [`open_node`](Self::open_node). Never blocks, so `Drop` can
    /// call it in every mode.
    fn close_node(&self, node: NodeId) {
        let _ = node;
    }

    /// The directory containing `dir`, pinned.
    async fn parent(&self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
        let _ = dir;
        Err(ErrorKind::Unsupported.into())
    }

    /// Writes a symlink's target into `buf` and returns its length.
    async fn read_link(&self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
        let _ = (link, buf);
        Err(ErrorKind::Unsupported.into())
    }

    /// Resolves `path` from the root and pins the result. [`Volume`] uses its
    /// driver's policy; other implementations get [`Lexical`].
    async fn resolve(&self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
        Lexical.resolve(&mut &*self, path).await
    }

    /// Creates `name` in `dir` and pins the new node.
    async fn create(
        &self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        meta: &SetMetadata,
    ) -> FsResult<NodeId, Self::DeviceError> {
        let _ = (dir, name, kind, meta);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Removes `name` from `dir`, which must be of `kind`. A directory must
    /// be empty. Fails with [`ErrorKind::Busy`] when `name` is the last name
    /// of an open node; see [`FsDriver::remove`].
    async fn remove(&self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), Self::DeviceError> {
        let _ = (dir, name, kind);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`.
    async fn rename(
        &self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), Self::DeviceError> {
        let _ = (from_dir, from, to_dir, to, flags);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Writes to a file at `offset`, growing it as needed.
    async fn write_at(&self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, Self::DeviceError> {
        let _ = (node, offset, buf);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Truncates or extends a file.
    async fn set_len(&self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
        let _ = (node, len);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Changes times, mode, owner or attributes.
    async fn set_metadata(&self, node: NodeId, changes: &SetMetadata) -> FsResult<(), Self::DeviceError> {
        let _ = (node, changes);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Makes one node durable; see [`FsDriver::sync_node`].
    async fn sync_node(&self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        let _ = node;
        Ok(())
    }

    /// Writes one node's pending metadata without flushing the device; see
    /// [`FsDriver::publish_node`]. The default calls
    /// [`sync_node`](Self::sync_node).
    async fn publish_node(&self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        self.sync_node(node).await
    }

    /// Writes every piece of cached metadata and flushes the device.
    async fn sync(&self) -> FsResult<(), Self::DeviceError> {
        Ok(())
    }
}

/// Every [`FsDriver`] method except `resolve`, forwarded to `**self`.
macro_rules! forward_driver_methods {
    () => {
        fn capabilities(&self) -> Capabilities {
            (**self).capabilities()
        }
        fn root(&self) -> NodeId {
            (**self).root()
        }
        async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError> {
            (**self).lookup(dir, name).await
        }
        async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError> {
            (**self).node_metadata(node).await
        }
        async fn read_dir_entry(
            &mut self,
            dir: NodeId,
            cursor: &mut DirCursor,
            name: &mut NameBuf,
        ) -> FsResult<Option<DirEntry>, Self::DeviceError> {
            (**self).read_dir_entry(dir, cursor, name).await
        }
        async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).read_at(node, offset, buf).await
        }
        async fn stats(&mut self) -> FsResult<FsStats, Self::DeviceError> {
            (**self).stats().await
        }
        fn forget(&mut self, node: NodeId) {
            (**self).forget(node)
        }
        async fn open_node(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).open_node(node).await
        }
        fn close_node(&mut self, node: NodeId) {
            (**self).close_node(node)
        }
        async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
            (**self).parent(dir).await
        }
        async fn read_link(&mut self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).read_link(link, buf).await
        }
        async fn create(
            &mut self,
            dir: NodeId,
            name: &Name,
            kind: NewNode<'_>,
            meta: &SetMetadata,
        ) -> FsResult<NodeId, Self::DeviceError> {
            (**self).create(dir, name, kind, meta).await
        }
        async fn remove(&mut self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), Self::DeviceError> {
            (**self).remove(dir, name, kind).await
        }
        async fn rename(
            &mut self,
            from_dir: NodeId,
            from: &Name,
            to_dir: NodeId,
            to: &Name,
            flags: RenameFlags,
        ) -> FsResult<(), Self::DeviceError> {
            (**self).rename(from_dir, from, to_dir, to, flags).await
        }
        async fn write_at(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).write_at(node, offset, buf).await
        }
        async fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
            (**self).set_len(node, len).await
        }
        async fn set_metadata(&mut self, node: NodeId, changes: &SetMetadata) -> FsResult<(), Self::DeviceError> {
            (**self).set_metadata(node, changes).await
        }
        async fn sync_node(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).sync_node(node).await
        }
        async fn publish_node(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).publish_node(node).await
        }
        async fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
            (**self).sync().await
        }
    };
}

/// Every [`FileSystem`] method, forwarded to `**self`.
macro_rules! forward_fs_methods {
    () => {
        fn capabilities(&self) -> Capabilities {
            (**self).capabilities()
        }
        fn root(&self) -> NodeId {
            (**self).root()
        }
        async fn lookup(&self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError> {
            (**self).lookup(dir, name).await
        }
        async fn node_metadata(&self, node: NodeId) -> FsResult<Metadata, Self::DeviceError> {
            (**self).node_metadata(node).await
        }
        async fn read_dir_entry(
            &self,
            dir: NodeId,
            cursor: &mut DirCursor,
            name: &mut NameBuf,
        ) -> FsResult<Option<DirEntry>, Self::DeviceError> {
            (**self).read_dir_entry(dir, cursor, name).await
        }
        async fn read_at(&self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).read_at(node, offset, buf).await
        }
        async fn stats(&self) -> FsResult<FsStats, Self::DeviceError> {
            (**self).stats().await
        }
        fn forget(&self, node: NodeId) {
            (**self).forget(node)
        }
        async fn open_node(&self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).open_node(node).await
        }
        fn close_node(&self, node: NodeId) {
            (**self).close_node(node)
        }
        async fn parent(&self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
            (**self).parent(dir).await
        }
        async fn read_link(&self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).read_link(link, buf).await
        }
        async fn resolve(&self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
            (**self).resolve(path).await
        }
        async fn create(
            &self,
            dir: NodeId,
            name: &Name,
            kind: NewNode<'_>,
            meta: &SetMetadata,
        ) -> FsResult<NodeId, Self::DeviceError> {
            (**self).create(dir, name, kind, meta).await
        }
        async fn remove(&self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), Self::DeviceError> {
            (**self).remove(dir, name, kind).await
        }
        async fn rename(
            &self,
            from_dir: NodeId,
            from: &Name,
            to_dir: NodeId,
            to: &Name,
            flags: RenameFlags,
        ) -> FsResult<(), Self::DeviceError> {
            (**self).rename(from_dir, from, to_dir, to, flags).await
        }
        async fn write_at(&self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).write_at(node, offset, buf).await
        }
        async fn set_len(&self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
            (**self).set_len(node, len).await
        }
        async fn set_metadata(&self, node: NodeId, changes: &SetMetadata) -> FsResult<(), Self::DeviceError> {
            (**self).set_metadata(node, changes).await
        }
        async fn sync_node(&self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).sync_node(node).await
        }
        async fn publish_node(&self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).publish_node(node).await
        }
        async fn sync(&self) -> FsResult<(), Self::DeviceError> {
            (**self).sync().await
        }
    };
}

impl<F: FsDriver + ?Sized> FsDriver for &mut F {
    type DeviceError = F::DeviceError;
    forward_driver_methods!();
    async fn resolve(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
        (**self).resolve(path).await
    }
}

#[cfg(feature = "alloc")]
impl<F: FsDriver + ?Sized> FsDriver for alloc::boxed::Box<F> {
    type DeviceError = F::DeviceError;
    forward_driver_methods!();
    async fn resolve(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
        (**self).resolve(path).await
    }
}

/// A shared filesystem is also a driver, so the path layer is written once
/// over [`FsDriver`].
impl<F: FileSystem + ?Sized> FsDriver for &F {
    type DeviceError = F::DeviceError;
    forward_driver_methods!();
    async fn resolve(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
        (**self).resolve(path).await
    }
}

impl<F: FileSystem + ?Sized> FileSystem for &F {
    type DeviceError = F::DeviceError;
    forward_fs_methods!();
}

impl<F: FileSystem + ?Sized> FileSystem for &mut F {
    type DeviceError = F::DeviceError;
    forward_fs_methods!();
}

#[cfg(feature = "alloc")]
impl<F: FileSystem + ?Sized> FileSystem for alloc::boxed::Box<F> {
    type DeviceError = F::DeviceError;
    forward_fs_methods!();
}

#[cfg(feature = "alloc")]
impl<F: FileSystem + ?Sized> FileSystem for alloc::sync::Arc<F> {
    type DeviceError = F::DeviceError;
    forward_fs_methods!();
}

#[cfg(feature = "alloc")]
local_only! {
    impl<F: FileSystem + ?Sized> FileSystem for alloc::rc::Rc<F> {
        type DeviceError = F::DeviceError;
        forward_fs_methods!();
    }
}

/// A [`FileSystem`] held by value and used as a driver. Owned handles
/// (`Arc`, `Rc`, [`Volume`]) call through it; it dereferences to the
/// filesystem.
#[derive(Debug, Clone)]
pub struct AsDriver<F>(F);

impl<F> AsDriver<F> {
    /// Wraps a filesystem.
    pub fn new(fs: F) -> Self {
        Self(fs)
    }

    /// Returns the filesystem.
    pub fn into_inner(self) -> F {
        self.0
    }
}

impl<F> core::ops::Deref for AsDriver<F> {
    type Target = F;

    fn deref(&self) -> &F {
        &self.0
    }
}

impl<F: FileSystem> FsDriver for AsDriver<F> {
    type DeviceError = F::DeviceError;
    forward_driver_methods!();
    async fn resolve(&mut self, path: &str) -> FsResult<NodeId, Self::DeviceError> {
        (**self).resolve(path).await
    }
}

}
