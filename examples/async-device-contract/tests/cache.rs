use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::cell::Cell;
use std::rc::Rc;

use hadris_experiment_async_device_contract::{BlockDevice, Cache, FileSystem, SendBlockDevice};
use hadris_io::{Error, ErrorKind, ErrorType};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};

#[derive(Default)]
struct State {
    pending: bool,
    buffer: usize,
    _local: Rc<()>,
}

struct Device {
    data: [u8; 1024],
    reads: Rc<Cell<usize>>,
    cancellations: Rc<Cell<usize>>,
    flushes: Rc<Cell<usize>>,
    fail_write: bool,
}
impl ErrorType for Device {
    type Error = Infallible;
}
impl BlockDevice for Device {
    type State = State;
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        2
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        if !state.pending {
            self.reads.set(self.reads.get() + 1);
            state.pending = true;
            state.buffer = buf.as_ptr() as usize;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        assert_eq!(state.buffer, buf.as_ptr() as usize);
        let first = first.get() as usize * 512;
        buf.copy_from_slice(&self.data[first..first + buf.len()]);
        Poll::Ready(Ok(()))
    }
    fn poll_write_blocks(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        let first = first.get() as usize * 512;
        self.data[first..first + buf.len()].copy_from_slice(buf);
        if self.fail_write {
            return Poll::Ready(Err(Error::new(ErrorKind::InvalidInput, "partial write")));
        }
        if !state.pending {
            state.pending = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(Ok(()))
    }
    fn poll_flush(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Infallible>>> {
        if !state.pending {
            self.flushes.set(self.flushes.get() + 1);
            state.pending = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(Ok(()))
    }
    fn cancel(&mut self, _: &mut State) {
        self.cancellations.set(self.cancellations.get() + 1);
    }
}
fn device(fail_write: bool) -> Device {
    Device {
        data: [7; 1024],
        reads: Rc::new(Cell::new(0)),
        cancellations: Rc::new(Cell::new(0)),
        flushes: Rc::new(Cell::new(0)),
        fail_write,
    }
}
fn complete<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(&mut cx) {
            return result;
        }
    }
}
fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn local_pending_read_populates_cache_and_hit_skips_device() {
    let inner = device(false);
    let reads = inner.reads.clone();
    let mut cache = Cache::new(inner);
    let mut buffer = [0; 512];
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    assert_eq!(buffer, [7; 512]);
    buffer.fill(0);
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    assert_eq!(buffer, [7; 512]);
    assert_eq!(reads.get(), 1);
    let mut larger = [0; 1024];
    complete(cache.read_blocks(BlockIndex::new(0), &mut larger)).unwrap();
    assert_eq!(larger, [7; 1024]);
    assert_eq!(reads.get(), 2);
}

#[test]
fn cancelled_pending_read_forwards_cancel_and_never_creates_hit() {
    let inner = device(false);
    let reads = inner.reads.clone();
    let cancellations = inner.cancellations.clone();
    let mut cache = Cache::new(inner);
    let mut buffer = [0; 512];
    let mut read = cache.read_blocks(BlockIndex::new(0), &mut buffer);
    assert!(poll_once(&mut read).is_pending());
    drop(read);
    assert_eq!(cancellations.get(), 1);
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    assert_eq!(reads.get(), 2);
}

#[test]
fn failed_write_cannot_leave_stale_cached_data() {
    let mut cache = Cache::new(device(true));
    let mut buffer = [0; 512];
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    assert!(complete(cache.write_blocks(BlockIndex::new(0), &[9; 512])).is_err());
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    assert_eq!(buffer, [9; 512]);
}

#[test]
fn cancelled_write_cannot_leave_stale_cached_data() {
    let inner = device(false);
    let cancellations = inner.cancellations.clone();
    let mut cache = Cache::new(inner);
    let mut buffer = [0; 512];
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    let bytes = [9; 512];
    let mut write = cache.write_blocks(BlockIndex::new(0), &bytes);
    assert!(poll_once(&mut write).is_pending());
    drop(write);
    assert_eq!(cancellations.get(), 1);
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    assert_eq!(buffer, [9; 512]);
}

fn require_send(_: impl Future + Send) {}
fn generic_send<D: SendBlockDevice>(fs: &mut FileSystem<D>, a: &mut [u8], b: &mut [u8]) {
    require_send(fs.read_pair(BlockIndex::new(0), a, b));
}
#[test]
fn borrowed_nested_partitions_and_cache_keep_single_bound_send_proof() {
    let mut bytes = [0; 2048];
    bytes[512..1024].fill(1);
    bytes[1024..1536].fill(2);
    let mut device = MemDevice::new(&mut bytes[..], BlockSize::new(512).unwrap());
    let first = Partition::new(&mut device, 512, 1536);
    let second = Partition::new(first, 0, 1024);
    let mut fs = FileSystem(Cache::new(second));
    let mut a = [0; 512];
    let mut b = [0; 512];
    generic_send(&mut fs, &mut a, &mut b);
    complete(fs.read_pair(BlockIndex::new(0), &mut a, &mut b)).unwrap();
    assert_eq!(a, [1; 512]);
    assert_eq!(b, [2; 512]);
    assert_eq!(fs.0.disk_offset(), 512);
}

#[test]
fn flush_forwards_pending_state_and_preserves_valid_cache() {
    let inner = device(false);
    let flushes = inner.flushes.clone();
    let reads = inner.reads.clone();
    let mut cache = Cache::new(inner);
    let mut buffer = [0; 512];
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    complete(cache.flush()).unwrap();
    assert_eq!(flushes.get(), 1);
    complete(cache.read_blocks(BlockIndex::new(0), &mut buffer)).unwrap();
    assert_eq!(reads.get(), 1);
}
