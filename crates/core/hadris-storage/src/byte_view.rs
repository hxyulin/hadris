/// Defines `ByteView` and its inherent methods in the calling module, over
/// that module's `BlockDevice`, passing the source through `$mode` (the
/// mode's `io_transform`, or `keep_async` for the poll-based mode). The
/// stream trait impls stay with each mode.
macro_rules! byte_view {
    ($mode:ident $(, #[$doc:meta])*) => {
        $mode! {

/// Byte-granular access to a block device.
///
/// Reads and writes may start and end anywhere. Partial blocks are handled by
/// read-modify-write through a one-block scratch buffer, which keeps the last
/// partial block it read or wrote, so small accesses within one block cost
/// one device read. A write that covers that block replaces it, and a failed
/// or cancelled access forgets it. Changes made to the device other than
/// through the view need [`get_mut`](Self::get_mut), which forgets it too.
$(#[$doc])*
#[derive(Debug)]
pub struct ByteView<D> {
    inner: D,
    position: u64,
    scratch: crate::scratch::Scratch,
    cached: Option<u64>,
}

impl<D: BlockDevice> ByteView<D> {
    /// Wraps a device.
    ///
    /// Without `alloc`, block sizes above 4096 bytes fail with
    /// [`ErrorKind::Unsupported`](hadris_io::ErrorKind::Unsupported) on the
    /// first partial-block access.
    pub fn new(inner: D) -> Self {
        Self {
            inner,
            position: 0,
            scratch: crate::scratch::Scratch::new(),
            cached: None,
        }
    }

    /// Device length in bytes.
    pub fn len(&self) -> u64 {
        self.inner
            .block_count()
            .saturating_mul(u64::from(self.inner.block_size().get()))
    }

    /// Whether the device has no blocks.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Recovers the device.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrows the device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    /// Mutably borrows the device, forgetting the block the view keeps.
    pub fn get_mut(&mut self) -> &mut D {
        self.cached = None;
        &mut self.inner
    }

    /// Fills `buf` from byte `offset`.
    ///
    /// A range past the end of the device fails with
    /// [`ErrorKind::InvalidInput`](hadris_io::ErrorKind::InvalidInput).
    pub async fn read_at(
        &mut self,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<D::Error>> {
        if !self.fits(offset, buf.len()) {
            return Err(crate::device::past_end(offset));
        }
        let size = self.inner.block_size().get() as usize;
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done as u64;
            let block = pos / size as u64;
            let within = (pos % size as u64) as usize;
            let left = buf.len() - done;
            if within == 0 && left >= size {
                let whole = left / size * size;
                self.inner
                    .read_blocks(crate::BlockIndex::new(block), &mut buf[done..done + whole])
                    .await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                self.load(block, size).await?;
                let scratch = self
                    .scratch
                    .get(size)
                    .ok_or_else(crate::device::block_too_large)?;
                buf[done..done + n].copy_from_slice(&scratch[within..within + n]);
                done += n;
            }
        }
        Ok(())
    }

    /// Writes all of `buf` at byte `offset`.
    ///
    /// A range past the end of the device fails with
    /// [`ErrorKind::InvalidInput`](hadris_io::ErrorKind::InvalidInput).
    pub async fn write_at(
        &mut self,
        offset: u64,
        buf: &[u8],
    ) -> Result<(), hadris_io::Error<D::Error>> {
        if !self.fits(offset, buf.len()) {
            return Err(crate::device::past_end(offset));
        }
        let size = self.inner.block_size().get() as usize;
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done as u64;
            let block = pos / size as u64;
            let within = (pos % size as u64) as usize;
            let left = buf.len() - done;
            if within == 0 && left >= size {
                let whole = left / size * size;
                if self
                    .cached
                    .is_some_and(|cached| cached.wrapping_sub(block) < (whole / size) as u64)
                {
                    self.cached = None;
                }
                self.inner
                    .write_blocks(crate::BlockIndex::new(block), &buf[done..done + whole])
                    .await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                self.load(block, size).await?;
                self.cached = None;
                let scratch = self
                    .scratch
                    .get(size)
                    .ok_or_else(crate::device::block_too_large)?;
                scratch[within..within + n].copy_from_slice(&buf[done..done + n]);
                self.inner
                    .write_blocks(crate::BlockIndex::new(block), scratch)
                    .await?;
                self.cached = Some(block);
                done += n;
            }
        }
        Ok(())
    }

    async fn load(&mut self, block: u64, size: usize) -> Result<(), hadris_io::Error<D::Error>> {
        if self.cached == Some(block) {
            return Ok(());
        }
        self.cached = None;
        let scratch = self
            .scratch
            .get(size)
            .ok_or_else(crate::device::block_too_large)?;
        self.inner
            .read_blocks(crate::BlockIndex::new(block), scratch)
            .await?;
        self.cached = Some(block);
        Ok(())
    }

    fn fits(&self, offset: u64, len: usize) -> bool {
        offset
            .checked_add(len as u64)
            .is_some_and(|end| end <= self.len())
    }

    fn remaining(&self) -> usize {
        usize::try_from(self.len().saturating_sub(self.position)).unwrap_or(usize::MAX)
    }

    fn seek_to(&mut self, pos: hadris_io::SeekFrom) -> Result<u64, hadris_io::Error<D::Error>> {
        let position = pos.resolve(self.position, self.len()).ok_or(hadris_io::Error::new(
            hadris_io::ErrorKind::InvalidInput,
            "seek to a negative or overflowing position",
        ))?;
        self.position = position;
        Ok(position)
    }
}

impl<D: hadris_io::ErrorType> hadris_io::ErrorType for ByteView<D> {
    type Error = hadris_io::Error<D::Error>;
}

        }
    };
}

/// Passes poll-mode source through unchanged.
#[cfg(feature = "async")]
macro_rules! keep_async {
    ($($item:tt)*) => { $($item)* };
}
