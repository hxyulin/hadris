use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::convert::Infallible;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use hadris_experiment_async_device_contract::{
    BlockDevice, FileSystem, Operation, SendBlockDevice,
};
use hadris_io::{Error, ErrorKind, ErrorType};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};

const SIZE: BlockSize = BlockSize::new(4).unwrap();
type Audit = Arc<Mutex<Vec<&'static str>>>;

#[derive(Default)]
struct State {
    started: bool,
    audit: Option<Audit>,
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(audit) = &self.audit {
            audit.lock().unwrap().push("state dropped");
        }
    }
}
#[derive(Default)]
struct LocalState {
    inner: State,
    _local: Rc<()>,
}
trait StateAccess: Default + Unpin {
    fn inner(&mut self) -> &mut State;
}
impl StateAccess for State {
    fn inner(&mut self) -> &mut State {
        self
    }
}
impl StateAccess for LocalState {
    fn inner(&mut self) -> &mut State {
        &mut self.inner
    }
}
struct PendingDevice<S, L = ()> {
    bytes: [u8; 32],
    audit: Audit,
    active: bool,
    fail: bool,
    _state: PhantomData<fn() -> S>,
    _local: L,
}
impl<S, L: Default> PendingDevice<S, L> {
    fn new(audit: Audit) -> Self {
        Self {
            bytes: core::array::from_fn(|i| i as u8),
            audit,
            active: false,
            fail: false,
            _state: PhantomData,
            _local: L::default(),
        }
    }
    fn advance(
        &mut self,
        state: &mut State,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Infallible>>> {
        if !state.started {
            assert!(!self.active);
            self.active = true;
            state.started = true;
            state.audit = Some(Arc::clone(&self.audit));
            self.audit.lock().unwrap().push("pending");
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            assert!(self.active);
            self.active = false;
            self.audit.lock().unwrap().push("ready");
            Poll::Ready(if self.fail {
                Err(Error::new(ErrorKind::Unsupported, "injected failure"))
            } else {
                Ok(())
            })
        }
    }
}
impl<S, L> ErrorType for PendingDevice<S, L> {
    type Error = Infallible;
}
impl<S: StateAccess, L: Default> BlockDevice for PendingDevice<S, L> {
    type State = S;
    fn block_size(&self) -> BlockSize {
        SIZE
    }
    fn block_count(&self) -> u64 {
        8
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        match self.advance(state.inner(), cx) {
            Poll::Ready(Ok(())) => {
                let start = first.get() as usize * 4;
                buf.copy_from_slice(&self.bytes[start..start + buf.len()]);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
    fn poll_write_blocks(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        match self.advance(state.inner(), cx) {
            Poll::Ready(Ok(())) => {
                let start = first.get() as usize * 4;
                self.bytes[start..start + buf.len()].copy_from_slice(buf);
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
    fn poll_flush(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Infallible>>> {
        self.advance(state.inner(), cx)
    }
    fn cancel(&mut self, state: &mut S) {
        assert!(self.active);
        assert!(state.inner().started);
        self.active = false;
        self.audit.lock().unwrap().push("cancelled");
    }
}

static_assertions::assert_impl_all!(PendingDevice<State>: Send, SendBlockDevice);
static_assertions::assert_impl_all!(PendingDevice<LocalState>: Send);
static_assertions::assert_not_impl_any!(PendingDevice<LocalState>: SendBlockDevice);
static_assertions::assert_not_impl_any!(Operation<'static, PendingDevice<LocalState>>: Send);
static_assertions::assert_not_impl_any!(PendingDevice<State, Rc<()>>: Send, SendBlockDevice);
static_assertions::assert_not_impl_any!(Operation<'static, PendingDevice<State, Rc<()>>>: Send);

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = core::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}
fn require_send(_: impl Future + Send) {}
fn send_contract<D: SendBlockDevice>(fs: &mut FileSystem<D>, a: &mut [u8], b: &mut [u8]) {
    require_send(fs.read_pair(BlockIndex::new(0), a, b));
}
fn pending<F: Future + Unpin>(future: &mut F) {
    assert!(
        Pin::new(future)
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}
fn audit() -> Audit {
    Arc::new(Mutex::new(Vec::new()))
}

#[test]
fn generic_send_bound_and_borrowed_nested_partitions_cross_scoped_threads() {
    let mut bytes = core::array::from_fn::<_, 32, _>(|i| i as u8);
    let mut device = MemDevice::new(&mut bytes[..], SIZE);
    let outer = Partition::new(&mut device, 4, 24);
    let inner = Partition::new(outer, 4, 16);
    assert_eq!(inner.disk_offset(), 8);
    let mut fs = FileSystem(inner);
    let mut a = [0; 4];
    let mut b = [0; 4];
    send_contract(&mut fs, &mut a, &mut b);
    std::thread::scope(|scope| {
        let future = fs.read_pair(BlockIndex::new(0), &mut a, &mut b);
        scope
            .spawn(move || block_on(future))
            .join()
            .unwrap()
            .unwrap();
    });
    assert_eq!(a, [8, 9, 10, 11]);
    assert_eq!(b, [12, 13, 14, 15]);
}

#[test]
fn local_devices_and_send_devices_with_local_state_use_the_same_reader() {
    let events = audit();
    let mut device = PendingDevice::<State, Rc<()>>::new(Arc::clone(&events));
    let mut fs = FileSystem(Partition::new(&mut device, 4, 16));
    let mut a = [0; 4];
    let mut b = [0; 4];
    block_on(fs.read_pair(BlockIndex::new(0), &mut a, &mut b)).unwrap();
    assert_eq!(a, [4, 5, 6, 7]);
    assert_eq!(b, [8, 9, 10, 11]);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "pending",
            "ready",
            "state dropped",
            "pending",
            "ready",
            "state dropped"
        ]
    );
    let mut fs = FileSystem(PendingDevice::<LocalState>::new(audit()));
    block_on(fs.read_pair(BlockIndex::new(0), &mut a, &mut b)).unwrap();
    assert_eq!(a, [0, 1, 2, 3]);
    assert_eq!(b, [4, 5, 6, 7]);
}

#[test]
fn dropping_pending_read_write_and_flush_cancels_before_state_and_borrows_release() {
    let events = audit();
    let mut device = PendingDevice::<State>::new(Arc::clone(&events));
    let mut buf = [0xA5; 4];
    let mut read = device.read_blocks(BlockIndex::new(0), &mut buf);
    pending(&mut read);
    drop(read);
    assert!(!device.active);
    assert_eq!(buf, [0xA5; 4]);
    assert_eq!(
        *events.lock().unwrap(),
        ["pending", "cancelled", "state dropped"]
    );
    events.lock().unwrap().clear();
    let mut part = Partition::new(&mut device, 4, 16);
    let mut write = part.write_blocks(BlockIndex::new(0), &[0xBB; 4]);
    pending(&mut write);
    drop(write);
    assert!(!part.get_ref().active);
    assert_eq!(
        *events.lock().unwrap(),
        ["pending", "cancelled", "state dropped"]
    );
    events.lock().unwrap().clear();
    let mut flush = part.flush();
    pending(&mut flush);
    drop(flush);
    assert!(!part.get_ref().active);
    assert_eq!(
        *events.lock().unwrap(),
        ["pending", "cancelled", "state dropped"]
    );
    block_on(part.write_blocks(BlockIndex::new(0), &[0xCC; 4])).unwrap();
    block_on(part.read_blocks(BlockIndex::new(0), &mut buf)).unwrap();
    assert_eq!(buf, [0xCC; 4]);
}

#[test]
fn unused_operations_do_not_start_or_cancel_io() {
    let events = audit();
    let mut device = PendingDevice::<State>::new(Arc::clone(&events));
    let mut buf = [0; 4];
    drop(device.read_blocks(BlockIndex::new(0), &mut buf));
    drop(device.write_blocks(BlockIndex::new(0), &buf));
    drop(device.flush());
    assert!(events.lock().unwrap().is_empty());
    assert!(!device.active);
}

#[test]
fn invalid_ranges_fail_before_io_and_pending_errors_do_not_cancel_completed_io() {
    let events = audit();
    let mut device = PendingDevice::<State>::new(Arc::clone(&events));
    let mut buf = [0; 4];
    for first in [8, u64::MAX] {
        assert_eq!(
            block_on(device.read_blocks(BlockIndex::new(first), &mut buf))
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
    }
    assert_eq!(
        block_on(device.read_blocks(BlockIndex::new(0), &mut buf[..3]))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    let mut part = Partition::new(&mut device, 1, 16);
    assert_eq!(
        block_on(part.read_blocks(BlockIndex::new(0), &mut buf))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    let mut part = Partition::new(&mut device, 4, 8);
    assert_eq!(
        block_on(part.write_blocks(BlockIndex::new(2), &buf))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    assert!(events.lock().unwrap().is_empty());
    device.fail = true;
    assert_eq!(
        block_on(device.read_blocks(BlockIndex::new(0), &mut buf))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    assert!(!device.active);
    assert_eq!(
        *events.lock().unwrap(),
        ["pending", "ready", "state dropped"]
    );
}

#[test]
fn cancelling_the_reader_between_reads_releases_the_second_operation() {
    let events = audit();
    let mut device = PendingDevice::<State>::new(Arc::clone(&events));
    let mut a = [0xA5; 4];
    let mut b = [0xA5; 4];
    {
        let mut fs = FileSystem(&mut device);
        let mut future = core::pin::pin!(fs.read_pair(BlockIndex::new(0), &mut a, &mut b));
        pending(&mut future);
        pending(&mut future);
    }
    assert_eq!(a, [0, 1, 2, 3]);
    assert_eq!(b, [0xA5; 4]);
    assert!(!device.active);
    assert_eq!(
        *events.lock().unwrap(),
        [
            "pending",
            "ready",
            "state dropped",
            "pending",
            "cancelled",
            "state dropped"
        ]
    );
    block_on(device.read_blocks(BlockIndex::new(1), &mut b)).unwrap();
    assert_eq!(b, [4, 5, 6, 7]);
}

#[test]
fn partition_extent_past_backing_device_is_rejected_before_driver_hooks() {
    let events = audit();
    let mut device = PendingDevice::<State>::new(Arc::clone(&events));
    let mut buf = [0; 4];
    let mut partition = Partition::new(&mut device, 28, 8);
    let mut read = partition.read_blocks(BlockIndex::new(1), &mut buf);
    match Pin::new(&mut read).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Err(error)) => assert_eq!(error.kind(), ErrorKind::InvalidInput),
        other => panic!(
            "partition must reject translated out-of-range IO before invoking the device: {other:?}"
        ),
    }
    drop(read);
    let mut write = partition.write_blocks(BlockIndex::new(1), &buf);
    match Pin::new(&mut write).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Err(error)) => assert_eq!(error.kind(), ErrorKind::InvalidInput),
        other => panic!(
            "partition must reject translated out-of-range writes before invoking the device: {other:?}"
        ),
    }
    drop(write);
    assert!(events.lock().unwrap().is_empty());
}
