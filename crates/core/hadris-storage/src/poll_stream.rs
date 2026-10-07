use crate::async_::BlockDevice;
use crate::{BlockIndex, BlockSize};
use core::task::{Context, Poll};
use hadris_io::{Error, ErrorKind, ErrorType, SeekFrom};

/// Seekable byte stream with operation-owned state.
///
/// Hooks must release borrowed buffers before returning, including Pending.
/// State must not retain pointers to its own movable fields. Hardware that
/// continues accessing memory between polls needs owned stable buffers.
pub trait Stream: ErrorType {
    /// State created separately for each operation.
    type State: Default + Unpin;
    /// Whether writes are accepted.
    fn writable(&self) -> bool {
        false
    }
    /// Whether writing past the current end can grow the stream.
    fn grows(&self) -> bool {
        false
    }
    /// Advances a read. A completed hook returns its byte count.
    fn poll_read(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error<Self::Error>>>;
    /// Advances positioning to `pos`.
    fn poll_seek(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<Result<u64, Error<Self::Error>>>;
    /// Advances a write. A completed hook returns its byte count.
    fn poll_write(
        &mut self,
        _: &mut Self::State,
        _: &mut Context<'_>,
        _: &[u8],
    ) -> Poll<Result<usize, Error<Self::Error>>> {
        Poll::Ready(Err(ErrorKind::ReadOnly.into()))
    }
    /// Makes earlier writes durable.
    fn poll_flush(
        &mut self,
        _: &mut Self::State,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(Ok(()))
    }
    /// Stops an interrupted operation, including a hook that panicked before
    /// starting I/O. This must be safe when idle and must not panic.
    fn cancel(&mut self, state: &mut Self::State);
}

/// Explicit bridge that runs synchronous stream operations during polling.
/// This adapter blocks its executor while the underlying call runs.
#[cfg(feature = "sync")]
#[derive(Debug)]
pub struct BlockingStream<T>(T, bool);
#[cfg(feature = "sync")]
impl<T> BlockingStream<T> {
    /// Wraps a synchronous seekable stream.
    pub const fn new(inner: T) -> Self {
        Self(inner, false)
    }
    /// Wraps a stream known to support extension beyond its current end.
    pub const fn new_growable(inner: T) -> Self {
        Self(inner, true)
    }
    /// Returns the stream.
    pub fn into_inner(self) -> T {
        self.0
    }
    /// Borrows the stream.
    pub const fn get_ref(&self) -> &T {
        &self.0
    }
    /// Mutably borrows the stream.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
#[cfg(feature = "sync")]
impl<T: ErrorType> ErrorType for BlockingStream<T> {
    type Error = T::Error;
}
#[cfg(feature = "sync")]
impl<T: hadris_io::sync::Read + hadris_io::sync::Seek + crate::sync::StreamWrite> Stream
    for BlockingStream<T>
{
    type State = ();
    fn writable(&self) -> bool {
        self.0.stream_writable()
    }
    fn grows(&self) -> bool {
        self.1 && self.writable()
    }
    fn poll_read(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error<T::Error>>> {
        Poll::Ready(
            self.0
                .read(buf)
                .map_err(|error| Error::device(error, "stream read failed")),
        )
    }
    fn poll_seek(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<Result<u64, Error<T::Error>>> {
        Poll::Ready(
            self.0
                .seek(pos)
                .map_err(|error| Error::device(error, "stream seek failed")),
        )
    }
    fn poll_write(
        &mut self,
        _: &mut (),
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error<T::Error>>> {
        Poll::Ready(self.0.stream_write_all(buf).map(|()| buf.len()))
    }
    fn poll_flush(&mut self, _: &mut (), _: &mut Context<'_>) -> Poll<Result<(), Error<T::Error>>> {
        Poll::Ready(self.0.stream_flush())
    }
    fn cancel(&mut self, _: &mut ()) {}
}

/// Block device over a poll-native seekable byte stream.
#[derive(Debug)]
pub struct StreamDevice<T> {
    inner: T,
    size: BlockSize,
    blocks: u64,
}
impl<T> StreamDevice<T> {
    /// Uses an explicit readable block count without inspecting the stream.
    pub const fn with_block_count(inner: T, size: BlockSize, blocks: u64) -> Self {
        Self {
            inner,
            size,
            blocks,
        }
    }
    /// Returns the stream.
    pub fn into_inner(self) -> T {
        self.inner
    }
    /// Borrows the stream.
    pub const fn get_ref(&self) -> &T {
        &self.inner
    }
    /// Mutably borrows the stream. Reconstruct the device if its length changes.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}
impl<T: Stream> StreamDevice<T> {
    /// Determines the readable size by seeking to the stream's end.
    pub async fn new(mut inner: T, size: BlockSize) -> Result<Self, Error<T::Error>> {
        let len = SeekOperation {
            stream: &mut inner,
            state: T::State::default(),
            pending: false,
        }
        .await?;
        Ok(Self::with_block_count(
            inner,
            size,
            len / u64::from(size.get()),
        ))
    }
}
impl<T: ErrorType> ErrorType for StreamDevice<T> {
    type Error = T::Error;
}

struct SeekOperation<'a, T: Stream> {
    stream: &'a mut T,
    state: T::State,
    pending: bool,
}
impl<T: Stream> core::future::Future for SeekOperation<'_, T> {
    type Output = Result<u64, Error<T::Error>>;
    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.pending = true;
        let result = this.stream.poll_seek(&mut this.state, cx, SeekFrom::End(0));
        if result.is_ready() {
            this.pending = false;
        }
        result
    }
}
impl<T: Stream> Drop for SeekOperation<'_, T> {
    fn drop(&mut self) {
        if self.pending {
            self.stream.cancel(&mut self.state);
        }
    }
}

/// State of a stream-backed block operation.
#[derive(Debug)]
#[non_exhaustive]
pub struct StreamState<S> {
    inner: S,
    positioned: bool,
    done: usize,
}
impl<S: Default> Default for StreamState<S> {
    fn default() -> Self {
        Self {
            inner: S::default(),
            positioned: false,
            done: 0,
        }
    }
}
impl<T: Stream> StreamDevice<T> {
    fn position(
        &mut self,
        state: &mut StreamState<T::State>,
        cx: &mut Context<'_>,
        first: BlockIndex,
    ) -> Poll<Result<(), Error<T::Error>>> {
        if !state.positioned {
            let Some(offset) = first.get().checked_mul(u64::from(self.size.get())) else {
                return Poll::Ready(Err(ErrorKind::InvalidInput.into()));
            };
            match self
                .inner
                .poll_seek(&mut state.inner, cx, SeekFrom::Start(offset))
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(actual)) if actual != offset => {
                    return Poll::Ready(Err(Error::new(
                        ErrorKind::Io,
                        "stream seek returned a different position",
                    )));
                }
                Poll::Ready(Ok(_)) => {
                    state.inner = T::State::default();
                    state.positioned = true;
                }
            }
        }
        Poll::Ready(Ok(()))
    }
}
impl<T: Stream> BlockDevice for StreamDevice<T> {
    type State = StreamState<T::State>;
    fn block_size(&self) -> BlockSize {
        self.size
    }
    fn block_count(&self) -> u64 {
        self.blocks
    }
    fn max_block_count(&self) -> u64 {
        if self.inner.grows() {
            u64::MAX / u64::from(self.size.get())
        } else {
            self.blocks
        }
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
    ) -> Poll<Result<(), Error<T::Error>>> {
        if let Err(error) =
            crate::device::check_blocks(self.block_size(), self.block_count(), first, buf.len())
        {
            return Poll::Ready(Err(error));
        }
        match self.position(state, cx, first) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        while state.done < buf.len() {
            match self
                .inner
                .poll_read(&mut state.inner, cx, &mut buf[state.done..])
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(n)) if n == 0 || n > buf.len() - state.done => {
                    return Poll::Ready(Err(Error::new(
                        ErrorKind::Io,
                        "stream read made invalid progress",
                    )));
                }
                Poll::Ready(Ok(n)) => {
                    state.done += n;
                    state.inner = T::State::default();
                }
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
    ) -> Poll<Result<(), Error<T::Error>>> {
        if let Err(error) =
            crate::device::check_blocks(self.block_size(), self.max_block_count(), first, buf.len())
        {
            return Poll::Ready(Err(error));
        }
        match self.position(state, cx, first) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        while state.done < buf.len() {
            match self
                .inner
                .poll_write(&mut state.inner, cx, &buf[state.done..])
            {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(n)) if n == 0 || n > buf.len() - state.done => {
                    return Poll::Ready(Err(Error::new(
                        ErrorKind::Io,
                        "stream write made invalid progress",
                    )));
                }
                Poll::Ready(Ok(n)) => {
                    state.done += n;
                    state.inner = T::State::default();
                    let end = first.get() + state.done as u64 / u64::from(self.size.get());
                    self.blocks = self.blocks.max(end);
                }
            }
        }
        Poll::Ready(Ok(()))
    }
    fn poll_flush(
        &mut self,
        state: &mut Self::State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<T::Error>>> {
        self.inner.poll_flush(&mut state.inner, cx)
    }
    fn cancel(&mut self, state: &mut Self::State) {
        self.inner.cancel(&mut state.inner);
    }
}
