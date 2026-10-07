#![no_std]

mod cache;
pub use cache::Cache;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use hadris_io::{Error, ErrorKind, ErrorType};
use hadris_storage::{BlockIndex, BlockSize, MemBuffer, MemDevice, Partition};

/// Experimental poll contract with operation-owned, movable state.
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
    /// Stops a pending operation when its future is dropped.
    /// This hook must not panic. Buffer safety must also hold when it is not
    /// called, such as when an operation future is forgotten.
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
        hadris_storage::sync::BlockDevice::block_size(self)
    }
    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(self)
    }
    fn writable(&self) -> bool {
        hadris_storage::sync::BlockDevice::writable(self)
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(hadris_storage::sync::BlockDevice::read_blocks(
            self, first, buf,
        ))
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(hadris_storage::sync::BlockDevice::write_blocks(
            self, first, buf,
        ))
    }
    fn cancel(&mut self, _: &mut ()) {}
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

/// A reader model using one multi-await algorithm for both device capabilities.
pub struct FileSystem<D>(pub D);
impl<D: BlockDevice> FileSystem<D> {
    /// Reads two blocks using the same device future contract.
    pub async fn read_pair(
        &mut self,
        first: BlockIndex,
        a: &mut [u8],
        b: &mut [u8],
    ) -> Result<(), Error<D::Error>> {
        self.0.read_blocks(first, a).await?;
        self.read_next(first, b).await
    }
    async fn read_next(&mut self, first: BlockIndex, b: &mut [u8]) -> Result<(), Error<D::Error>> {
        let next = first
            .get()
            .checked_add(1)
            .ok_or(Error::new(ErrorKind::InvalidInput, "block overflow"))?;
        self.0.read_blocks(BlockIndex::new(next), b).await
    }
}
