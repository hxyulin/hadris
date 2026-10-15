use super::super::io;
use super::volume::Volume;
use super::*;
use crate::{Error, OpenOptions};
use alloc::vec::Vec;
use hadris_io::SeekFrom;

io_transform! {

/// An open file of a [`Volume`], with its own position.
///
/// Several handles on one node see each other's writes and size at once,
/// and a handle keeps working after its file is renamed or moved. The file
/// is open while the handle lives, so removing its last name fails with
/// [`ErrorKind::Busy`].
///
/// [`close`](Self::close) publishes the file's size and times and reports
/// errors. In the sync mode dropping the handle closes it too, ignoring
/// errors; the async mode cannot await in `Drop`, so there a dropped file is
/// closed by the next call on the volume. It implements the `hadris-io`
/// traits, and in the sync mode the `std::io` traits.
#[must_use = "dropping a file closes it without reporting errors"]
pub struct File<F: FileSystem> {
    vol: Volume<F>,
    node: NodeId,
    pos: u64,
    options: OpenOptions,
    open: bool,
}

impl<F: FileSystem> File<F> {
    pub(super) fn new(vol: Volume<F>, node: NodeId, options: OpenOptions) -> Self {
        Self { vol, node, pos: 0, options, open: true }
    }

    /// The file's node.
    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Reads at the position and advances it.
    pub async fn read(&mut self, buf: &mut [u8]) -> FsResult<usize, F::DeviceError> {
        if !self.options.is_read() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let (vol, node, pos) = (&self.vol, self.node, self.pos);
        let n = watched!(vol, { vol.enter().await?.read(node, pos, buf).await })?;
        self.pos += n as u64;
        Ok(n)
    }

    /// Writes at the position, or at the end in append mode, and advances
    /// it by the successful byte count. An error may leave partial changes
    /// without reporting progress; see [`FileSystem::write`].
    pub async fn write(&mut self, buf: &[u8]) -> FsResult<usize, F::DeviceError> {
        if !self.options.is_write() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let (vol, node, append, pos) = (&self.vol, self.node, self.options.is_append(), &mut self.pos);
        watched!(vol, {
            let mut fs = vol.enter().await?;
            if append {
                *pos = fs.stat(node).await?.len();
            }
            let n = fs.write(node, *pos, buf).await?;
            *pos += n as u64;
            Ok(n)
        })
    }

    /// Reads at `offset` without moving the position, as `pread`.
    pub async fn read_at(&self, offset: u64, buf: &mut [u8]) -> FsResult<usize, F::DeviceError> {
        if !self.options.is_read() {
            return Err(ErrorKind::InvalidInput.into());
        }
        watched!(self.vol, { self.vol.enter().await?.read(self.node, offset, buf).await })
    }

    /// Writes at `offset` without moving the position, as `pwrite`, also in
    /// append mode. Returns the successful byte count; see
    /// [`FileSystem::write`] for errors.
    pub async fn write_at(&self, offset: u64, buf: &[u8]) -> FsResult<usize, F::DeviceError> {
        if !self.options.is_write() {
            return Err(ErrorKind::InvalidInput.into());
        }
        watched!(self.vol, { self.vol.enter().await?.write(self.node, offset, buf).await })
    }

    /// Changes the file's times, permissions, owner or attributes. It needs
    /// no write access, as `fchmod` does not, but a read-only mount refuses
    /// it.
    pub async fn set_attr(&self, changes: &SetAttr) -> FsResult<(), F::DeviceError> {
        watched!(self.vol, { self.vol.enter().await?.setattr(self.node, changes).await })
    }

    /// Moves the position and returns it.
    pub async fn seek(&mut self, pos: SeekFrom) -> FsResult<u64, F::DeviceError> {
        let len = match pos {
            SeekFrom::End(_) => self.metadata().await?.len(),
            _ => 0,
        };
        self.pos = pos.resolve(self.pos, len).ok_or(ErrorKind::InvalidInput)?;
        Ok(self.pos)
    }

    /// Truncates or extends the file.
    pub async fn set_len(&mut self, len: u64) -> FsResult<(), F::DeviceError> {
        if !self.options.is_write() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let (vol, node) = (&self.vol, self.node);
        watched!(vol, { vol.enter().await?.truncate(node, len).await })
    }

    /// The file's metadata.
    pub async fn metadata(&self) -> FsResult<Metadata, F::DeviceError> {
        watched!(self.vol, { self.vol.enter().await?.stat(self.node).await })
    }

    /// Makes the file durable, as `fsync` does.
    pub async fn sync_all(&self) -> FsResult<(), F::DeviceError> {
        watched!(self.vol, { self.vol.enter().await?.fsync(self.node).await })
    }

    /// Closes the file, publishing its size and times. It does not flush the
    /// device; call [`sync_all`](Self::sync_all) first for durability.
    pub async fn close(mut self) -> FsResult<(), F::DeviceError> {
        let (vol, node, open) = (&self.vol, self.node, &mut self.open);
        watched!(vol, {
            let mut fs = vol.enter().await?;
            let mut held = vol.hold_file(node);
            *open = false;
            let closed = fs.close(node).await;
            held.closed();
            held.forget(&mut *fs);
            closed
        })
    }
}

impl<F: FileSystem> core::fmt::Debug for File<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("File")
            .field("node", &self.node)
            .field("pos", &self.pos)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl<F: FileSystem> Drop for File<F> {
    fn drop(&mut self) {
        if self.open {
            self.vol.release(self.node, true);
        }
    }
}

impl<F: FileSystem> hadris_io::ErrorType for File<F> {
    type Error = Error<F::DeviceError>;
}

impl<F: FileSystem> io::Read for File<F> {
    async fn read(&mut self, buf: &mut [u8]) -> FsResult<usize, F::DeviceError> {
        File::read(self, buf).await
    }
}

impl<F: FileSystem> io::Write for File<F> {
    async fn write(&mut self, buf: &[u8]) -> FsResult<usize, F::DeviceError> {
        File::write(self, buf).await
    }

    async fn flush(&mut self) -> FsResult<(), F::DeviceError> {
        Ok(())
    }
}

impl<F: FileSystem> io::Seek for File<F> {
    async fn seek(&mut self, pos: SeekFrom) -> FsResult<u64, F::DeviceError> {
        File::seek(self, pos).await
    }
}

/// A listing of a directory of a [`Volume`].
///
/// An `Iterator` in the sync mode; call [`next_entry`](Self::next_entry) in
/// the async mode. Fuses after an error.
pub struct ReadDir<F: FileSystem> {
    vol: Volume<F>,
    dir: NodeId,
    cursor: DirCursor,
    done: bool,
}

impl<F: FileSystem> ReadDir<F> {
    pub(super) fn new(vol: Volume<F>, dir: NodeId) -> Self {
        Self { vol, dir, cursor: DirCursor::START, done: false }
    }

    /// Opens the file `entry`, listed by this `ReadDir`, as
    /// [`Volume::open_node`] does.
    pub async fn open(&self, entry: &DirEntry, options: OpenOptions) -> FsResult<File<F>, F::DeviceError> {
        self.vol.open_node(self.dir, entry, options).await
    }

    /// The next entry, or `None` at the end or after an error.
    pub async fn next_entry(&mut self) -> Option<FsResult<DirEntry, F::DeviceError>> {
        if self.done {
            return None;
        }
        let (vol, dir, cursor) = (&self.vol, self.dir, self.cursor);
        let next = watched!(vol, {
            match vol.enter().await {
                Ok(mut fs) => fs.readdir(dir, cursor).await,
                Err(err) => Err(err),
            }
        });
        match next {
            Ok(Some(entry)) => {
                self.cursor = entry.next_cursor();
                Some(Ok(entry))
            }
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

impl<F: FileSystem> core::fmt::Debug for ReadDir<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReadDir")
            .field("dir", &self.dir)
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

impl<F: FileSystem> Drop for ReadDir<F> {
    fn drop(&mut self) {
        self.vol.release(self.dir, false);
    }
}

/// A depth-first walk below a directory of a [`Volume`], from
/// [`Volume::walk`]: each directory comes before its children, which come
/// in `readdir` order, and every entry carries its path.
///
/// An `Iterator` in the sync mode; call [`next_entry`](Self::next_entry) in
/// the async mode. The walk pins the directory it started from and nothing
/// below it, and locks the volume once per step. The ids it lists stay
/// valid while the tree does not change, so a change made between steps may
/// end the walk with [`ErrorKind::NotFound`]. A directory that is one of
/// its own ancestors ends it with [`ErrorKind::Corrupt`], and one more than
/// 1024 levels deep with [`ErrorKind::LimitExceeded`]. Fuses after an
/// error.
pub struct WalkDir<F: FileSystem> {
    vol: Volume<F>,
    top: NodeId,
    walk: Walk<'static>,
    path: Vec<u8>,
    ends: Vec<usize>,
    parents: Vec<NodeId>,
}

/// One entry of a [`WalkDir`].
#[derive(Clone)]
pub struct WalkDirEntry {
    path: Vec<u8>,
    dir: NodeId,
    entry: DirEntry,
    depth: u32,
}

impl WalkDirEntry {
    /// The path of the entry: the walked path, `/` and the names below it.
    pub fn path(&self) -> &[u8] {
        &self.path
    }

    /// The directory that lists the entry, for [`Volume::open_node`].
    pub fn dir(&self) -> NodeId {
        self.dir
    }

    /// The directory entry, with the name, node and metadata `readdir`
    /// gave.
    pub fn entry(&self) -> &DirEntry {
        &self.entry
    }

    /// The depth, 1 for the children of the walked directory.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Takes the path.
    pub fn into_path(self) -> Vec<u8> {
        self.path
    }
}

impl core::fmt::Debug for WalkDirEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WalkDirEntry")
            .field("path", &crate::name::Name::from_bytes(&self.path))
            .field("dir", &self.dir)
            .field("entry", &self.entry)
            .field("depth", &self.depth)
            .finish()
    }
}

impl<F: FileSystem> WalkDir<F> {
    pub(super) fn new(vol: Volume<F>, top: NodeId, path: &[u8]) -> Self {
        let mut base = path.to_vec();
        while base.last() == Some(&b'/') {
            base.pop();
        }
        Self {
            vol,
            top,
            walk: Walk::new(top),
            ends: alloc::vec![base.len()],
            parents: alloc::vec![top],
            path: base,
        }
    }

    /// Stops the walk from descending into the directory just returned.
    pub fn skip_dir(&mut self) {
        self.walk.skip_dir();
    }

    /// The next entry, or `None` at the end or after an error.
    pub async fn next_entry(&mut self) -> Option<FsResult<WalkDirEntry, F::DeviceError>> {
        let (vol, walk) = (&self.vol, &mut self.walk);
        let next = watched!(vol, {
            match vol.enter().await {
                Ok(mut fs) => walk.next(&mut *fs).await,
                Err(err) => Err(err),
            }
        });
        let found = match next {
            Ok(Some(found)) => found,
            Ok(None) => return None,
            Err(err) => return Some(Err(err)),
        };
        let depth = found.depth();
        let entry = *found.entry();
        let parent = depth as usize - 1;
        self.ends.truncate(depth as usize);
        self.parents.truncate(depth as usize);
        self.path.truncate(self.ends[parent]);
        self.path.push(b'/');
        self.path.extend_from_slice(entry.name().as_bytes());
        let dir = self.parents[parent];
        if entry.file_type().is_dir() {
            self.ends.push(self.path.len());
            self.parents.push(entry.node());
        }
        Some(Ok(WalkDirEntry { path: self.path.clone(), dir, entry, depth }))
    }
}

impl<F: FileSystem> core::fmt::Debug for WalkDir<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WalkDir")
            .field("top", &self.top)
            .field("walk", &self.walk)
            .finish_non_exhaustive()
    }
}

impl<F: FileSystem> Drop for WalkDir<F> {
    fn drop(&mut self) {
        self.vol.release(self.top, false);
    }
}

}

sync_only! {
    impl<F: FileSystem> Iterator for ReadDir<F> {
        type Item = FsResult<DirEntry, F::DeviceError>;

        fn next(&mut self) -> Option<Self::Item> {
            self.next_entry()
        }
    }

    impl<F: FileSystem> Iterator for WalkDir<F> {
        type Item = FsResult<WalkDirEntry, F::DeviceError>;

        fn next(&mut self) -> Option<Self::Item> {
            self.next_entry()
        }
    }

    impl<F: FileSystem> std::io::Read for File<F> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            Ok(File::read(self, buf)?)
        }
    }

    impl<F: FileSystem> std::io::Write for File<F> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(File::write(self, buf)?)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<F: FileSystem> std::io::Seek for File<F> {
        fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
            Ok(File::seek(self, pos.into())?)
        }
    }
}
