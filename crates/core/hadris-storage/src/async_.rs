//! Unified asynchronous block storage.
//! Operation futures derive Send from their device and operation state.
use crate::device::check_blocks;
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
    /// Physical block size, a multiple of `block_size`, which it defaults
    /// to. Formatters align their layout to it.
    fn physical_block_size(&self) -> BlockSize {
        self.block_size()
    }
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
    /// Rejects unaligned buffers and ranges beyond `block_count`.
    /// No access to `buf` may continue after this call returns.
    fn poll_read_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>>;
    /// Advances a write; no access to `buf` may continue after return.
    /// Rejects unaligned buffers and ranges beyond `max_block_count`.
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
impl<D: BlockDevice + ?Sized> Future for Operation<'_, D> {
    type Output = Result<(), Error<D::Error>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.done, "operation polled after completion");
        if !this.checked {
            let result = match &this.request {
                Request::Read(first, buf) => check_blocks(
                    this.device.block_size(),
                    this.device.block_count(),
                    *first,
                    buf.len(),
                )
                .map(drop),
                Request::Write(first, buf) => check_blocks(
                    this.device.block_size(),
                    this.device.max_block_count(),
                    *first,
                    buf.len(),
                )
                .map(drop),
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
        fn physical_block_size(&self) -> BlockSize {
            D::physical_block_size(self)
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
        Poll::Ready(self.read_into(first, buf))
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(self.write_from(first, buf))
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
        self.len() as u64 / u64::from(crate::device::BLOCK_512.get())
    }
    fn max_block_count(&self) -> u64 {
        crate::device::VEC_MAX_BLOCKS
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
        Poll::Ready(crate::device::vec_read(self, first, buf))
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(crate::device::vec_write(self, first, buf))
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
    fn physical_block_size(&self) -> BlockSize {
        self.get_ref().physical_block_size()
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
        let limit = self.get_ref().block_count();
        let at = match self.locate(self.block_size(), limit, first, buf.len()) {
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
        let limit = self.get_ref().max_block_count();
        let at = match self.locate(self.block_size(), limit, first, buf.len()) {
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
/// A read-only view of a device: it is not writable, reads go to the device,
/// and writes fail with [`ErrorKind::ReadOnly`] without reaching it. Flush
/// writes nothing.
impl<D: BlockDevice> BlockDevice for crate::ReadOnly<D> {
    type State = D::State;
    fn block_size(&self) -> BlockSize {
        self.0.block_size()
    }
    fn physical_block_size(&self) -> BlockSize {
        self.0.physical_block_size()
    }
    fn block_count(&self) -> u64 {
        self.0.block_count()
    }
    fn disk_offset(&self) -> u64 {
        self.0.disk_offset()
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        self.0.poll_read_blocks(state, cx, first, buf)
    }
    fn cancel(&mut self, state: &mut Self::State) {
        self.0.cancel(state);
    }
}
use hadris_io::SeekFrom;

byte_view!(keep_async);

impl<D: SendBlockDevice> hadris_io::async_::Read for ByteView<D> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let n = buf.len().min(self.remaining());
        self.read_at(self.position, &mut buf[..n]).await?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<D: SendBlockDevice> hadris_io::async_::Write for ByteView<D> {
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

impl<D: SendBlockDevice> hadris_io::async_::Seek for ByteView<D> {
    async fn seek(&mut self, pos: SeekFrom) -> Result<u64, Self::Error> {
        self.seek_to(pos)
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
        self.seek_to(pos)
    }
}

#[path = "poll_stream.rs"]
mod stream;
#[cfg(feature = "sync")]
pub use stream::BlockingStream;
pub use stream::{Stream, StreamDevice, StreamState};
