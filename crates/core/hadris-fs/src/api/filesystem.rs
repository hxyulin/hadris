use super::*;

io_transform! {

/// A mounted filesystem: node ids, no paths, no lock.
///
/// Every format implements it, and every method takes `&mut self`, so a
/// kernel, a FUSE layer or a format tool calls it directly and shares a
/// volume through `Volume` only when it wants to. The read half is
/// required; the write half defaults to [`ErrorKind::ReadOnly`], so a
/// read-only format implements only the read half and gains writes later by
/// overriding them.
///
/// The contract:
///
/// - `lookup`, `parent`, `resolve`, `create` and `mkdir` pin the node they
///   return; `forget(node, count)` drops `count` pins, as FUSE
///   `forget(nlookup)`. The root is always pinned, and forgetting it does
///   nothing. `forget` never fails, never blocks and does no I/O.
/// - A pin never blocks a removal. A removed pinned node keeps its id until
///   its last `forget`, and every other method answers
///   [`ErrorKind::NotFound`] for it. `open` and `close` bracket reads and
///   writes; removing the last name of an open node fails with
///   [`ErrorKind::Busy`].
/// - `open` fails with [`ErrorKind::IsADirectory`] for a directory, with
///   [`ErrorKind::Symlink`] for a symlink, and with
///   [`ErrorKind::ReadOnly`] for [`OpenMode::Write`] on a read-only mount.
///   After that the caller's open mode is trusted.
/// - `close` publishes the node's size and times. `fsync` makes one node
///   durable and flushes the device. `sync` writes every piece of cached
///   metadata and flushes the device.
/// - `readdir` returns the entry at or after `from`, or `None` at the end;
///   the caller continues from the entry's `next_cursor`. `.` and `..` are
///   never listed, and every cursor stays at or below
///   [`DirCursor::MAX_RAW`]. Entries are not pinned, but a listed id equals
///   what `lookup` of that name returns until the directory changes.
/// - Names that are empty, `.` or `..`, or contain `/` or NUL fail with
///   [`ErrorKind::InvalidInput`]. `lookup` follows the format's rules for
///   case and aliases, and `create` and `mkdir` fail with
///   [`ErrorKind::AlreadyExists`] when any name matches.
/// - A directory has one id however it is reached.
/// - `create` and `mkdir` ignore the fields of `attrs` the format cannot
///   store. `setattr` succeeds when the value the format would report after
///   storing it equals what was asked, after rounding to the field's
///   resolution and deriving dependent bits, and fails with
///   [`ErrorKind::Unsupported`] for a field the format does not store.
/// - `read` is short at the end of a file and returns zeros for unwritten
///   ranges; `write` past the end fills the gap with zeros. `rename` keeps
///   the moved node's id.
/// - Reads never change times. Drivers validate arguments and known
///   read-only or unsupported operations before mutation. These preflight
///   rejections leave logical contents and metadata unchanged.
/// - Mutation is not transactional. An I/O failure, a device becoming
///   read-only, or cancellation after mutation starts may leave partial
///   changes in memory and on disk. An error kind alone does not establish
///   whether mutation started; rollback is not guaranteed. A driver may
///   also enter read-only mode after a device refusal.
/// - `write` reports confirmed progress only through `Ok(n)`. An `Err`
///   carries no byte count and must not be treated as proof that no bytes
///   were written. Durability still requires `fsync` or `sync`.
/// - In the async mode a call whose future is dropped before it completes
///   leaves no newly acquired pin. This resource guarantee does not imply
///   rollback of filesystem changes. Compound helpers may have completed
///   earlier operations before a later one fails.
///
/// # Adding methods
///
/// Methods added after 3.0 have a default body: `Unsupported` for queries,
/// `ReadOnly` for writes, or a composition of existing methods, with a
/// matching [`Capabilities`] flag when callers need to ask first.
pub trait FileSystem {
    /// The device's own error, returned inside [`Error`](crate::Error).
    type DeviceError: core::error::Error + Send + Sync + 'static;

    /// What this filesystem supports, for this mount.
    fn capabilities(&self) -> Capabilities;

    /// The root directory. Always pinned.
    fn root(&self) -> NodeId;

    /// Space usage.
    async fn statfs(&mut self) -> FsResult<FsStats, Self::DeviceError>;

    /// Writes the volume label into `buf` as UTF-8 and returns it, or `None`
    /// when the volume has none. 384 bytes hold every format's label;
    /// [`ErrorKind::LimitExceeded`] if `buf` is too small.
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, Self::DeviceError>;

    /// Finds `name` in `dir` and pins the result.
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError>;

    /// Drops `count` pins of `node`. Never fails, never blocks and does no
    /// I/O, so `Drop` can call it.
    fn forget(&mut self, node: NodeId, count: u64);

    /// The directory containing `dir`, pinned. The root is its own parent.
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError>;

    /// Resolves `path` from the root with the policy `how` and pins the
    /// result. The default composes `lookup`, `stat`, `parent` and
    /// `readlink`; a driver overrides it only to resolve faster.
    async fn resolve(&mut self, path: &[u8], how: Resolve) -> FsResult<NodeId, Self::DeviceError> {
        paths::resolve(self, path, how).await
    }

    /// Metadata of a pinned node, or of an entry just listed by `readdir`.
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError>;

    /// The entry of `dir` at or after `from`, or `None` at the end.
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, Self::DeviceError>;

    /// Writes a symlink's target to the start of `buf` and returns that part.
    /// [`ErrorKind::LimitExceeded`] if `buf` is too small, and
    /// [`ErrorKind::InvalidInput`] if `node` is not a symlink.
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], Self::DeviceError>;

    /// Opens a pinned file for `mode`. The caller keeps its pin while the
    /// file is open.
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), Self::DeviceError>;

    /// Ends one `open` and publishes the node's size and times, without
    /// flushing the device.
    async fn close(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError>;

    /// Reads from an open file at `offset`. Returns 0 at or past the end.
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError>;

    /// Changes times, permissions, owner or attributes.
    async fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> FsResult<(), Self::DeviceError> {
        let _ = (node, changes);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Writes to an open file at `offset`, growing it as needed.
    ///
    /// `Ok(n)` reports bytes written; a short write is allowed. On `Err`,
    /// data or metadata may already have changed and progress is unknown.
    /// Retrying the whole request is not guaranteed to be safe.
    async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, Self::DeviceError> {
        let _ = (node, offset, buf);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Truncates or extends a file.
    async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
        let _ = (node, len);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Makes one node durable: writes its pending data and metadata, then
    /// flushes the device.
    async fn fsync(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
        let _ = node;
        Err(ErrorKind::ReadOnly.into())
    }

    /// Creates the empty file `name` in `dir` with `attrs` and pins it.
    async fn create(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
        let _ = (dir, name, attrs);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Creates the empty directory `name` in `dir` with `attrs` and pins it.
    async fn mkdir(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
        let _ = (dir, name, attrs);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Removes `name` from `dir`. A directory fails with
    /// [`ErrorKind::IsADirectory`], the last name of an open node with
    /// [`ErrorKind::Busy`].
    async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        let _ = (dir, name);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Removes the empty directory `name` from `dir`. Anything but a
    /// directory fails with [`ErrorKind::NotADirectory`], a directory with
    /// entries with [`ErrorKind::DirectoryNotEmpty`].
    async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
        let _ = (dir, name);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`. The moved node keeps
    /// its id. Replacing an existing `to` removes it as `unlink` or `rmdir`
    /// would, so an open target fails with [`ErrorKind::Busy`].
    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> FsResult<(), Self::DeviceError> {
        let _ = (from_dir, from, to_dir, to, mode);
        Err(ErrorKind::ReadOnly.into())
    }

    /// Writes every piece of cached metadata and flushes the device.
    async fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
        Err(ErrorKind::ReadOnly.into())
    }
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
        async fn statfs(&mut self) -> FsResult<FsStats, Self::DeviceError> {
            (**self).statfs().await
        }
        async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, Self::DeviceError> {
            (**self).label(buf).await
        }
        async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, Self::DeviceError> {
            (**self).lookup(dir, name).await
        }
        fn forget(&mut self, node: NodeId, count: u64) {
            (**self).forget(node, count)
        }
        async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, Self::DeviceError> {
            (**self).parent(dir).await
        }
        async fn resolve(&mut self, path: &[u8], how: Resolve) -> FsResult<NodeId, Self::DeviceError> {
            (**self).resolve(path, how).await
        }
        async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, Self::DeviceError> {
            (**self).stat(node).await
        }
        async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, Self::DeviceError> {
            (**self).readdir(dir, from).await
        }
        async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], Self::DeviceError> {
            (**self).readlink(node, buf).await
        }
        async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), Self::DeviceError> {
            (**self).open(node, mode).await
        }
        async fn close(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).close(node).await
        }
        async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).read(node, offset, buf).await
        }
        async fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> FsResult<(), Self::DeviceError> {
            (**self).setattr(node, changes).await
        }
        async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, Self::DeviceError> {
            (**self).write(node, offset, buf).await
        }
        async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), Self::DeviceError> {
            (**self).truncate(node, len).await
        }
        async fn fsync(&mut self, node: NodeId) -> FsResult<(), Self::DeviceError> {
            (**self).fsync(node).await
        }
        async fn create(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
            (**self).create(dir, name, attrs).await
        }
        async fn mkdir(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, Self::DeviceError> {
            (**self).mkdir(dir, name, attrs).await
        }
        async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
            (**self).unlink(dir, name).await
        }
        async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), Self::DeviceError> {
            (**self).rmdir(dir, name).await
        }
        async fn rename(
            &mut self,
            from_dir: NodeId,
            from: &Name,
            to_dir: NodeId,
            to: &Name,
            mode: RenameMode,
        ) -> FsResult<(), Self::DeviceError> {
            (**self).rename(from_dir, from, to_dir, to, mode).await
        }
        async fn sync(&mut self) -> FsResult<(), Self::DeviceError> {
            (**self).sync().await
        }
    };
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

}
