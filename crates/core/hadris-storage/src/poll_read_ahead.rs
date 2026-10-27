use super::async_::BlockDevice;
use crate::read_ahead::Plan;
use crate::{BlockIndex, BlockSize};
use core::task::{Context, Poll};
use hadris_io::{Error, ErrorType};

/// Two-window bounded read-ahead for sequential access.
///
/// Writes invalidate the windows they overlap before reaching the device,
/// including failed or cancelled writes; a window elsewhere stays valid.
#[derive(Debug)]
pub struct ReadAhead<D> {
    inner: D,
    windows: crate::read_ahead::Windows,
}
#[derive(Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Fetching,
    Bypass,
}

/// Progress of a read-ahead request.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ReadAheadOperation<S> {
    child: S,
    pending: bool,
    phase: Phase,
    slot: usize,
    fetched: usize,
}
impl<D: BlockDevice> ReadAhead<D> {
    /// Retains up to `capacity` blocks. Zero disables read-ahead.
    pub fn new(inner: D, capacity: usize) -> Self {
        let size = inner.block_size().get() as usize;
        let windows = crate::read_ahead::Windows::new(size, capacity, inner.block_count());
        Self { inner, windows }
    }
    /// Maximum retained block count.
    pub fn capacity(&self) -> usize {
        self.windows.capacity()
    }
    /// Invalidates retained data.
    pub fn clear(&mut self) {
        self.windows.clear();
    }
    /// Borrows the device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }
    /// Mutably borrows the device and invalidates retained data.
    pub fn get_mut(&mut self) -> &mut D {
        self.clear();
        &mut self.inner
    }
    /// Recovers the device.
    pub fn into_inner(self) -> D {
        self.inner
    }
}
impl<D: ErrorType> ErrorType for ReadAhead<D> {
    type Error = D::Error;
}
impl<D: BlockDevice> BlockDevice for ReadAhead<D> {
    type State = ReadAheadOperation<D::State>;
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }
    fn physical_block_size(&self) -> BlockSize {
        self.inner.physical_block_size()
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
    fn poll_read_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if let Err(error) =
            crate::device::check_blocks(self.block_size(), self.block_count(), first, buf.len())
        {
            return Poll::Ready(Err(error));
        }
        let size = self.block_size().get() as usize;
        if state.phase == Phase::Idle {
            let block_count = self.block_count();
            match self
                .windows
                .plan(self.block_size(), block_count, first, buf)
            {
                Plan::Hit => return Poll::Ready(Ok(())),
                Plan::Bypass => state.phase = Phase::Bypass,
                Plan::Fetch { slot, fetched } => {
                    state.slot = slot;
                    state.fetched = fetched;
                    state.phase = Phase::Fetching;
                }
            }
        }
        state.pending = true;
        let result = if state.phase == Phase::Fetching {
            let target = self.windows.target(state.slot, state.fetched);
            self.inner
                .poll_read_blocks(&mut state.child, cx, first, target)
        } else {
            self.inner
                .poll_read_blocks(&mut state.child, cx, first, buf)
        };
        state.pending = result.is_pending();
        match result {
            Poll::Ready(Ok(())) if state.phase == Phase::Fetching => {
                self.windows
                    .fill(state.slot, state.fetched, first.get(), buf);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(_))
                if state.phase == Phase::Fetching && state.fetched * size != buf.len() =>
            {
                state.child = Default::default();
                state.phase = Phase::Bypass;
                self.poll_read_blocks(state, cx, first, buf)
            }
            other => other,
        }
    }
    fn poll_write_blocks(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if let Err(error) =
            crate::device::check_blocks(self.block_size(), self.max_block_count(), first, buf.len())
        {
            return Poll::Ready(Err(error));
        }
        let size = self.block_size().get() as usize;
        self.windows.invalidate(size, first.get(), buf.len());
        state.pending = true;
        let result = self
            .inner
            .poll_write_blocks(&mut state.child, cx, first, buf);
        state.pending = result.is_pending();
        result
    }
    fn poll_flush(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        state.pending = true;
        let result = self.inner.poll_flush(&mut state.child, cx);
        state.pending = result.is_pending();
        result
    }
    fn cancel(&mut self, state: &mut Self::State) {
        if state.pending {
            self.inner.cancel(&mut state.child);
            state.pending = false;
        }
    }
}
