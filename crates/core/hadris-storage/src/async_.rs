//! Unified asynchronous block storage.
//! Operation futures derive Send from their device and operation state.
use crate::{BlockIndex, BlockSize, MemBuffer, MemDevice, Partition};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use hadris_io::{Error, ErrorKind, ErrorType};

/// Poll contract with operation-owned, movable state.
/// State must not retain pointers to its own fields across polls. Safe poll
/// hooks must stop accessing caller buffers before returning, including Pending.
/// Hardware backends need owned stable buffers for I/O that continues between
/// polls; cancellation alone cannot protect buffers if a future is forgotten.
pub trait BlockDevice: ErrorType {
    /// State created independently for each read, write or flush operation.
    type State: Default + Unpin;
    /// Logical block size.
    fn block_size(&self) -> BlockSize;
    /// Addressable blocks.
    fn block_count(&self) -> u64;
    /// Maximum writable blocks.
    fn max_block_count(&self) -> u64 {
        self.block_count()
    }
    /// Partition byte offset.
    fn disk_offset(&self) -> u64 {
        0
    }
    /// Whether writes are accepted.
    fn writable(&self) -> bool {
        false
    }
    /// Advances a read using the same arguments and state on every poll.
    /// No access to `buf` may continue after this call returns.
    fn poll_read_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>>;
    /// Advances a write; no access to `buf` may continue after return.
    fn poll_write_blocks(
        &mut self,
        _: &mut Self::State,
        _: &mut Context<'_>,
        _: BlockIndex,
        _: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(Err(Error::new(ErrorKind::ReadOnly, "device is read-only")))
    }
    /// Advances durability of earlier writes.
    fn poll_flush(
        &mut self,
        _: &mut Self::State,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(Ok(()))
    }
    /// Stops an operation when its future is dropped, including when a poll
    /// hook panics before returning Pending or starting I/O.
    /// This hook must be a safe no-op when no operation started and must not
    /// panic. Buffer safety must also hold when it is not called, such as when
    /// an operation future is forgotten.
    fn cancel(&mut self, state: &mut Self::State);
    /// Reads whole blocks without boxing the future.
    fn read_blocks<'a>(&'a mut self, first: BlockIndex, buf: &'a mut [u8]) -> Operation<'a, Self> {
        Operation::new(self, Request::Read(first, buf))
    }
    /// Writes whole blocks without boxing the future.
    fn write_blocks<'a>(&'a mut self, first: BlockIndex, buf: &'a [u8]) -> Operation<'a, Self> {
        Operation::new(self, Request::Write(first, buf))
    }
    /// Flushes without boxing the future.
    fn flush(&mut self) -> Operation<'_, Self> {
        Operation::new(self, Request::Flush)
    }
}

/// Stronger guarantee derived from the device and its operation state.
pub trait SendBlockDevice: BlockDevice<State: Send> + Send {}
impl<D: BlockDevice + Send + ?Sized> SendBlockDevice for D where D::State: Send {}

enum Request<'a> {
    Read(BlockIndex, &'a mut [u8]),
    Write(BlockIndex, &'a [u8]),
    Flush,
}

/// Concrete future whose auto traits follow the device and operation state.
#[non_exhaustive]
pub struct Operation<'a, D: BlockDevice + ?Sized> {
    device: &'a mut D,
    state: D::State,
    request: Request<'a>,
    checked: bool,
    pending: bool,
    done: bool,
}
impl<'a, D: BlockDevice + ?Sized> Operation<'a, D> {
    fn new(device: &'a mut D, request: Request<'a>) -> Self {
        Self {
            device,
            state: D::State::default(),
            request,
            checked: false,
            pending: false,
            done: false,
        }
    }
}
fn range<E>(size: BlockSize, count: u64, first: BlockIndex, len: usize) -> Result<(), Error<E>> {
    let size = size.get() as usize;
    if len % size != 0
        || first
            .get()
            .checked_add((len / size) as u64)
            .is_none_or(|end| end > count)
    {
        return Err(
            Error::new(ErrorKind::InvalidInput, "block range outside device")
                .with_location(hadris_io::Location::Block(first.get())),
        );
    }
    Ok(())
}
impl<D: BlockDevice + ?Sized> Future for Operation<'_, D> {
    type Output = Result<(), Error<D::Error>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.done, "operation polled after completion");
        if !this.checked {
            let result = match &this.request {
                Request::Read(first, buf) => range(
                    this.device.block_size(),
                    this.device.block_count(),
                    *first,
                    buf.len(),
                ),
                Request::Write(first, buf) => range(
                    this.device.block_size(),
                    this.device.max_block_count(),
                    *first,
                    buf.len(),
                ),
                Request::Flush => Ok(()),
            };
            this.checked = true;
            if let Err(error) = result {
                this.done = true;
                return Poll::Ready(Err(error));
            }
        }
        this.pending = true;
        let result = match &mut this.request {
            Request::Read(first, buf) => {
                this.device
                    .poll_read_blocks(&mut this.state, cx, *first, buf)
            }
            Request::Write(first, buf) => {
                this.device
                    .poll_write_blocks(&mut this.state, cx, *first, buf)
            }
            Request::Flush => this.device.poll_flush(&mut this.state, cx),
        };
        this.pending = result.is_pending();
        this.done = result.is_ready();
        result
    }
}
impl<D: BlockDevice + ?Sized> Drop for Operation<'_, D> {
    fn drop(&mut self) {
        if self.pending {
            self.device.cancel(&mut self.state);
        }
    }
}

macro_rules! borrow_device {
    () => {
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
        fn poll_read_blocks(
            &mut self,
            state: &mut Self::State,
            cx: &mut Context<'_>,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Poll<Result<(), Error<Self::Error>>> {
            D::poll_read_blocks(self, state, cx, first, buf)
        }
        fn poll_write_blocks(
            &mut self,
            state: &mut Self::State,
            cx: &mut Context<'_>,
            first: BlockIndex,
            buf: &[u8],
        ) -> Poll<Result<(), Error<Self::Error>>> {
            D::poll_write_blocks(self, state, cx, first, buf)
        }
        fn poll_flush(
            &mut self,
            state: &mut Self::State,
            cx: &mut Context<'_>,
        ) -> Poll<Result<(), Error<Self::Error>>> {
            D::poll_flush(self, state, cx)
        }
        fn cancel(&mut self, state: &mut Self::State) {
            D::cancel(self, state);
        }
    };
}
impl<D: BlockDevice + ?Sized> BlockDevice for &mut D {
    type State = D::State;
    borrow_device!();
}

impl<B: MemBuffer> BlockDevice for MemDevice<B> {
    type State = ();
    fn block_size(&self) -> BlockSize {
        self.block_size
    }
    fn block_count(&self) -> u64 {
        self.block_count()
    }
    fn writable(&self) -> bool {
        self.buffer.writable()
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(
            self.range(first, buf.len())
                .map(|range| buf.copy_from_slice(&self.buffer.bytes()[range])),
        )
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        let range = match self.range(first, buf.len()) {
            Ok(range) => range,
            Err(e) => return Poll::Ready(Err(e)),
        };
        Poll::Ready(match self.buffer.bytes_mut() {
            Some(bytes) => {
                bytes[range].copy_from_slice(buf);
                Ok(())
            }
            None => Err(Error::new(ErrorKind::ReadOnly, "device is read-only")),
        })
    }
    fn cancel(&mut self, _: &mut ()) {}
}
#[cfg(feature = "alloc")]
impl BlockDevice for alloc::vec::Vec<u8> {
    type State = ();
    fn block_size(&self) -> BlockSize {
        crate::device::BLOCK_512
    }
    fn block_count(&self) -> u64 {
        self.len() as u64 / 512
    }
    fn max_block_count(&self) -> u64 {
        isize::MAX as u64 / 512
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(
            crate::device::check_blocks(self.block_size(), self.block_count(), first, buf.len())
                .map(|_| {
                    let start = first.get() as usize * 512;
                    buf.copy_from_slice(&self[start..start + buf.len()]);
                }),
        )
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        let result = crate::device::check_blocks(
            self.block_size(),
            self.max_block_count(),
            first,
            buf.len(),
        )
        .and_then(|_| {
            let start = first.get() as usize * 512;
            let end = start + buf.len();
            if end > self.len() {
                self.try_reserve(end - self.len())
                    .map_err(|_| Error::new(ErrorKind::NoSpace, "cannot grow the image"))?;
                self.resize(end, 0);
            }
            self[start..end].copy_from_slice(buf);
            Ok(())
        });
        Poll::Ready(result)
    }
    fn cancel(&mut self, _: &mut ()) {}
}
#[cfg(feature = "alloc")]
impl<D: BlockDevice + ?Sized> BlockDevice for alloc::boxed::Box<D> {
    type State = D::State;
    borrow_device!();
}

impl<D: BlockDevice> BlockDevice for Partition<D> {
    type State = D::State;
    fn block_size(&self) -> BlockSize {
        self.get_ref().block_size()
    }
    fn block_count(&self) -> u64 {
        self.len() / u64::from(self.block_size().get())
    }
    fn disk_offset(&self) -> u64 {
        self.get_ref().disk_offset().saturating_add(self.offset())
    }
    fn writable(&self) -> bool {
        self.get_ref().writable()
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        let at = match partition_block(self, first, buf.len(), false) {
            Ok(at) => at,
            Err(e) => return Poll::Ready(Err(e)),
        };
        self.get_mut().poll_read_blocks(state, cx, at, buf)
    }
    fn poll_write_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        let at = match partition_block(self, first, buf.len(), true) {
            Ok(at) => at,
            Err(e) => return Poll::Ready(Err(e)),
        };
        self.get_mut().poll_write_blocks(state, cx, at, buf)
    }
    fn poll_flush(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        self.get_mut().poll_flush(state, cx)
    }
    fn cancel(&mut self, state: &mut Self::State) {
        self.get_mut().cancel(state);
    }
}
fn partition_block<D: BlockDevice>(
    part: &Partition<D>,
    first: BlockIndex,
    len: usize,
    writing: bool,
) -> Result<BlockIndex, Error<D::Error>> {
    let size = u64::from(part.block_size().get());
    if part.offset() % size != 0 || part.len() % size != 0 {
        return Err(Error::new(ErrorKind::InvalidInput, "unaligned partition"));
    }
    range(part.block_size(), part.block_count(), first, len)?;
    let at = part
        .offset()
        .checked_div(size)
        .and_then(|offset| offset.checked_add(first.get()))
        .map(BlockIndex::new)
        .ok_or(Error::new(
            ErrorKind::InvalidInput,
            "partition offset overflow",
        ))?;
    let limit = if writing {
        part.get_ref().max_block_count()
    } else {
        part.get_ref().block_count()
    };
    range(part.block_size(), limit, at, len)?;
    Ok(at)
}

use hadris_io::SeekFrom;
fn past_end<E>(offset: u64) -> Error<E> {
    Error::new(ErrorKind::InvalidInput, "byte range outside device")
        .with_location(hadris_io::Location::Byte(offset))
}
fn block_too_large<E>() -> Error<E> {
    Error::new(
        ErrorKind::Unsupported,
        "block size exceeds scratch capacity",
    )
}
/// Byte-granular read-modify-write access to a block device.
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
        Self {
            inner,
            position: 0,
            scratch: crate::scratch::Scratch::new(),
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
                self.inner
                    .read_blocks(BlockIndex::new(block), &mut buf[done..done + whole])
                    .await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                let scratch = self.scratch.get(size).ok_or_else(block_too_large)?;
                self.inner
                    .read_blocks(BlockIndex::new(block), scratch)
                    .await?;
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
                self.inner
                    .write_blocks(BlockIndex::new(block), &buf[done..done + whole])
                    .await?;
                done += whole;
            } else {
                let n = (size - within).min(left);
                let scratch = self.scratch.get(size).ok_or_else(block_too_large)?;
                self.inner
                    .read_blocks(BlockIndex::new(block), scratch)
                    .await?;
                scratch[within..within + n].copy_from_slice(&buf[done..done + n]);
                self.inner
                    .write_blocks(BlockIndex::new(block), scratch)
                    .await?;
                done += n;
            }
        }
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
}

impl<D: ErrorType> ErrorType for ByteView<D> {
    type Error = Error<D::Error>;
}

impl<D: SendBlockDevice> hadris_io::r#async::Read for ByteView<D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min(self.remaining());
        self.read_at(self.position, &mut buf[..n]).await?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<D: SendBlockDevice> hadris_io::r#async::Write for ByteView<D> {
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

impl<D: SendBlockDevice> hadris_io::r#async::Seek for ByteView<D> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let position = pos.resolve(self.position, self.len()).ok_or(Error::new(
            ErrorKind::InvalidInput,
            "seek to a negative or overflowing position",
        ))?;
        self.position = position;
        Ok(position)
    }
}

#[cfg(feature = "alloc")]
pub use crate::poll_cache::{Cache, CacheOperation};
#[cfg(feature = "alloc")]
pub use crate::poll_read_ahead::{ReadAhead, ReadAheadOperation};

impl<D: BlockDevice> hadris_io::local::Read for ByteView<D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min(self.remaining());
        self.read_at(self.position, &mut buf[..n]).await?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<D: BlockDevice> hadris_io::local::Write for ByteView<D> {
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

impl<D: BlockDevice> hadris_io::local::Seek for ByteView<D> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        let position = pos.resolve(self.position, self.len()).ok_or(Error::new(
            ErrorKind::InvalidInput,
            "seek to a negative or overflowing position",
        ))?;
        self.position = position;
        Ok(position)
    }
}

#[cfg(all(feature = "std", feature = "sync"))]
impl BlockDevice for crate::host::FileDevice {
    type State = ();
    fn block_size(&self) -> BlockSize {
        crate::sync::BlockDevice::block_size(self)
    }
    fn block_count(&self) -> u64 {
        crate::sync::BlockDevice::block_count(self)
    }
    fn max_block_count(&self) -> u64 {
        crate::sync::BlockDevice::max_block_count(self)
    }
    fn disk_offset(&self) -> u64 {
        crate::sync::BlockDevice::disk_offset(self)
    }
    fn writable(&self) -> bool {
        crate::sync::BlockDevice::writable(self)
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(crate::sync::BlockDevice::read_blocks(self, first, buf))
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(crate::sync::BlockDevice::write_blocks(self, first, buf))
    }
    fn poll_flush(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(crate::sync::BlockDevice::flush(self))
    }
    fn cancel(&mut self, _: &mut ()) {}
}

#[path = "poll_stream.rs"]
mod stream;
#[cfg(feature = "sync")]
pub use stream::BlockingStream;
pub use stream::{Stream, StreamDevice, StreamState};
