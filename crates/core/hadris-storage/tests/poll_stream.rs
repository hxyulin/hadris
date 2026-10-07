#![cfg(feature = "async")]
use core::future::Future;
use core::task::{Context, Poll, Waker};
use hadris_io::{Error, ErrorKind, ErrorType, SeekFrom};
use hadris_storage::async_::{BlockDevice, SendBlockDevice, Stream, StreamDevice};
use hadris_storage::{BlockIndex, BlockSize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Parker {
    thread: std::thread::Thread,
    wakes: AtomicUsize,
}
impl std::task::Wake for Parker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        self.thread.unpark();
    }
}
fn run<F: Future>(future: F) -> (F::Output, usize) {
    let parker = Arc::new(Parker {
        thread: std::thread::current(),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(parker.clone());
    let mut cx = Context::from_waker(&waker);
    let mut future = core::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => return (result, parker.wakes.load(Ordering::SeqCst)),
            Poll::Pending => std::thread::park(),
        }
    }
}
#[derive(Default)]
struct State {
    ready: Option<Arc<AtomicBool>>,
}
struct Native {
    data: Vec<u8>,
    pos: usize,
    suspend: Option<&'static str>,
    error: Option<&'static str>,
    zero: bool,
    instant: bool,
    cancels: Arc<AtomicUsize>,
}
impl Native {
    fn new() -> Self {
        Self {
            data: vec![0; 16],
            pos: 0,
            suspend: None,
            error: None,
            zero: false,
            instant: false,
            cancels: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn advance(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
        kind: &'static str,
    ) -> Poll<Result<(), Error<core::convert::Infallible>>> {
        if self.suspend == Some(kind) {
            return Poll::Pending;
        }
        if self.instant {
            return if self.error == Some(kind) {
                Poll::Ready(Err(Error::new(ErrorKind::Io, "injected failure")))
            } else {
                Poll::Ready(Ok(()))
            };
        }
        match &state.ready {
            None => {
                let ready = Arc::new(AtomicBool::new(false));
                state.ready = Some(ready.clone());
                let waker = cx.waker().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    ready.store(true, Ordering::Release);
                    waker.wake();
                });
                Poll::Pending
            }
            Some(ready) if !ready.load(Ordering::Acquire) => Poll::Pending,
            Some(_) => {
                if self.error == Some(kind) {
                    Poll::Ready(Err(Error::new(ErrorKind::Io, "injected failure")))
                } else {
                    Poll::Ready(Ok(()))
                }
            }
        }
    }
}
impl ErrorType for Native {
    type Error = core::convert::Infallible;
}
impl Stream for Native {
    type State = State;
    fn writable(&self) -> bool {
        true
    }
    fn grows(&self) -> bool {
        true
    }
    fn poll_seek(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
        pos: SeekFrom,
    ) -> Poll<Result<u64, Error<Self::Error>>> {
        match self.advance(state, cx, "seek") {
            Poll::Ready(Ok(())) => {
                self.pos = pos
                    .resolve(self.pos as u64, self.data.len() as u64)
                    .unwrap() as usize;
                Poll::Ready(Ok(self.pos as u64))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_read(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error<Self::Error>>> {
        match self.advance(state, cx, "read") {
            Poll::Ready(Ok(())) => {
                let n = if self.zero {
                    0
                } else {
                    buf.len()
                        .min(2)
                        .min(self.data.len().saturating_sub(self.pos))
                };
                buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_write(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error<Self::Error>>> {
        match self.advance(state, cx, "write") {
            Poll::Ready(Ok(())) => {
                let n = if self.zero { 0 } else { buf.len().min(2) };
                self.data.resize(self.data.len().max(self.pos + n), 0);
                self.data[self.pos..self.pos + n].copy_from_slice(&buf[..n]);
                self.pos += n;
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_flush(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        self.advance(state, cx, "flush")
    }
    fn cancel(&mut self, state: &mut State) {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        state.ready = None;
    }
}
fn require_send<D: SendBlockDevice>(device: &mut D, buf: &mut [u8]) {
    fn send(_: impl Future + Send) {}
    send(device.read_blocks(BlockIndex::new(0), buf));
}
#[test]
fn runtime_wakeups_partial_io_and_growth_round_trip() {
    let (result, wakes) = run(StreamDevice::new(Native::new(), BlockSize::new(4).unwrap()));
    let mut device = result.unwrap();
    assert!(wakes > 0);
    require_send(&mut device, &mut [0; 4]);
    let (result, wakes) = run(device.write_blocks(BlockIndex::new(4), &[7; 8]));
    result.unwrap();
    assert!(wakes >= 5);
    assert_eq!(device.block_count(), 6);
    let mut data = [0; 8];
    let (result, wakes) = run(device.read_blocks(BlockIndex::new(4), &mut data));
    result.unwrap();
    assert!(wakes >= 5);
    assert_eq!(data, [7; 8]);
    let (result, wakes) = run(device.flush());
    result.unwrap();
    assert!(wakes > 0);
}
#[test]
fn all_pending_stream_phases_cancel_and_allow_reuse() {
    for phase in ["seek", "read", "write", "flush"] {
        let mut native = Native::new();
        native.suspend = Some(phase);
        native.instant = true;
        let cancels = native.cancels.clone();
        let mut device = StreamDevice::with_block_count(native, BlockSize::new(4).unwrap(), 4);
        let mut buf = [0; 4];
        {
            let operation = if phase == "write" {
                device.write_blocks(BlockIndex::new(0), &[9; 4])
            } else if phase == "flush" {
                device.flush()
            } else {
                device.read_blocks(BlockIndex::new(0), &mut buf)
            };
            let mut operation = core::pin::pin!(operation);
            let mut cx = Context::from_waker(Waker::noop());
            assert!(operation.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(cancels.load(Ordering::SeqCst), 1);
        device.get_mut().suspend = None;
        run(device.write_blocks(BlockIndex::new(0), &[3; 4]))
            .0
            .unwrap();
        run(device.read_blocks(BlockIndex::new(0), &mut buf))
            .0
            .unwrap();
        assert_eq!(buf, [3; 4]);
    }
}
#[test]
fn stream_errors_zero_progress_and_read_only_are_reported() {
    for phase in ["seek", "read", "write", "flush"] {
        let mut native = Native::new();
        native.instant = true;
        native.error = Some(phase);
        let mut device = StreamDevice::with_block_count(native, BlockSize::new(4).unwrap(), 4);
        let result = if phase == "write" {
            run(device.write_blocks(BlockIndex::new(0), &[9; 4])).0
        } else if phase == "flush" {
            run(device.flush()).0
        } else {
            run(device.read_blocks(BlockIndex::new(0), &mut [0; 4])).0
        };
        assert_eq!(result.unwrap_err().kind(), ErrorKind::Io);
        device.get_mut().error = None;
        run(device.flush()).0.unwrap();
    }
    let mut native = Native::new();
    native.instant = true;
    native.zero = true;
    let mut device = StreamDevice::with_block_count(native, BlockSize::new(4).unwrap(), 4);
    assert_eq!(
        run(device.read_blocks(BlockIndex::new(0), &mut [0; 4]))
            .0
            .unwrap_err()
            .kind(),
        ErrorKind::Io
    );
    assert_eq!(
        run(device.write_blocks(BlockIndex::new(0), &[0; 4]))
            .0
            .unwrap_err()
            .kind(),
        ErrorKind::Io
    );
}
#[cfg(all(feature = "sync", feature = "std"))]
#[test]
fn explicit_blocking_bridge_grows_only_when_requested() {
    use hadris_storage::async_::BlockingStream;
    let cursor = hadris_io::StdIo::new(std::io::Cursor::new(vec![0; 8]));
    let mut fixed = run(StreamDevice::new(
        BlockingStream::new(cursor),
        BlockSize::new(4).unwrap(),
    ))
    .0
    .unwrap();
    assert_eq!(fixed.max_block_count(), 2);
    assert_eq!(
        run(fixed.write_blocks(BlockIndex::new(2), &[1; 4]))
            .0
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    let cursor = fixed.into_inner().into_inner();
    let mut growable = run(StreamDevice::new(
        BlockingStream::new_growable(cursor),
        BlockSize::new(4).unwrap(),
    ))
    .0
    .unwrap();
    run(growable.write_blocks(BlockIndex::new(2), &[1; 4]))
        .0
        .unwrap();
    assert_eq!(growable.block_count(), 3);
    let cursor = hadris_io::StdIo::new(std::io::Cursor::new(vec![0; 8]));
    let read_only = hadris_storage::ReadOnly::new(cursor);
    let mut device = run(StreamDevice::new(
        BlockingStream::new(read_only),
        BlockSize::new(4).unwrap(),
    ))
    .0
    .unwrap();
    assert_eq!(
        run(device.write_blocks(BlockIndex::new(0), &[1; 4]))
            .0
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );
}
