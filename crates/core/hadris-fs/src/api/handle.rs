use super::super::io;
use super::volume::Volume;
use super::*;
use crate::{Error, OpenOptions};
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
        let n = self.vol.lock().await.read(self.node, self.pos, buf).await?;
        self.pos += n as u64;
        Ok(n)
    }

    /// Writes at the position, or at the end in append mode, and advances
    /// it.
    pub async fn write(&mut self, buf: &[u8]) -> FsResult<usize, F::DeviceError> {
        if !self.options.is_write() {
            return Err(ErrorKind::InvalidInput.into());
        }
        let mut fs = self.vol.lock().await;
        if self.options.is_append() {
            self.pos = fs.stat(self.node).await?.len();
        }
        let n = fs.write(self.node, self.pos, buf).await?;
        self.pos += n as u64;
        Ok(n)
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
        self.vol.lock().await.truncate(self.node, len).await
    }

    /// The file's metadata.
    pub async fn metadata(&self) -> FsResult<Metadata, F::DeviceError> {
        self.vol.lock().await.stat(self.node).await
    }

    /// Makes the file durable, as `fsync` does.
    pub async fn sync_all(&self) -> FsResult<(), F::DeviceError> {
        self.vol.lock().await.fsync(self.node).await
    }

    /// Closes the file, publishing its size and times. It does not flush the
    /// device; call [`sync_all`](Self::sync_all) first for durability.
    pub async fn close(mut self) -> FsResult<(), F::DeviceError> {
        self.open = false;
        let mut fs = self.vol.lock().await;
        let closed = fs.close(self.node).await;
        fs.forget(self.node, 1);
        closed
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

    /// The next entry, or `None` at the end or after an error.
    pub async fn next_entry(&mut self) -> Option<FsResult<DirEntry, F::DeviceError>> {
        if self.done {
            return None;
        }
        let next = self.vol.lock().await.readdir(self.dir, self.cursor).await;
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

}

sync_only! {
    impl<F: FileSystem> Iterator for ReadDir<F> {
        type Item = FsResult<DirEntry, F::DeviceError>;

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
