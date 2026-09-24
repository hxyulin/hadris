use super::*;

io_transform! {

/// An open file as plain data: node, position and mode.
///
/// Every method takes the driver, so a kernel file table can hold many of
/// these beside one `&mut` driver. It is `Copy` and does not close or forget
/// its node on drop; its owner calls [`close`](Self::close). While it is
/// open, removing the file's last name fails with [`ErrorKind::Busy`].
#[derive(Debug, Clone, Copy)]
pub struct OpenFile {
    node: NodeId,
    pos: u64,
    opts: OpenOptions,
    dirty: bool,
}

impl OpenFile {
    /// Opens `path` on `fs`, pins its node and marks it open.
    ///
    /// Fails with [`ErrorKind::ReadOnly`] before touching anything when
    /// `opts` writes and the filesystem is not writable, with
    /// [`ErrorKind::IsADirectory`] for a directory and with
    /// [`ErrorKind::Symlink`] for a symlink the resolver did not follow.
    pub async fn open<D: FsDriver + ?Sized>(
        fs: &mut D,
        path: &str,
        opts: OpenOptions,
    ) -> FsResult<Self, D::DeviceError> {
        let node = open_node(fs, path, opts).await?;
        Self::from_pinned(fs, node, opts).await
    }

    /// Opens a node the caller pinned, taking over the pin: on failure the
    /// node is forgotten.
    pub async fn from_pinned<D: FsDriver + ?Sized>(
        fs: &mut D,
        node: NodeId,
        opts: OpenOptions,
    ) -> FsResult<Self, D::DeviceError> {
        if let Err(err) = fs.open_node(node).await {
            fs.forget(node);
            return Err(err);
        }
        Ok(Self { node, pos: 0, opts, dirty: false })
    }

    /// The file's node.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Current position.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Current length.
    pub async fn len<D: FsDriver + ?Sized>(&self, fs: &mut D) -> FsResult<u64, D::DeviceError> {
        Ok(fs.node_metadata(self.node).await?.len())
    }

    /// Reads at the position and advances it.
    pub async fn read<D: FsDriver + ?Sized>(
        &mut self,
        fs: &mut D,
        buf: &mut [u8],
    ) -> FsResult<usize, D::DeviceError> {
        if !self.opts.is_read() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let n = fs.read_at(self.node, self.pos, buf).await?;
        self.pos += n as u64;
        Ok(n)
    }

    /// Writes at the position, or at the end in append mode, and advances it.
    pub async fn write<D: FsDriver + ?Sized>(
        &mut self,
        fs: &mut D,
        buf: &[u8],
    ) -> FsResult<usize, D::DeviceError> {
        if !self.opts.is_write() {
            return Err(ErrorKind::InvalidInput.into());
        }
        if self.opts.is_append() {
            self.pos = self.len(fs).await?;
        }
        let n = fs.write_at(self.node, self.pos, buf).await?;
        self.pos += n as u64;
        self.dirty = true;
        Ok(n)
    }

    /// Moves the position.
    pub async fn seek<D: FsDriver + ?Sized>(
        &mut self,
        fs: &mut D,
        pos: SeekFrom,
    ) -> FsResult<u64, D::DeviceError> {
        let len = if matches!(pos, SeekFrom::End(_)) { self.len(fs).await? } else { 0 };
        self.pos = pos.resolve(self.pos, len).ok_or(ErrorKind::InvalidInput)?;
        Ok(self.pos)
    }

    /// Makes the node durable, as `fsync` does.
    pub async fn sync_all<D: FsDriver + ?Sized>(&mut self, fs: &mut D) -> FsResult<(), D::DeviceError> {
        fs.sync_node(self.node).await?;
        self.dirty = false;
        Ok(())
    }

    /// Publishes the node's metadata if it was written, closes and forgets
    /// it, and returns `prior` or the publish error. It does not flush the
    /// device; call [`sync_all`](Self::sync_all) first, or `sync` on the
    /// filesystem, for durability.
    pub async fn close<D: FsDriver + ?Sized>(
        self,
        fs: &mut D,
        prior: FsResult<(), D::DeviceError>,
    ) -> FsResult<(), D::DeviceError> {
        let synced = if self.dirty { fs.publish_node(self.node).await } else { Ok(()) };
        fs.close_node(self.node);
        fs.forget(self.node);
        prior.and(synced)
    }
}

/// How a handle reaches its filesystem.
///
/// Implemented for `&mut D` (raw tier), `&F` (shared tier), `Arc<F>` and
/// `Rc<F>` (owned tier) and [`Volume`] by value. Users name it only in
/// generic code over handles.
pub trait Access {
    /// The device's own error.
    type DeviceError: core::error::Error + Send + Sync + 'static;
    /// The driver the handle owns and calls through.
    type Driver: FsDriver<DeviceError = Self::DeviceError>;
    /// Converts into the driver.
    fn into_driver(self) -> Self::Driver;
}

impl<'a, D: FsDriver + ?Sized> Access for &'a mut D {
    type DeviceError = D::DeviceError;
    type Driver = &'a mut D;

    fn into_driver(self) -> &'a mut D {
        self
    }
}

impl<'a, F: FileSystem + ?Sized> Access for &'a F {
    type DeviceError = F::DeviceError;
    type Driver = &'a F;

    fn into_driver(self) -> &'a F {
        self
    }
}

#[cfg(feature = "alloc")]
impl<F: FileSystem + ?Sized> Access for alloc::sync::Arc<F> {
    type DeviceError = F::DeviceError;
    type Driver = AsDriver<Self>;

    fn into_driver(self) -> AsDriver<Self> {
        AsDriver::new(self)
    }
}

#[cfg(feature = "alloc")]
local_only! {
    impl<F: FileSystem + ?Sized> Access for alloc::rc::Rc<F> {
        type DeviceError = F::DeviceError;
        type Driver = AsDriver<Self>;

        fn into_driver(self) -> AsDriver<Self> {
            AsDriver::new(self)
        }
    }
}

impl<F: FsDriver, K: LockKind> Access for Volume<F, K> {
    type DeviceError = F::DeviceError;
    type Driver = AsDriver<Self>;

    fn into_driver(self) -> AsDriver<Self> {
        AsDriver::new(self)
    }
}

/// An open file on any tier: `File<&mut FatFs<_>>` raw, `File<&Volume<..>>`
/// shared, `File<Arc<Volume<..>>>` owned.
///
/// Implements the `hadris-io` traits and, with `std` in sync builds, the
/// `std::io` traits. The node is open while the handle lives, so removing
/// the file's last name fails with [`ErrorKind::Busy`]. Dropping it closes
/// and forgets the node without flushing; call [`close`](Self::close) to see
/// write errors.
#[must_use = "dropping a file closes its node without reporting errors"]
pub struct File<A: Access> {
    fs: A::Driver,
    file: OpenFile,
}

impl<A: Access> File<A> {
    /// Opens `path` through `fs`.
    pub async fn open(fs: A, path: &str, opts: OpenOptions) -> FsResult<Self, A::DeviceError> {
        let mut fs = fs.into_driver();
        let file = OpenFile::open(&mut fs, path, opts).await?;
        Ok(Self { fs, file })
    }

    /// Opens a node the caller pinned, taking over the pin: on failure the
    /// node is forgotten. The file closes and forgets it on drop.
    pub async fn from_pinned(fs: A, node: NodeId, opts: OpenOptions) -> FsResult<Self, A::DeviceError> {
        let mut fs = fs.into_driver();
        let file = OpenFile::from_pinned(&mut fs, node, opts).await?;
        Ok(Self { fs, file })
    }

    /// The file's node.
    pub fn node(&self) -> NodeId {
        self.file.node
    }

    /// Current position.
    pub fn position(&self) -> u64 {
        self.file.pos
    }

    /// The filesystem, for other calls while the file is open.
    pub fn driver(&mut self) -> &mut A::Driver {
        &mut self.fs
    }

    /// Current length.
    pub async fn len(&mut self) -> FsResult<u64, A::DeviceError> {
        self.file.len(&mut self.fs).await
    }

    /// Whether the file is empty.
    pub async fn is_empty(&mut self) -> FsResult<bool, A::DeviceError> {
        Ok(self.len().await? == 0)
    }

    /// Truncates or extends the file.
    pub async fn set_len(&mut self, len: u64) -> FsResult<(), A::DeviceError> {
        self.fs.set_len(self.file.node, len).await?;
        self.file.dirty = true;
        Ok(())
    }

    /// Makes the file durable, as `fsync` does: its data and metadata, then
    /// a device flush.
    pub async fn sync_all(&mut self) -> FsResult<(), A::DeviceError> {
        self.file.sync_all(&mut self.fs).await
    }

    /// Publishes the file's metadata if it was written, then closes and
    /// forgets it. It does not flush the device; call
    /// [`sync_all`](Self::sync_all) first for durability.
    pub async fn close(mut self) -> FsResult<(), A::DeviceError> {
        if self.file.dirty {
            self.fs.publish_node(self.file.node).await?;
        }
        Ok(())
    }
}

impl<A: Access> core::fmt::Debug for File<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("File").field("file", &self.file).finish_non_exhaustive()
    }
}

impl<A: Access> Drop for File<A> {
    fn drop(&mut self) {
        self.fs.close_node(self.file.node);
        self.fs.forget(self.file.node);
    }
}

impl<A: Access> hadris_io::ErrorType for File<A> {
    type Error = Error<A::DeviceError>;
}

impl<A: Access> io::Read for File<A> {
    async fn read(&mut self, buf: &mut [u8]) -> FsResult<usize, A::DeviceError> {
        self.file.read(&mut self.fs, buf).await
    }
}

impl<A: Access> io::Write for File<A> {
    async fn write(&mut self, buf: &[u8]) -> FsResult<usize, A::DeviceError> {
        self.file.write(&mut self.fs, buf).await
    }

    /// Publishes the file's metadata, as [`close`](File::close) does,
    /// without flushing the device.
    async fn flush(&mut self) -> FsResult<(), A::DeviceError> {
        self.fs.publish_node(self.file.node).await
    }
}

impl<A: Access> io::Seek for File<A> {
    async fn seek(&mut self, pos: SeekFrom) -> FsResult<u64, A::DeviceError> {
        self.file.seek(&mut self.fs, pos).await
    }
}

/// An open directory on any tier. An `Iterator` in sync builds; call
/// [`next_entry`](Self::next_entry) in async builds. Fuses after an error.
pub struct Dir<A: Access> {
    fs: A::Driver,
    node: NodeId,
    cursor: DirCursor,
    done: bool,
}

impl<A: Access> Dir<A> {
    /// Opens the directory at `path` through `fs`.
    pub async fn open(fs: A, path: &str) -> FsResult<Self, A::DeviceError> {
        let mut fs = fs.into_driver();
        let node = fs.resolve(path).await?;
        let mut dir = Self { fs, node, cursor: DirCursor::start(), done: false };
        if !dir.fs.node_metadata(node).await?.file_type().is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok(dir)
    }

    /// The directory's node.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// The position, for resuming a listing later with the same node.
    pub fn cursor(&self) -> DirCursor {
        self.cursor
    }

    /// The next entry, or `None` at the end or after an error.
    pub async fn next_entry(&mut self) -> Option<FsResult<DirItem, A::DeviceError>> {
        if self.done {
            return None;
        }
        let mut name = NameBuf::new();
        match self.fs.read_dir_entry(self.node, &mut self.cursor, &mut name).await {
            Ok(Some(entry)) => match DirItem::new(name, entry) {
                Some(item) => Some(Ok(item)),
                None => {
                    self.done = true;
                    Some(Err(ErrorKind::Corrupt.into()))
                }
            },
            Ok(None) => {
                self.done = true;
                None
            }
            Err(err) => {
                self.done = true;
                Some(Err(err))
            }
        }
    }
}

impl<A: Access> core::fmt::Debug for Dir<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Dir")
            .field("node", &self.node)
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

impl<A: Access> Drop for Dir<A> {
    fn drop(&mut self) {
        self.fs.forget(self.node);
    }
}

sync_only! {
    impl<A: Access> Iterator for Dir<A> {
        type Item = FsResult<DirItem, A::DeviceError>;

        fn next(&mut self) -> Option<Self::Item> {
            self.next_entry()
        }
    }

    #[cfg(feature = "std")]
    impl<A: Access> std::io::Read for File<A> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            Ok(io::Read::read(self, buf)?)
        }
    }

    #[cfg(feature = "std")]
    impl<A: Access> std::io::Write for File<A> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(io::Write::write(self, buf)?)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(io::Write::flush(self)?)
        }
    }

    #[cfg(feature = "std")]
    impl<A: Access> std::io::Seek for File<A> {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            Ok(io::Seek::seek(self, pos.into())?)
        }
    }
}

}
