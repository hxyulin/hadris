use super::{MaybeSend, Read, Seek, Write};
use crate::device::{byte_offset, check_blocks};
use crate::{
    BlockIndex, BlockSize, MemBuffer, MemDevice, OutOfRange, ReadOnly, StorageError, WriteError,
};
use hadris_io::{ErrorType, ExactError, SeekFrom};

mod sealed {
    pub trait Sealed {}
}

io_transform! {

/// A device addressed in whole logical blocks.
///
/// Buffers passed to [`read_blocks`](Self::read_blocks) and
/// [`write_blocks`](Self::write_blocks) must be a whole number of blocks, and
/// the request must lie within the device. A read-only device implements only
/// `block_size`, `block_count` and `read_blocks`: the default `write_blocks`
/// answers [`WriteError::ReadOnly`].
pub trait BlockDevice: ErrorType {
    /// Size of one block.
    fn block_size(&self) -> BlockSize;

    /// Number of addressable blocks.
    fn block_count(&self) -> u64;

    /// Reads whole blocks starting at `first`.
    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error>;

    /// Writes whole blocks starting at `first`.
    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<Self::Error>> {
        let _ = (first, buf);
        Err(WriteError::ReadOnly)
    }

    /// Flushes buffered writes to the underlying storage.
    ///
    /// A write-back device writes here, so it can report
    /// [`WriteError::ReadOnly`] too.
    async fn flush(&mut self) -> Result<(), WriteError<Self::Error>> {
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

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error> {
        D::read_blocks(self, first, buf).await
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<Self::Error>> {
        D::write_blocks(self, first, buf).await
    }

    async fn flush(&mut self) -> Result<(), WriteError<Self::Error>> {
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

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error> {
        D::read_blocks(self, first, buf).await
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<Self::Error>> {
        D::write_blocks(self, first, buf).await
    }

    async fn flush(&mut self) -> Result<(), WriteError<Self::Error>> {
        D::flush(self).await
    }
}

impl<B: MemBuffer + MaybeSend> BlockDevice for MemDevice<B> {
    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        MemDevice::block_count(self)
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), OutOfRange> {
        let range = self.range(first, buf.len())?;
        buf.copy_from_slice(&self.buffer.bytes()[range]);
        Ok(())
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<OutOfRange>> {
        let range = self.range(first, buf.len())?;
        let bytes = self.buffer.bytes_mut().ok_or(WriteError::ReadOnly)?;
        bytes[range].copy_from_slice(buf);
        Ok(())
    }
}

impl<T: Read> Read for ReadOnly<T> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.0.read(buf).await
    }
}

impl<T: Seek> Seek for ReadOnly<T> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        self.0.seek(pos).await
    }
}

/// The write half of a [`StreamDevice`] stream.
///
/// Implemented for every [`Write`] and for [`ReadOnly`], which answers
/// [`WriteError::ReadOnly`]. A second `BlockDevice` impl for read-only
/// streams would overlap the first, so the marker type carries the choice.
/// The trait is sealed.
pub trait StreamWrite: ErrorType + sealed::Sealed {
    /// Writes all of `buf`.
    async fn stream_write_all(&mut self, buf: &[u8]) -> Result<(), WriteError<ExactError<Self::Error>>>;

    /// Flushes the stream.
    async fn stream_flush(&mut self) -> Result<(), WriteError<Self::Error>>;
}

impl<T: Write + ?Sized> sealed::Sealed for T {}
impl<T: ErrorType + MaybeSend> sealed::Sealed for ReadOnly<T> {}

impl<T: Write + ?Sized> StreamWrite for T {
    async fn stream_write_all(&mut self, buf: &[u8]) -> Result<(), WriteError<ExactError<Self::Error>>> {
        Ok(self.write_all(buf).await?)
    }

    async fn stream_flush(&mut self) -> Result<(), WriteError<Self::Error>> {
        Ok(self.flush().await?)
    }
}

impl<T: ErrorType + MaybeSend> StreamWrite for ReadOnly<T> {
    async fn stream_write_all(&mut self, _buf: &[u8]) -> Result<(), WriteError<ExactError<Self::Error>>> {
        Err(WriteError::ReadOnly)
    }

    async fn stream_flush(&mut self) -> Result<(), WriteError<Self::Error>> {
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
    /// Wraps `inner`, measuring its length by seeking to the end.
    ///
    /// Trailing bytes that do not fill a block are not addressable.
    pub async fn new(mut inner: T, block_size: BlockSize) -> Result<Self, T::Error> {
        let len = inner.seek(SeekFrom::End(0)).await?;
        let block_count = len / u64::from(block_size.get());
        Ok(Self { inner, block_size, block_count })
    }
}

impl<T> StreamDevice<T> {
    /// Wraps `inner` with a known block count.
    pub fn with_block_count(inner: T, block_size: BlockSize, block_count: u64) -> Self {
        Self { inner, block_size, block_count }
    }

    /// Recovers the stream.
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Borrows the stream.
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Mutably borrows the stream.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: ErrorType> ErrorType for StreamDevice<T> {
    type Error = StorageError<T::Error>;
}

impl<T: Read + Seek + StreamWrite> BlockDevice for StreamDevice<T> {
    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error> {
        check_blocks(self.block_size, self.block_count, first, buf.len())?;
        let offset = byte_offset(self.block_size, first)?;
        self.inner.seek(SeekFrom::Start(offset)).await.map_err(StorageError::Device)?;
        Ok(self.inner.read_exact(buf).await?)
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<Self::Error>> {
        check_blocks(self.block_size, self.block_count, first, buf.len()).map_err(StorageError::from)?;
        let offset = byte_offset(self.block_size, first).map_err(StorageError::from)?;
        self.inner.seek(SeekFrom::Start(offset)).await.map_err(StorageError::Device)?;
        self.inner
            .stream_write_all(buf)
            .await
            .map_err(|err| err.map_device(StorageError::from))
    }

    async fn flush(&mut self) -> Result<(), WriteError<Self::Error>> {
        self.inner.stream_flush().await.map_err(|err| err.map_device(StorageError::Device))
    }
}

/// A contiguous block range of another device.
///
/// `D` can be owned or `&mut`. Block 0 of the slice is block `first` of the
/// underlying device. Requests past the end of the slice fail with
/// [`StorageError::OutOfRange`] and never reach the device.
#[derive(Debug)]
pub struct Slice<D> {
    inner: D,
    first: u64,
    count: u64,
}

impl<D: BlockDevice> Slice<D> {
    /// Restricts `inner` to `count` blocks starting at `first`.
    ///
    /// Fails, returning `inner`, if the range does not fit.
    pub fn new(inner: D, first: BlockIndex, count: u64) -> Result<Self, D> {
        match first.get().checked_add(count) {
            Some(end) if end <= inner.block_count() => Ok(Self { inner, first: first.get(), count }),
            _ => Err(inner),
        }
    }

    /// First block of the slice on the underlying device.
    pub fn first(&self) -> BlockIndex {
        BlockIndex::new(self.first)
    }

    /// Recovers the underlying device.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrows the underlying device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    /// Mutably borrows the underlying device.
    pub fn get_mut(&mut self) -> &mut D {
        &mut self.inner
    }
}

impl<D: ErrorType> ErrorType for Slice<D> {
    type Error = StorageError<D::Error>;
}

impl<D: BlockDevice> BlockDevice for Slice<D> {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.count
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error> {
        check_blocks(self.inner.block_size(), self.count, first, buf.len())?;
        self.inner
            .read_blocks(BlockIndex::new(self.first + first.get()), buf)
            .await
            .map_err(StorageError::Device)
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<Self::Error>> {
        check_blocks(self.inner.block_size(), self.count, first, buf.len()).map_err(StorageError::from)?;
        self.inner
            .write_blocks(BlockIndex::new(self.first + first.get()), buf)
            .await
            .map_err(|err| err.map_device(StorageError::Device))
    }

    async fn flush(&mut self) -> Result<(), WriteError<Self::Error>> {
        self.inner.flush().await.map_err(|err| err.map_device(StorageError::Device))
    }
}

/// A write-back LRU cache of whole blocks.
///
/// Writes stay in memory until [`flush`](BlockDevice::flush), eviction or
/// [`finish`](Self::finish). Dropping the cache discards unflushed writes.
///
/// The first write goes straight to the device, so a device that refuses
/// writes says so on that call rather than at a later flush. Requests outside
/// the device bypass the cache and fail with the device's own error.
#[cfg(feature = "alloc")]
#[derive(Debug)]
pub struct Cache<D> {
    inner: D,
    state: crate::cache::CacheState,
    written: bool,
}

#[cfg(feature = "alloc")]
impl<D: BlockDevice> Cache<D> {
    /// Caches up to `capacity` blocks of `inner`. A capacity of zero is treated as one.
    pub fn new(inner: D, capacity: usize) -> Self {
        let block_size = inner.block_size().get() as usize;
        Self { inner, state: crate::cache::CacheState::new(capacity, block_size), written: false }
    }

    /// Whether any cached block has unflushed writes.
    pub fn is_dirty(&self) -> bool {
        self.state.is_dirty()
    }

    /// Flushes every dirty block and returns the underlying device.
    pub async fn finish(mut self) -> Result<D, WriteError<D::Error>> {
        self.flush().await?;
        Ok(self.inner)
    }

    /// Returns the underlying device, discarding unflushed writes.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrows the underlying device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    async fn slot_for(&mut self, index: u64, load: bool) -> Result<usize, WriteError<D::Error>> {
        if let Some(slot) = self.state.lookup(index) {
            return Ok(slot);
        }
        let slot = match self.state.victim() {
            Some(slot) => {
                if let Some(evicted) = self.state.dirty_index(slot) {
                    self.inner.write_blocks(BlockIndex::new(evicted), self.state.data(slot)).await?;
                }
                slot
            }
            None => self.state.grow(),
        };
        self.state.forget(slot);
        if load {
            self.inner.read_blocks(BlockIndex::new(index), self.state.data_mut(slot)).await?;
        }
        self.state.assign(slot, index);
        Ok(slot)
    }

    fn in_range(&self, first: BlockIndex, len: usize) -> bool {
        check_blocks(self.inner.block_size(), self.inner.block_count(), first, len).is_ok()
    }
}

#[cfg(feature = "alloc")]
impl<D: ErrorType> ErrorType for Cache<D> {
    type Error = D::Error;
}

#[cfg(feature = "alloc")]
impl<D: BlockDevice> BlockDevice for Cache<D> {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error> {
        if !self.in_range(first, buf.len()) {
            return self.inner.read_blocks(first, buf).await;
        }
        let size = self.inner.block_size().get() as usize;
        for (i, chunk) in buf.chunks_exact_mut(size).enumerate() {
            let slot = match self.slot_for(first.get() + i as u64, true).await {
                Ok(slot) => slot,
                Err(WriteError::Device(err)) => return Err(err),
                Err(_) => {
                    self.inner.read_blocks(BlockIndex::new(first.get() + i as u64), chunk).await?;
                    continue;
                }
            };
            chunk.copy_from_slice(self.state.data(slot));
        }
        Ok(())
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<Self::Error>> {
        if !self.written || !self.in_range(first, buf.len()) {
            self.inner.write_blocks(first, buf).await?;
            self.written = true;
            self.state.invalidate(first.get(), buf.len() / self.inner.block_size().get() as usize);
            return Ok(());
        }
        let size = self.inner.block_size().get() as usize;
        for (i, chunk) in buf.chunks_exact(size).enumerate() {
            let slot = self.slot_for(first.get() + i as u64, false).await?;
            self.state.data_mut(slot).copy_from_slice(chunk);
            self.state.mark_dirty(slot);
        }
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), WriteError<Self::Error>> {
        while let Some(slot) = self.state.next_dirty() {
            let Some(index) = self.state.dirty_index(slot) else { break };
            self.inner.write_blocks(BlockIndex::new(index), self.state.data(slot)).await?;
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
    /// Wraps a device.
    ///
    /// Without `alloc`, block sizes above 4096 bytes fail with
    /// [`StorageError::BlockTooLarge`] on the first partial-block access.
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

    /// Recovers the device.
    pub fn into_inner(self) -> D {
        self.inner
    }

    /// Borrows the device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }

    /// Mutably borrows the device.
    pub fn get_mut(&mut self) -> &mut D {
        &mut self.inner
    }

    /// Fills `buf` from byte `offset`.
    pub async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), StorageError<D::Error>> {
        if !self.fits(offset, buf.len()) {
            return Err(StorageError::UnexpectedEof);
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
                    .read_blocks(BlockIndex::new(block), &mut buf[done..done + whole])
                    .await
                    .map_err(StorageError::Device)?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                let scratch = self.scratch.get(size).ok_or(StorageError::BlockTooLarge)?;
                self.inner.read_blocks(BlockIndex::new(block), scratch).await.map_err(StorageError::Device)?;
                buf[done..done + n].copy_from_slice(&scratch[within..within + n]);
                done += n;
            }
        }
        Ok(())
    }

    /// Writes all of `buf` at byte `offset`.
    pub async fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<(), StorageError<D::Error>> {
        if !self.fits(offset, buf.len()) {
            return Err(StorageError::OutOfRange);
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
                self.inner.write_blocks(BlockIndex::new(block), &buf[done..done + whole]).await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                let scratch = self.scratch.get(size).ok_or(StorageError::BlockTooLarge)?;
                self.inner.read_blocks(BlockIndex::new(block), scratch).await.map_err(StorageError::Device)?;
                scratch[within..within + n].copy_from_slice(&buf[done..done + n]);
                self.inner.write_blocks(BlockIndex::new(block), scratch).await?;
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

impl<D: ErrorType> ErrorType for ByteView<D> {
    type Error = StorageError<D::Error>;
}

impl<D: BlockDevice> Read for ByteView<D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min(self.remaining());
        self.read_at(self.position, &mut buf[..n]).await?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<D: BlockDevice> Write for ByteView<D> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min(self.remaining());
        self.write_at(self.position, &buf[..n]).await?;
        self.position += n as u64;
        Ok(n)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(self.inner.flush().await?)
    }
}

impl<D: BlockDevice> Seek for ByteView<D> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let position = pos
            .resolve(self.position, self.len())
            .ok_or(StorageError::OutOfRange)?;
        self.position = position;
        Ok(position)
    }
}

}
