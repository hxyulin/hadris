use super::async_::BlockDevice;
use crate::{BlockIndex, BlockSize};
use core::task::{Context, Poll};
use hadris_io::{Error, ErrorKind, ErrorType};

/// The phase of a write that goes straight to the device once the dirty
/// blocks it covers are written back.
const PASS_THROUGH: u8 = 4;

/// A write-back LRU cache. Dropping it discards unflushed writes. Before a
/// write goes straight to the device, the dirty blocks it covers are written
/// back and every cached copy of them is dropped.
#[derive(Debug)]
pub struct Cache<D> {
    inner: D,
    cache: crate::cache::CacheState,
    written: bool,
}
/// Independent progress for a cached operation.
#[derive(Default)]
#[non_exhaustive]
pub struct CacheOperation<S> {
    child: S,
    pending: bool,
    done: usize,
    loaded_end: usize,
    phase: u8,
    slot: usize,
    start: u64,
    len: usize,
}
impl<D: BlockDevice> Cache<D> {
    /// Caches at most `capacity` blocks; zero is treated as one.
    pub fn new(inner: D, capacity: usize) -> Self {
        let size = inner.block_size().get() as usize;
        Self {
            inner,
            cache: crate::cache::CacheState::new(capacity, size),
            written: false,
        }
    }
    /// Whether unflushed writes remain.
    pub fn is_dirty(&self) -> bool {
        self.cache.is_dirty()
    }
    /// Returns the underlying device, discarding unflushed writes.
    pub fn into_inner(self) -> D {
        self.inner
    }
    /// Borrows the underlying device.
    pub fn get_ref(&self) -> &D {
        &self.inner
    }
    /// Flushes the cache and returns its device. On failure the cache comes
    /// back with the error, still holding the blocks it could not write.
    #[allow(clippy::result_large_err)]
    pub async fn finish(mut self) -> Result<D, (Self, Error<D::Error>)> {
        match self.flush().await {
            Ok(()) => Ok(self.inner),
            Err(error) => Err((self, error)),
        }
    }
    fn slot(
        &mut self,
        state: &mut CacheOperation<D::State>,
        cx: &mut Context<'_>,
        index: u64,
    ) -> Poll<Result<usize, Error<D::Error>>> {
        if state.phase != 1 {
            if let Some(slot) = self.cache.lookup(index) {
                return Poll::Ready(Ok(slot));
            }
            state.slot = self.cache.victim().unwrap_or_else(|| self.cache.grow());
            state.phase = 1;
        }
        if let Some(evicted) = self.cache.dirty_index(state.slot) {
            state.pending = true;
            let result = self.inner.poll_write_blocks(
                &mut state.child,
                cx,
                BlockIndex::new(evicted),
                self.cache.data(state.slot),
            );
            state.pending = result.is_pending();
            match result {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {
                    self.cache.clean_range(evicted, 1);
                    state.child = Default::default();
                }
            }
        }
        self.cache.forget(state.slot);
        self.cache.assign(state.slot, index);
        state.phase = 0;
        Poll::Ready(Ok(state.slot))
    }
}
impl<D: ErrorType> ErrorType for Cache<D> {
    type Error = D::Error;
}
impl<D: BlockDevice> BlockDevice for Cache<D> {
    type State = CacheOperation<D::State>;
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
        if let Err(error) =
            crate::device::check_blocks(self.block_size(), self.block_count(), first, buf.len())
        {
            return Poll::Ready(Err(error));
        }
        let size = self.block_size().get() as usize;
        let count = buf.len() / size;
        if count >= self.cache.capacity() {
            state.pending = true;
            let result = self
                .inner
                .poll_read_blocks(&mut state.child, cx, first, buf);
            state.pending = result.is_pending();
            if let Poll::Ready(Ok(())) = result {
                for index in self.cache.dirty_in(first.get(), count) {
                    let at = (index - first.get()) as usize * size;
                    let slot = self.cache.peek(index).unwrap();
                    buf[at..at + size].copy_from_slice(self.cache.data(slot));
                }
            }
            return result;
        }
        while state.done < count {
            let index = first.get() + state.done as u64;
            if state.phase == 0 {
                if let Some(slot) = self.cache.lookup(index) {
                    buf[state.done * size..(state.done + 1) * size]
                        .copy_from_slice(self.cache.data(slot));
                    state.done += 1;
                    continue;
                }
                state.loaded_end = state.done + 1;
                while state.loaded_end < count
                    && self
                        .cache
                        .peek(first.get() + state.loaded_end as u64)
                        .is_none()
                {
                    state.loaded_end += 1;
                }
                state.phase = 2;
            }
            if state.phase == 2 {
                state.pending = true;
                let result = self.inner.poll_read_blocks(
                    &mut state.child,
                    cx,
                    BlockIndex::new(index),
                    &mut buf[state.done * size..state.loaded_end * size],
                );
                state.pending = result.is_pending();
                match result {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Ready(Ok(())) => {
                        state.child = Default::default();
                        state.phase = 3;
                    }
                }
            }
            let slot = match self.slot(state, cx, index) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) if e.kind() == ErrorKind::ReadOnly => {
                    state.phase = 0;
                    state.child = Default::default();
                    state.done += 1;
                    continue;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(slot)) => slot,
            };
            self.cache
                .data_mut(slot)
                .copy_from_slice(&buf[state.done * size..(state.done + 1) * size]);
            state.done += 1;
            if state.done < state.loaded_end {
                state.phase = 3;
            }
        }
        Poll::Ready(Ok(()))
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
        let count = buf.len() / size;
        let in_range = crate::device::check_blocks::<core::convert::Infallible>(
            self.block_size(),
            self.block_count(),
            first,
            buf.len(),
        )
        .is_ok();
        if !self.written || !in_range || count >= self.cache.capacity() {
            if state.phase != PASS_THROUGH {
                loop {
                    let Some(index) = self.cache.dirty_in(first.get(), count).next() else {
                        break;
                    };
                    let slot = self.cache.peek(index).unwrap_or_default();
                    state.pending = true;
                    let result = self.inner.poll_write_blocks(
                        &mut state.child,
                        cx,
                        BlockIndex::new(index),
                        self.cache.data(slot),
                    );
                    state.pending = result.is_pending();
                    match result {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Ready(Ok(())) => {
                            self.cache.clean_range(index, 1);
                            state.child = Default::default();
                        }
                    }
                }
                self.cache.invalidate(first.get(), count);
                state.phase = PASS_THROUGH;
            }
            state.pending = true;
            let result = self
                .inner
                .poll_write_blocks(&mut state.child, cx, first, buf);
            state.pending = result.is_pending();
            if let Poll::Ready(Ok(())) = result {
                self.written = true;
            }
            return result;
        }
        while state.done < count {
            let index = first.get() + state.done as u64;
            let slot = match self.slot(state, cx, index) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(slot)) => slot,
            };
            self.cache
                .data_mut(slot)
                .copy_from_slice(&buf[state.done * size..(state.done + 1) * size]);
            self.cache.mark_dirty(slot);
            state.done += 1;
        }
        Poll::Ready(Ok(()))
    }
    fn poll_flush(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        loop {
            if state.phase == 0 {
                if let Some(start) = self.cache.first_dirty() {
                    state.start = start;
                    state.len = self.cache.dirty_run(start);
                    self.cache.gather(start, state.len);
                    state.phase = 1;
                } else {
                    state.phase = 2;
                }
            }
            state.pending = true;
            let result = if state.phase == 1 {
                self.inner.poll_write_blocks(
                    &mut state.child,
                    cx,
                    BlockIndex::new(state.start),
                    self.cache.run(),
                )
            } else {
                self.inner.poll_flush(&mut state.child, cx)
            };
            state.pending = result.is_pending();
            match result {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {
                    if state.phase == 2 {
                        return Poll::Ready(Ok(()));
                    }
                    self.cache.clean_range(state.start, state.len);
                    state.child = Default::default();
                    state.phase = 0;
                }
            }
        }
    }
    fn cancel(&mut self, state: &mut Self::State) {
        if state.pending {
            self.inner.cancel(&mut state.child);
            state.pending = false;
        }
    }
}
