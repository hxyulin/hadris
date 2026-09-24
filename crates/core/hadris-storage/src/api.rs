use super::{MaybeSend, Read, Seek, Write};
use crate::device::{byte_offset, check_blocks};
use crate::{BlockIndex, BlockSize, MemBuffer, MemDevice, Partition, ReadOnly};
use core::convert::Infallible;
use hadris_io::{Error, ErrorKind, ErrorType, SeekFrom};

mod sealed {
    pub trait Sealed {}
}

io_transform! {

/// A device addressed in whole logical blocks.
///
/// Buffers passed to [`read_blocks`](Self::read_blocks) and
/// [`write_blocks`](Self::write_blocks) must be a whole number of blocks, and
/// the request must lie within the device. Every method returns
/// [`Error<Self::Error>`](Error): a device failure is
/// [`Error::device`], and a request the device refuses itself, such as one
/// past its end, is an [`ErrorKind`] with a location. A read-only device
/// implements only `block_size`, `block_count` and `read_blocks`: it is
/// not [`writable`](Self::writable), and the default `write_blocks`
/// answers [`ErrorKind::ReadOnly`]. A device that accepts writes overrides
/// `writable`, `write_blocks` and, if it buffers, `flush`.
pub trait BlockDevice: ErrorType {
    /// Size of one block.
    fn block_size(&self) -> BlockSize;

    /// Number of addressable blocks.
    fn block_count(&self) -> u64;

    /// Number of blocks the device can hold when written past its end.
    ///
    /// A device that grows on write, such as a `Vec<u8>` or a host image
    /// file, reports more than [`block_count`](Self::block_count), which it
    /// defaults to. A writer checks the size it plans against this before
    /// writing anything.
    fn max_block_count(&self) -> u64 {
        self.block_count()
    }

    /// Byte offset of block 0 within the disk this device is a window of.
    ///
    /// 0 unless the device is a window of another, such as a
    /// [`Partition`], which adds its offset.
    fn disk_offset(&self) -> u64 {
        0
    }

    /// Whether the device accepts writes at all.
    ///
    /// False unless overridden. A driver mounts a device that says false
    /// read-only. A device that says true may still refuse a later write
    /// with [`ErrorKind::ReadOnly`], for example when media are
    /// write-protected while mounted.
    fn writable(&self) -> bool {
        false
    }

    /// Reads whole blocks starting at `first`.
    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>>;

    /// Writes whole blocks starting at `first`.
    ///
    /// A device that refuses writes returns [`ErrorKind::ReadOnly`].
    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        let _ = (first, buf);
        Err(Error::new(ErrorKind::ReadOnly, "device is read-only"))
    }

    /// Flushes buffered writes to the underlying storage.
    ///
    /// A write-back device writes here, so it can report
    /// [`ErrorKind::ReadOnly`] too.
    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
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

    fn max_block_count(&self) -> u64 {
        D::max_block_count(self)
    }

    fn disk_offset(&self) -> u64 {
        D::disk_offset(self)
    }

    fn writable(&self) -> bool {
        D::writable(self)
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        D::read_blocks(self, first, buf).await
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        D::write_blocks(self, first, buf).await
    }

    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
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

    fn max_block_count(&self) -> u64 {
        D::max_block_count(self)
    }

    fn disk_offset(&self) -> u64 {
        D::disk_offset(self)
    }

    fn writable(&self) -> bool {
        D::writable(self)
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        D::read_blocks(self, first, buf).await
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        D::write_blocks(self, first, buf).await
    }

    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
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

    fn writable(&self) -> bool {
        self.buffer.writable()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Infallible>> {
        let range = self.range(first, buf.len())?;
        buf.copy_from_slice(&self.buffer.bytes()[range]);
        Ok(())
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Infallible>> {
        let range = self.range(first, buf.len())?;
        let bytes = self
            .buffer
            .bytes_mut()
            .ok_or(Error::new(ErrorKind::ReadOnly, "memory buffer is read-only"))?;
        bytes[range].copy_from_slice(buf);
        Ok(())
    }
}

/// An in-memory image with 512-byte blocks that grows when written past
/// its end, filling any gap with zeros. Trailing bytes that do not fill a
/// block are not readable until a write covers them. A write fails with
/// [`ErrorKind::NoSpace`] when the vector cannot grow.
#[cfg(feature = "alloc")]
impl BlockDevice for alloc::vec::Vec<u8> {
    fn block_size(&self) -> BlockSize {
        crate::device::BLOCK_512
    }

    fn block_count(&self) -> u64 {
        self.len() as u64 / u64::from(crate::device::BLOCK_512.get())
    }

    fn max_block_count(&self) -> u64 {
        isize::MAX as u64 / u64::from(crate::device::BLOCK_512.get())
    }

    fn writable(&self) -> bool {
        true
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Infallible>> {
        check_blocks(crate::device::BLOCK_512, self.block_count(), first, buf.len())?;
        let start = byte_offset(crate::device::BLOCK_512, first)? as usize;
        buf.copy_from_slice(&self[start..start + buf.len()]);
        Ok(())
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Infallible>> {
        check_blocks(crate::device::BLOCK_512, self.max_block_count(), first, buf.len())?;
        let start = byte_offset(crate::device::BLOCK_512, first)? as usize;
        let end = start + buf.len();
        if end > self.len() {
            self.try_reserve(end - self.len())
                .map_err(|_| Error::new(ErrorKind::NoSpace, "cannot grow the image"))?;
            self.resize(end, 0);
        }
        self[start..end].copy_from_slice(buf);
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
/// [`ErrorKind::ReadOnly`]. A second `BlockDevice` impl for read-only
/// streams would overlap the first, so the marker type carries the choice.
/// The trait is sealed.
pub trait StreamWrite: ErrorType + sealed::Sealed {
    /// Whether the stream accepts writes: false for [`ReadOnly`].
    fn stream_writable(&self) -> bool;

    /// Writes all of `buf`.
    async fn stream_write_all(&mut self, buf: &[u8]) -> Result<(), Error<Self::Error>>;

    /// Flushes the stream.
    async fn stream_flush(&mut self) -> Result<(), Error<Self::Error>>;
}

impl<T: Write + ?Sized> sealed::Sealed for T {}
impl<T: ErrorType + MaybeSend> sealed::Sealed for ReadOnly<T> {}

impl<T: Write + ?Sized> StreamWrite for T {
    fn stream_writable(&self) -> bool {
        true
    }

    async fn stream_write_all(&mut self, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        Ok(self.write_all(buf).await?)
    }

    async fn stream_flush(&mut self) -> Result<(), Error<Self::Error>> {
        self.flush().await.map_err(|err| Error::device(err, "flushing the stream failed"))
    }
}

impl<T: ErrorType + MaybeSend> StreamWrite for ReadOnly<T> {
    fn stream_writable(&self) -> bool {
        false
    }

    async fn stream_write_all(&mut self, _buf: &[u8]) -> Result<(), Error<Self::Error>> {
        Err(Error::new(ErrorKind::ReadOnly, "stream is read-only"))
    }

    async fn stream_flush(&mut self) -> Result<(), Error<Self::Error>> {
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
    type Error = T::Error;
}

impl<T: Read + Seek + StreamWrite> BlockDevice for StreamDevice<T> {
    fn block_size(&self) -> BlockSize {
        self.block_size
    }

    fn block_count(&self) -> u64 {
        self.block_count
    }

    fn writable(&self) -> bool {
        self.inner.stream_writable()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        check_blocks(self.block_size, self.block_count, first, buf.len())?;
        let offset = byte_offset(self.block_size, first)?;
        self.inner
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|err| Error::device(err, "seeking the stream failed"))?;
        self.inner
            .read_exact(buf)
            .await
            .map_err(|err| Error::from(err).with_location(hadris_io::Location::Block(first.get())))
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        check_blocks(self.block_size, self.block_count, first, buf.len())?;
        let offset = byte_offset(self.block_size, first)?;
        self.inner
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|err| Error::device(err, "seeking the stream failed"))?;
        self.inner.stream_write_all(buf).await
    }

    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        self.inner.stream_flush().await
    }
}

impl<D: BlockDevice> BlockDevice for Partition<D> {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.len() / u64::from(self.inner.block_size().get())
    }

    fn disk_offset(&self) -> u64 {
        self.inner.disk_offset().saturating_add(self.offset())
    }

    fn writable(&self) -> bool {
        self.inner.writable()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        let at = self.locate(self.inner.block_size(), first, buf.len())?;
        self.inner.read_blocks(at, buf).await
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        let at = self.locate(self.inner.block_size(), first, buf.len())?;
        self.inner.write_blocks(at, buf).await
    }

    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        self.inner.flush().await
    }
}

fn past_end<E>(offset: u64) -> Error<E> {
    Error::new(ErrorKind::InvalidInput, "byte range past the end of the device")
        .with_location(hadris_io::Location::Byte(offset))
}

fn block_too_large<E>() -> Error<E> {
    Error::new(ErrorKind::Unsupported, "block size exceeds the adapter buffer")
}

/// A write-back LRU cache of whole blocks.
///
/// Writes stay in memory until [`flush`](BlockDevice::flush), eviction or
/// [`finish`](Self::finish). Dropping the cache discards unflushed writes.
/// Flush writes each run of consecutive dirty blocks in one device call.
///
/// The first write goes straight to the device, so a device that refuses
/// writes says so on that call rather than at a later flush. Requests outside
/// the device bypass the cache and fail with the device's own error. Requests
/// of at least `capacity` blocks also go straight to the device, since caching
/// them would evict everything else; reads still see cached dirty blocks.
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
    pub async fn finish(mut self) -> Result<D, Error<D::Error>> {
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

    async fn slot_for(&mut self, index: u64, load: bool) -> Result<usize, Error<D::Error>> {
        if let Some(slot) = self.state.lookup(index) {
            return Ok(slot);
        }
        let slot = match self.state.victim() {
            Some(slot) => {
                if let Some(evicted) = self.state.dirty_index(slot) {
                    self.write_back(evicted).await?;
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

    async fn write_back(&mut self, index: u64) -> Result<(), Error<D::Error>> {
        let slot = self.state.peek(index).unwrap_or_default();
        self.inner.write_blocks(BlockIndex::new(index), self.state.data(slot)).await?;
        self.state.clean_range(index, 1);
        Ok(())
    }

    fn in_range(&self, first: BlockIndex, len: usize) -> bool {
        check_blocks::<Infallible>(self.inner.block_size(), self.inner.block_count(), first, len).is_ok()
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

    fn max_block_count(&self) -> u64 {
        self.inner.max_block_count()
    }

    fn disk_offset(&self) -> u64 {
        self.inner.disk_offset()
    }

    fn writable(&self) -> bool {
        self.inner.writable()
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        if !self.in_range(first, buf.len()) {
            return self.inner.read_blocks(first, buf).await;
        }
        let size = self.inner.block_size().get() as usize;
        let count = buf.len() / size;
        if count >= self.state.capacity() {
            self.inner.read_blocks(first, buf).await?;
            for index in self.state.dirty_in(first.get(), count) {
                let at = (index - first.get()) as usize * size;
                let slot = self.state.peek(index).unwrap_or_default();
                buf[at..at + size].copy_from_slice(self.state.data(slot));
            }
            return Ok(());
        }
        let mut i = 0;
        while i < count {
            let index = first.get() + i as u64;
            if let Some(slot) = self.state.lookup(index) {
                buf[i * size..(i + 1) * size].copy_from_slice(self.state.data(slot));
                i += 1;
                continue;
            }
            let mut end = i + 1;
            while end < count && self.state.peek(first.get() + end as u64).is_none() {
                end += 1;
            }
            self.inner.read_blocks(BlockIndex::new(index), &mut buf[i * size..end * size]).await?;
            for k in i..end {
                let slot = match self.slot_for(first.get() + k as u64, false).await {
                    Ok(slot) => slot,
                    Err(err) if err.kind() == ErrorKind::ReadOnly => continue,
                    Err(err) => return Err(err),
                };
                self.state.data_mut(slot).copy_from_slice(&buf[k * size..(k + 1) * size]);
            }
            i = end;
        }
        Ok(())
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
        let size = self.inner.block_size().get() as usize;
        let count = buf.len() / size;
        if !self.written || !self.in_range(first, buf.len()) || count >= self.state.capacity() {
            self.inner.write_blocks(first, buf).await?;
            self.written = true;
            self.state.invalidate(first.get(), count);
            return Ok(());
        }
        for (i, chunk) in buf.chunks_exact(size).enumerate() {
            let slot = self.slot_for(first.get() + i as u64, false).await?;
            self.state.data_mut(slot).copy_from_slice(chunk);
            self.state.mark_dirty(slot);
        }
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        while let Some(start) = self.state.first_dirty() {
            let len = self.state.dirty_run(start);
            let run = self.state.gather(start, len);
            self.inner.write_blocks(BlockIndex::new(start), run).await?;
            self.state.clean_range(start, len);
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
    ///
    /// A range past the end of the device fails with
    /// [`ErrorKind::InvalidInput`].
    pub async fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error<D::Error>> {
        if !self.fits(offset, buf.len()) {
            return Err(past_end(offset));
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
                self.inner.read_blocks(BlockIndex::new(block), &mut buf[done..done + whole]).await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                let scratch = self.scratch.get(size).ok_or_else(block_too_large)?;
                self.inner.read_blocks(BlockIndex::new(block), scratch).await?;
                buf[done..done + n].copy_from_slice(&scratch[within..within + n]);
                done += n;
            }
        }
        Ok(())
    }

    /// Writes all of `buf` at byte `offset`.
    pub async fn write_at(&mut self, offset: u64, buf: &[u8]) -> Result<(), Error<D::Error>> {
        if !self.fits(offset, buf.len()) {
            return Err(past_end(offset));
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
                let scratch = self.scratch.get(size).ok_or_else(block_too_large)?;
                self.inner.read_blocks(BlockIndex::new(block), scratch).await?;
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
    type Error = Error<D::Error>;
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
        self.inner.flush().await
    }
}

impl<D: BlockDevice> Seek for ByteView<D> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let position = pos
            .resolve(self.position, self.len())
            .ok_or(Error::new(ErrorKind::InvalidInput, "seek to a negative or overflowing position"))?;
        self.position = position;
        Ok(position)
    }
}

}
