use super::async_::BlockDevice;
use crate::{BlockIndex, BlockSize};
use core::task::{Context, Poll};
use hadris_io::{Error, ErrorType};

/// Two-window bounded read-ahead for sequential access.
#[derive(Debug)]
pub struct ReadAhead<D> {
    inner: D,
    capacity: usize,
    block_size: usize,
    windows: [Window; 2],
    recent: usize,
}
#[derive(Debug, Default)]
struct Window {
    first: u64,
    count: usize,
    data: alloc::vec::Vec<u8>,
}
/// Progress of a read-ahead request.
#[derive(Default)]
#[non_exhaustive]
pub struct ReadAheadOperation<S> {
    child: S,
    pending: bool,
    phase: u8,
    slot: usize,
    fetched: usize,
}
impl<D: BlockDevice> ReadAhead<D> {
    /// Retains up to `capacity` blocks. Zero disables read-ahead.
    pub fn new(inner: D, capacity: usize) -> Self {
        let size = inner.block_size().get() as usize;
        let capacity = capacity
            .min(isize::MAX as usize / size)
            .min(usize::try_from(inner.block_count()).unwrap_or(usize::MAX));
        Self {
            inner,
            capacity,
            block_size: size,
            windows: Default::default(),
            recent: 1,
        }
    }
    /// Maximum retained block count.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// Invalidates retained data.
    pub fn clear(&mut self) {
        for window in &mut self.windows {
            window.count = 0;
        }
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
        let size = self.block_size().get() as usize;
        let count = buf.len() / size;
        if size != self.block_size {
            self.windows = Default::default();
            self.block_size = size;
            self.capacity = self.capacity.min(isize::MAX as usize / size);
        }
        if state.phase == 0 {
            if self.capacity == 0 || buf.is_empty() {
                state.phase = 2;
            } else {
                for (slot, window) in self.windows.iter().enumerate() {
                    if first.get() >= window.first {
                        let offset = first.get() - window.first;
                        if offset <= window.count as u64
                            && count as u64 <= window.count as u64 - offset
                        {
                            let at = offset as usize * size;
                            buf.copy_from_slice(&window.data[at..at + buf.len()]);
                            self.recent = slot;
                            return Poll::Ready(Ok(()));
                        }
                    }
                }
                let adjacent = self
                    .windows
                    .iter()
                    .position(|w| w.count != 0 && w.first + w.count as u64 == first.get());
                state.slot = adjacent.unwrap_or_else(|| {
                    if self.capacity == 1 {
                        0
                    } else {
                        1 - self.recent
                    }
                });
                let capacity = if state.slot == 0 {
                    self.capacity - self.capacity / 2
                } else {
                    self.capacity / 2
                };
                if count > capacity {
                    state.phase = 2;
                } else {
                    let available = usize::try_from(self.block_count().saturating_sub(first.get()))
                        .unwrap_or(usize::MAX);
                    state.fetched = if adjacent.is_some() {
                        capacity.min(available)
                    } else {
                        count
                    };
                    let window = &mut self.windows[state.slot];
                    window.count = 0;
                    window.data.resize(capacity * size, 0);
                    state.phase = 1;
                }
            }
        }
        state.pending = true;
        let result = if state.phase == 1 {
            self.inner.poll_read_blocks(
                &mut state.child,
                cx,
                first,
                &mut self.windows[state.slot].data[..state.fetched * size],
            )
        } else {
            self.inner
                .poll_read_blocks(&mut state.child, cx, first, buf)
        };
        state.pending = result.is_pending();
        match result {
            Poll::Ready(Ok(())) if state.phase == 1 => {
                let window = &mut self.windows[state.slot];
                window.first = first.get();
                window.count = state.fetched;
                buf.copy_from_slice(&window.data[..buf.len()]);
                self.recent = state.slot;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(_)) if state.phase == 1 && state.fetched != count => {
                state.child = Default::default();
                state.phase = 2;
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
        self.clear();
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
        self.clear();
    }
}
