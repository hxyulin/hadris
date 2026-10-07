#![cfg(all(feature = "async", feature = "alloc"))]

use core::task::{Context, Poll, Waker};
use hadris_io::{Error, ErrorKind};
use hadris_storage::async_::{BlockDevice, Cache, ReadAhead};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

fn invalid<E>(result: Poll<Result<(), Error<E>>>) {
    match result {
        Poll::Ready(Err(error)) => assert_eq!(error.kind(), ErrorKind::InvalidInput),
        _ => panic!("invalid block request was not rejected"),
    }
}

fn check<D: BlockDevice>(device: &mut D) {
    let mut cx = Context::from_waker(Waker::noop());
    for (first, len) in [(0, 1), (0, 5), (3, 8), (4, 4), (u64::MAX, 4)] {
        let mut buf = vec![42; len];
        invalid(device.poll_read_blocks(
            &mut D::State::default(),
            &mut cx,
            BlockIndex::new(first),
            &mut buf,
        ));
        assert_eq!(buf, vec![42; len]);
        invalid(device.poll_write_blocks(
            &mut D::State::default(),
            &mut cx,
            BlockIndex::new(first),
            &buf,
        ));
    }
}

fn device() -> MemDevice<Vec<u8>> {
    MemDevice::new(vec![7; 16], BlockSize::new(4).unwrap())
}

#[test]
fn cache_poll_hooks_reject_invalid_ranges_without_changing_dirty_data() {
    let mut cache = Cache::new(device(), 8);
    let mut cx = Context::from_waker(Waker::noop());
    assert!(matches!(
        cache.poll_write_blocks(
            &mut Default::default(),
            &mut cx,
            BlockIndex::new(0),
            &[8; 4]
        ),
        Poll::Ready(Ok(()))
    ));
    assert!(matches!(
        cache.poll_write_blocks(
            &mut Default::default(),
            &mut cx,
            BlockIndex::new(1),
            &[9; 4]
        ),
        Poll::Ready(Ok(()))
    ));
    assert!(cache.is_dirty());
    check(&mut cache);
    assert!(cache.is_dirty());
    let mut buf = [0; 4];
    assert!(matches!(
        cache.poll_read_blocks(
            &mut Default::default(),
            &mut cx,
            BlockIndex::new(1),
            &mut buf
        ),
        Poll::Ready(Ok(()))
    ));
    assert_eq!(buf, [9; 4]);
}

#[test]
fn read_ahead_poll_hooks_reject_invalid_ranges() {
    for capacity in [0, 1, 8] {
        check(&mut ReadAhead::new(device(), capacity));
    }
}

#[cfg(all(feature = "sync", feature = "std"))]
#[test]
fn fixed_stream_poll_hooks_reject_invalid_ranges_without_touching_the_stream() {
    use hadris_storage::async_::{BlockingStream, StreamDevice};
    let cursor = hadris_io::StdIo::new(std::io::Cursor::new(vec![7; 16]));
    let mut stream =
        StreamDevice::with_block_count(BlockingStream::new(cursor), BlockSize::new(4).unwrap(), 4);
    check(&mut stream);
    let cursor = stream.into_inner().into_inner().into_inner();
    assert_eq!(cursor.position(), 0);
    assert_eq!(cursor.into_inner(), vec![7; 16]);
}

/// Fails the first multi-block write while its flag is set.
#[derive(Debug)]
struct FailOnce(MemDevice<Vec<u8>>, bool);

impl hadris_io::ErrorType for FailOnce {
    type Error = core::convert::Infallible;
}

impl BlockDevice for FailOnce {
    type State = ();
    fn block_size(&self) -> BlockSize {
        BlockDevice::block_size(&self.0)
    }
    fn block_count(&self) -> u64 {
        BlockDevice::block_count(&self.0)
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut (),
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        self.0.poll_read_blocks(state, cx, first, buf)
    }
    fn poll_write_blocks(
        &mut self,
        state: &mut (),
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if self.1 && buf.len() > 4 {
            self.1 = false;
            return Poll::Ready(Err(Error::new(ErrorKind::Io, "failed write")));
        }
        self.0.poll_write_blocks(state, cx, first, buf)
    }
    fn cancel(&mut self, _: &mut ()) {}
}

fn ready<F: core::future::Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(Waker::noop());
    let mut future = core::pin::pin!(future);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(out) => out,
        Poll::Pending => panic!("memory device future was pending"),
    }
}

#[test]
fn cache_finish_returns_the_cache_when_its_flush_fails() {
    let mut cache = Cache::new(FailOnce(device(), true), 4);
    for (index, fill) in [(3, 3), (0, 1), (1, 2)] {
        ready(cache.write_blocks(BlockIndex::new(index), &[fill; 4])).unwrap();
    }
    let (cache, error) = ready(cache.finish()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert!(cache.is_dirty());
    let device = ready(cache.finish()).unwrap();
    assert_eq!(&device.0.get_ref()[..8], &[1, 1, 1, 1, 2, 2, 2, 2]);
}
