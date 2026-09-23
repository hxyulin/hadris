use super::{Read, Seek, Write};
use crate::device::{byte_offset, check_blocks, read_only};
use crate::{Access, BlockIndex, BlockSize, MemBuffer, MemDevice, ReadOnly};
use hadris_io::{Error, ErrorKind, Result, SeekFrom};

io_transform! {

/// A device addressed in whole logical blocks.
///
/// Buffers passed to [`read_blocks`](Self::read_blocks) and
/// [`write_blocks`](Self::write_blocks) must be a whole number of blocks, and
/// the request must lie within the device. A read-only device implements only
/// `block_size`, `block_count` and `read_blocks`.
pub trait BlockDevice {
    /// Size of one block.
    fn block_size(&self) -> BlockSize;

    /// Number of addressable blocks.
    fn block_count(&self) -> u64;

    /// Whether the device accepts writes.
    fn access(&self) -> Access {
        Access::ReadOnly
    }

    /// Read whole blocks starting at `first`.
    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()>;

    /// Write whole blocks starting at `first`.
    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> {
        let _ = (first, buf);
        Err(read_only())
    }

    /// Flush buffered writes to the underlying storage.
    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<D: BlockDevice + ?Sized> BlockDevice for &mut D {
    fn block_size(&self) -> BlockSize {
        D::block_size(self)
    }

    fn block_count(&self) -> u64 {
        D::block_count(self)
    }

    fn access(&self) -> Access {
        D::access(self)
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()> {
        D::read_blocks(self, first, buf).await
    }

    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> {
        D::write_blocks(self, first, buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        D::flush(self).await
    }
}

#[cfg(feature = "alloc")]
impl<D: BlockDevice + ?Sized> BlockDevice for alloc::boxed::Box<D> {
    fn block_size(&self) -> BlockSize {
        D::block_size(self)
    }

    fn block_count(&self) -> u64 {
        D::block_count(self)
    }

    fn access(&self) -> Access {
        D::access(self)
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()> {
        D::read_blocks(self, first, buf).await
    }

    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> {
        D::write_blocks(self, first, buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        D::flush(self).await
    }
}

impl<B: MemBuffer> BlockDevice for MemDevice<B> {
    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        MemDevice::block_count(self)
    }

    fn access(&self) -> Access {
        if B::WRITABLE { Access::ReadWrite } else { Access::ReadOnly }
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()> {
        let range = self.range(first, buf.len())?;
        buf.copy_from_slice(&self.buffer.bytes()[range]);
        Ok(())
    }

    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> {
        let range = self.range(first, buf.len())?;
        let bytes = self.buffer.bytes_mut().ok_or_else(read_only)?;
        bytes[range].copy_from_slice(buf);
        Ok(())
    }
}

impl<T: Read> Read for ReadOnly<T> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf).await
    }
}

impl<T: Seek> Seek for ReadOnly<T> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        self.0.seek(pos).await
    }
}

/// The write half of a [`StreamDevice`] stream.
///
/// Implemented for every [`Write`] and for [`ReadOnly`], which rejects writes.
pub trait StreamWrite {
    /// Whether the stream accepts writes.
    fn stream_access(&self) -> Access;

    /// Write all of `buf`.
    async fn stream_write_all(&mut self, buf: &[u8]) -> Result<()>;

    /// Flush the stream.
    async fn stream_flush(&mut self) -> Result<()>;
}

impl<T: Write + ?Sized> StreamWrite for T {
    fn stream_access(&self) -> Access {
        Access::ReadWrite
    }

    async fn stream_write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.write_all(buf).await
    }

    async fn stream_flush(&mut self) -> Result<()> {
        self.flush().await
    }
}

impl<T> StreamWrite for ReadOnly<T> {
    fn stream_access(&self) -> Access {
        Access::ReadOnly
    }

    async fn stream_write_all(&mut self, _buf: &[u8]) -> Result<()> {
        Err(read_only())
    }

    async fn stream_flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A block device over a seekable byte stream.
///
/// Any block size works, so a FAT image with 512-byte sectors and an ISO with
/// 2048-byte sectors can both open over the same file. Wrap the stream in
/// [`ReadOnly`] when it does not implement [`Write`].
#[derive(Debug)]
pub struct StreamDevice<T> {
    inner: T,
    block_size: BlockSize,
    block_count: u64,
}

impl<T: Seek> StreamDevice<T> {
    /// Wrap `inner`, measuring its length by seeking to the end.
    ///
    /// Trailing bytes that do not fill a block are not addressable.
    pub async fn new(mut inner: T, block_size: BlockSize) -> Result<Self> {
        let len = inner.seek(SeekFrom::End(0)).await?;
        let block_count = len / u64::from(block_size.get());
        Ok(Self { inner, block_size, block_count })
    }
}

impl<T> StreamDevice<T> {
    /// Wrap `inner` with a known block count.
    pub fn with_block_count(inner: T, block_size: BlockSize, block_count: u64) -> Self {
        Self { inner, block_size, block_count }
    }

    /// Recover the stream.
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Borrow the stream.
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Mutably borrow the stream.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Read + Seek + StreamWrite> BlockDevice for StreamDevice<T> {
    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn access(&self) -> Access {
        self.inner.stream_access()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()> {
        check_blocks(self.block_size, self.block_count, first, buf.len())?;
        let offset = byte_offset(self.block_size, first)?;
        self.inner.seek(SeekFrom::Start(offset)).await?;
        self.inner.read_exact(buf).await
    }

    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> {
        if !self.inner.stream_access().is_writable() {
            return Err(read_only());
        }
        check_blocks(self.block_size, self.block_count, first, buf.len())?;
        let offset = byte_offset(self.block_size, first)?;
        self.inner.seek(SeekFrom::Start(offset)).await?;
        self.inner.stream_write_all(buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        self.inner.stream_flush().await
    }
}

/// A contiguous block range of another device.
///
/// `D` can be owned or `&mut`. Block 0 of the slice is block `first` of the
/// underlying device.
#[derive(Debug)]
pub struct Slice<D> {
    inner: D,
    first: u64,
    count: u64,
}

impl<D: BlockDevice> Slice<D> {
    /// Restrict `inner` to `count` blocks starting at `first`.
    pub fn new(inner: D, first: BlockIndex, count: u64) -> Result<Self> {
        match first.0.checked_add(count) {
            Some(end) if end <= inner.block_count() => Ok(Self { inner, first: first.0, count }),
            _ => Err(Error::new(ErrorKind::InvalidInput, "slice is out of range")),
        }
    }

    /// First block of the slice on the underlying device.
    pub fn first(&self) -> BlockIndex {
        BlockIndex(self.first)
    }

    /// Recover the underlying device.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrow the underlying device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    /// Mutably borrow the underlying device.
    pub fn get_mut(&mut self) -> &mut D {
        &mut self.inner
    }
}

impl<D: BlockDevice> BlockDevice for Slice<D> {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.count
    }

    fn access(&self) -> Access {
        self.inner.access()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()> {
        check_blocks(self.inner.block_size(), self.count, first, buf.len())?;
        self.inner.read_blocks(BlockIndex(self.first + first.0), buf).await
    }

    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> {
        check_blocks(self.inner.block_size(), self.count, first, buf.len())?;
        self.inner.write_blocks(BlockIndex(self.first + first.0), buf).await
    }

    async fn flush(&mut self) -> Result<()> {
        self.inner.flush().await
    }
}

/// A write-back LRU cache of whole blocks.
///
/// Writes stay in memory until [`flush`](BlockDevice::flush), eviction or
/// [`finish`](Self::finish). Dropping the cache discards unflushed writes.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct Cache<D> {
    inner: D,
    state: crate::cache::CacheState,
}

#[cfg(feature = "alloc")]
impl<D: BlockDevice> Cache<D> {
    /// Cache up to `capacity` blocks of `inner`. A capacity of zero is treated as one.
    pub fn new(inner: D, capacity: usize) -> Self {
        let block_size = inner.block_size().get() as usize;
        Self { inner, state: crate::cache::CacheState::new(capacity, block_size) }
    }

    /// Whether any cached block has unflushed writes.
    pub fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    /// Flush every dirty block and return the underlying device.
    pub async fn finish(mut self) -> Result<D> {
        self.flush().await?;
        Ok(self.inner)
    }

    /// Return the underlying device, discarding unflushed writes.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrow the underlying device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    async fn slot_for(&mut self, index: u64, load: bool) -> Result<usize> {
        if let Some(slot) = self.state.lookup(index) {
            return Ok(slot);
        }
        let slot = match self.state.victim() {
            Some(slot) => {
                if let Some(evicted) = self.state.dirty_index(slot) {
                    self.inner.write_blocks(BlockIndex(evicted), self.state.data(slot)).await?;
                }
                slot
            }
            None => self.state.grow(),
        };
        self.state.forget(slot);
        if load {
            self.inner.read_blocks(BlockIndex(index), self.state.data_mut(slot)).await?;
        }
        self.state.assign(slot, index);
        Ok(slot)
    }
}

#[cfg(feature = "alloc")]
impl<D: BlockDevice> BlockDevice for Cache<D> {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn access(&self) -> Access {
        self.inner.access()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<()> {
        check_blocks(self.inner.block_size(), self.inner.block_count(), first, buf.len())?;
        let size = self.inner.block_size().get() as usize;
        for (i, chunk) in buf.chunks_exact_mut(size).enumerate() {
            let slot = self.slot_for(first.0 + i as u64, true).await?;
            chunk.copy_from_slice(self.state.data(slot));
        }
        Ok(())
    }

    async fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<()> {
        if !self.inner.access().is_writable() {
            return Err(read_only());
        }
        check_blocks(self.inner.block_size(), self.inner.block_count(), first, buf.len())?;
        let size = self.inner.block_size().get() as usize;
        for (i, chunk) in buf.chunks_exact(size).enumerate() {
            let slot = self.slot_for(first.0 + i as u64, false).await?;
            self.state.data_mut(slot).copy_from_slice(chunk);
            self.state.mark_dirty(slot);
        }
        Ok(())
    }

    async fn flush(&mut self) -> Result<()> {
        while let Some(slot) = self.state.next_dirty() {
            let index = self.state.dirty_index(slot).expect("next_dirty returns a dirty slot");
            self.inner.write_blocks(BlockIndex(index), self.state.data(slot)).await?;
            self.state.clean(slot);
        }
        self.inner.flush().await
    }
}

/// Byte-granular access to a block device.
///
/// Reads and writes may start and end anywhere. Partial blocks are handled by
/// read-modify-write through a one-block scratch buffer. Also usable as a
/// [`Read`] + [`Write`] + [`Seek`] stream.
#[derive(Debug)]
pub struct ByteView<D> {
    inner: D,
    position: u64,
    scratch: crate::scratch::Scratch,
}

impl<D: BlockDevice> ByteView<D> {
    /// Wrap a device.
    ///
    /// Without `alloc`, block sizes above 4096 bytes fail with
    /// [`ErrorKind::Unsupported`] on the first partial-block access.
    pub fn new(inner: D) -> Self {
        Self { inner, position: 0, scratch: crate::scratch::Scratch::new() }
    }

    /// Device length in bytes.
    pub fn len(&self) -> u64 {
        self.inner.block_count().saturating_mul(u64::from(self.inner.block_size().get()))
    }

    /// Whether the device has no blocks.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Recover the device.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrow the device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    /// Mutably borrow the device.
    pub fn get_mut(&mut self) -> &mut D {
        &mut self.inner
    }

    /// Fill `buf` from byte `offset`.
    pub async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if !self.fits(offset, buf.len()) {
            return Err(Error::from_kind(ErrorKind::UnexpectedEof));
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
                self.inner.read_blocks(BlockIndex(block), &mut buf[done..done + whole]).await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                let scratch = self.scratch.get(size)?;
                self.inner.read_blocks(BlockIndex(block), scratch).await?;
                buf[done..done + n].copy_from_slice(&scratch[within..within + n]);
                done += n;
            }
        }
        Ok(())
    }

    /// Write all of `buf` at byte `offset`.
    pub async fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<()> {
        if !self.inner.access().is_writable() {
            return Err(read_only());
        }
        if !self.fits(offset, buf.len()) {
            return Err(Error::new(ErrorKind::InvalidInput, "write past the end of the device"));
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
                self.inner.write_blocks(BlockIndex(block), &buf[done..done + whole]).await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                let scratch = self.scratch.get(size)?;
                self.inner.read_blocks(BlockIndex(block), scratch).await?;
                scratch[within..within + n].copy_from_slice(&buf[done..done + n]);
                self.inner.write_blocks(BlockIndex(block), scratch).await?;
                done += n;
            }
        }
        Ok(())
    }

    fn fits(&self, offset: u64, len: usize) -> bool {
        offset.checked_add(len as u64).is_some_and(|end| end <= self.len())
    }

    fn remaining(&self) -> usize {
        usize::try_from(self.len().saturating_sub(self.position)).unwrap_or(usize::MAX)
    }
}

impl<D: BlockDevice> Read for ByteView<D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = buf.len().min(self.remaining());
        self.read_at(self.position, &mut buf[..n]).await?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<D: BlockDevice> Write for ByteView<D> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let n = buf.len().min(self.remaining());
        self.write_at(self.position, &buf[..n]).await?;
        self.position += n as u64;
        Ok(n)
    }

    async fn flush(&mut self) -> Result<()> {
        self.inner.flush().await
    }
}

impl<D: BlockDevice> Seek for ByteView<D> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let position = match pos {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.len().checked_add_signed(delta),
        }
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "seek overflows"))?;
        self.position = position;
        Ok(position)
    }
}

}
