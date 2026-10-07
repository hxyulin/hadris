#![cfg(feature = "alloc")]

use hadris_storage::{BlockIndex, BlockSize, MemDevice};

fn image() -> MemDevice<Vec<u8>> {
    MemDevice::new(
        (0..4096).map(|n| (n / 16) as u8).collect(),
        BlockSize::new(16).unwrap(),
    )
}

#[allow(unused_macros)]
macro_rules! model {
    ($run:expr) => {
        for capacity in [0, 1, 2, 3, 16, 63, 1024] {
            let mut expected = image().into_inner();
            let mut dev = ReadAhead::new(image(), capacity);
            let mut seed = 42u64;
            for step in 0..1000 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let first = seed as usize % 256;
                let count = (1 + (seed >> 16) as usize % 20).min(256 - first);
                let range = first * 16..(first + count) * 16;
                if step % 5 == 0 {
                    let data = vec![step as u8; count * 16];
                    $run(dev.write_blocks(BlockIndex::new(first as u64), &data)).unwrap();
                    expected[range].copy_from_slice(&data);
                } else {
                    let mut data = vec![0; count * 16];
                    $run(dev.read_blocks(BlockIndex::new(first as u64), &mut data)).unwrap();
                    assert_eq!(data, expected[range], "capacity {capacity}, step {step}");
                }
            }
            assert_eq!(dev.into_inner().into_inner(), expected);
        }
    };
}

#[cfg(feature = "sync")]
mod sync {
    use super::*;
    use hadris_io::{Error, ErrorKind, ErrorType};
    use hadris_storage::Partition;
    use hadris_storage::sync::{BlockDevice, ReadAhead};

    struct Probe {
        inner: MemDevice<Vec<u8>>,
        reads: Vec<(u64, usize)>,
        reject_large: bool,
        fail_write: bool,
    }

    impl Probe {
        fn new() -> Self {
            Self {
                inner: image(),
                reads: Vec::new(),
                reject_large: false,
                fail_write: false,
            }
        }
    }
    impl ErrorType for Probe {
        type Error = core::convert::Infallible;
    }
    impl BlockDevice for Probe {
        fn block_size(&self) -> BlockSize {
            self.inner.block_size()
        }
        fn block_count(&self) -> u64 {
            self.inner.block_count()
        }
        fn max_block_count(&self) -> u64 {
            999
        }
        fn disk_offset(&self) -> u64 {
            512
        }
        fn writable(&self) -> bool {
            true
        }
        fn read_blocks(
            &mut self,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Result<(), Error<Self::Error>> {
            self.reads.push((first.get(), buf.len() / 16));
            if self.reject_large && buf.len() > 16 {
                buf.fill(255);
                return Err(Error::new(ErrorKind::Io, "injected failure"));
            }
            self.inner.read_blocks(first, buf)
        }
        fn write_blocks(
            &mut self,
            first: BlockIndex,
            buf: &[u8],
        ) -> Result<(), Error<Self::Error>> {
            if self.fail_write {
                self.inner.write_blocks(first, &buf[..16])?;
                return Err(Error::new(ErrorKind::Io, "injected failure"));
            }
            self.inner.write_blocks(first, buf)
        }
    }

    #[test]
    fn random_reads_and_writes_match_the_device() {
        model!(core::convert::identity);
    }

    #[test]
    fn two_windows_batch_alternating_regions_and_bound_the_tail() {
        let mut dev = ReadAhead::new(Probe::new(), 16);
        let mut buf = [0; 16];
        for block in [0, 128, 1, 129, 7, 135, 255] {
            dev.read_blocks(BlockIndex::new(block), &mut buf).unwrap();
            assert_eq!(buf, [block as u8; 16]);
        }
        assert_eq!(
            dev.get_ref().reads,
            [(0, 1), (128, 1), (1, 8), (129, 8), (255, 1)]
        );
        assert_eq!(dev.max_block_count(), 999);
        assert_eq!(dev.disk_offset(), 512);
        assert!(dev.writable());
    }

    #[test]
    fn failed_speculation_retries_exactly_and_never_publishes_partial_data() {
        let mut probe = Probe::new();
        probe.reject_large = true;
        let mut dev = ReadAhead::new(probe, 16);
        let mut buf = [0; 16];
        for block in [0, 1, 2] {
            dev.read_blocks(BlockIndex::new(block), &mut buf).unwrap();
            assert_eq!(buf, [block as u8; 16]);
        }
        assert_eq!(dev.get_ref().reads, [(0, 1), (1, 8), (1, 1), (2, 1)]);
    }

    #[test]
    fn writes_and_mutable_access_invalidate_even_on_partial_failure() {
        let mut dev = ReadAhead::new(Probe::new(), 16);
        let mut buf = [0; 16];
        dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        dev.get_mut().fail_write = true;
        dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        assert!(dev.write_blocks(BlockIndex::new(0), &[42; 32]).is_err());
        dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        assert_eq!(buf, [42; 16]);
        dev.get_mut().inner.get_mut()[..16].fill(43);
        dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        assert_eq!(buf, [43; 16]);
    }

    #[test]
    fn writes_invalidate_only_the_windows_they_overlap() {
        let mut dev = ReadAhead::new(Probe::new(), 16);
        let mut buf = [0; 16];
        for block in [0, 1, 2] {
            dev.read_blocks(BlockIndex::new(block), &mut buf).unwrap();
        }
        assert_eq!(dev.get_ref().reads, [(0, 1), (1, 8)]);
        dev.write_blocks(BlockIndex::new(200), &[7; 16]).unwrap();
        dev.read_blocks(BlockIndex::new(3), &mut buf).unwrap();
        assert_eq!((buf, dev.get_ref().reads.len()), ([3; 16], 2));
        dev.write_blocks(BlockIndex::new(4), &[7; 16]).unwrap();
        dev.read_blocks(BlockIndex::new(4), &mut buf).unwrap();
        assert_eq!((buf, dev.get_ref().reads.len()), ([7; 16], 3));
    }

    #[test]
    fn replacing_the_device_can_change_its_block_size() {
        let mut dev = ReadAhead::new(image(), 16);
        let mut buf = [0; 16];
        dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        *dev.get_mut() = MemDevice::new(vec![42; 4096], BlockSize::new(32).unwrap());
        let mut buf = [0; 32];
        dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        assert_eq!(buf, [42; 32]);
        dev.read_blocks(BlockIndex::new(1), &mut buf).unwrap();
        assert_eq!(buf, [42; 32]);
    }

    #[test]
    fn disabled_large_and_invalid_requests_preserve_device_behavior() {
        let mut dev = ReadAhead::new(Probe::new(), 0);
        let mut buf = [0; 16];
        for _ in 0..2 {
            dev.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        }
        assert_eq!(dev.get_ref().reads, [(0, 1), (0, 1)]);
        let mut dev = ReadAhead::new(Probe::new(), 16);
        dev.read_blocks(BlockIndex::new(0), &mut [0; 160]).unwrap();
        for (first, len) in [(0, 15), (256, 16), (u64::MAX, 32)] {
            assert!(
                dev.read_blocks(BlockIndex::new(first), &mut vec![0; len])
                    .is_err()
            );
        }
        assert_eq!(dev.get_ref().reads.len(), 4);
        let partition = Partition::new(Probe::new(), 16, 32);
        let mut dev = ReadAhead::new(partition, usize::MAX);
        assert_eq!(dev.capacity(), 2);
        dev.read_blocks(BlockIndex::new(1), &mut buf).unwrap();
        assert_eq!(buf, [2; 16]);
        assert!(dev.read_blocks(BlockIndex::new(2), &mut buf).is_err());
    }
}

#[cfg(feature = "async")]
fn block_on<F: core::future::Future>(future: F) -> F::Output {
    let mut context = core::task::Context::from_waker(core::task::Waker::noop());
    let mut future = core::pin::pin!(future);
    loop {
        if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return out;
        }
    }
}

#[cfg(feature = "async")]
mod async_modes {
    use super::*;
    #[test]
    fn send_model() {
        use hadris_storage::r#async::{BlockDevice, ReadAhead};
        fn send<F: core::future::Future + Send>(future: F) -> F::Output {
            block_on(future)
        }
        model!(send);
    }
    #[test]
    fn local_model() {
        use hadris_storage::local::{BlockDevice, ReadAhead};
        model!(block_on);
    }
}

#[cfg(feature = "async")]
mod cancellation {
    use super::*;
    use hadris_io::{Error, ErrorType};
    use hadris_storage::r#async::{BlockDevice, ReadAhead};

    struct Yielding {
        inner: MemDevice<Vec<u8>>,
        suspend: std::sync::Arc<std::sync::atomic::AtomicBool>,
        reads: usize,
    }
    impl ErrorType for Yielding {
        type Error = core::convert::Infallible;
    }
    impl BlockDevice for Yielding {
        type State = ();
        fn cancel(&mut self, _: &mut ()) {}
        fn block_size(&self) -> BlockSize {
            self.inner.block_size()
        }
        fn block_count(&self) -> u64 {
            256
        }
        fn poll_read_blocks(
            &mut self,
            state: &mut (),
            cx: &mut core::task::Context<'_>,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> core::task::Poll<Result<(), Error<Self::Error>>> {
            self.reads += 1;
            if let core::task::Poll::Ready(Err(e)) =
                self.inner.poll_read_blocks(state, cx, first, buf)
            {
                return core::task::Poll::Ready(Err(e));
            }
            if self.suspend.load(std::sync::atomic::Ordering::Relaxed) {
                buf.fill(255);
                return core::task::Poll::Pending;
            }
            core::task::Poll::Ready(Ok(()))
        }
        fn poll_write_blocks(
            &mut self,
            state: &mut (),
            cx: &mut core::task::Context<'_>,
            first: BlockIndex,
            buf: &[u8],
        ) -> core::task::Poll<Result<(), Error<Self::Error>>> {
            if let core::task::Poll::Ready(Err(e)) =
                self.inner.poll_write_blocks(state, cx, first, buf)
            {
                return core::task::Poll::Ready(Err(e));
            }
            if self.suspend.load(std::sync::atomic::Ordering::Relaxed) {
                return core::task::Poll::Pending;
            }
            core::task::Poll::Ready(Ok(()))
        }
    }
    fn cancel<F: core::future::Future + Send>(future: F) {
        let mut context = core::task::Context::from_waker(core::task::Waker::noop());
        let mut future = core::pin::pin!(future);
        assert!(future.as_mut().poll(&mut context).is_pending());
    }
    #[test]
    fn cancelled_refill_and_write_cannot_leave_stale_windows() {
        let suspend = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut dev = ReadAhead::new(
            Yielding {
                inner: image(),
                suspend: suspend.clone(),
                reads: 0,
            },
            16,
        );
        let mut buf = [0; 16];
        block_on(dev.read_blocks(BlockIndex::new(0), &mut buf)).unwrap();
        block_on(dev.read_blocks(BlockIndex::new(128), &mut buf)).unwrap();
        suspend.store(true, std::sync::atomic::Ordering::Relaxed);
        cancel(dev.read_blocks(BlockIndex::new(32), &mut buf));
        suspend.store(false, std::sync::atomic::Ordering::Relaxed);
        block_on(dev.read_blocks(BlockIndex::new(0), &mut buf)).unwrap();
        assert_eq!(buf, [0; 16]);
        block_on(dev.read_blocks(BlockIndex::new(32), &mut buf)).unwrap();
        assert_eq!(buf, [32; 16]);
        suspend.store(true, std::sync::atomic::Ordering::Relaxed);
        cancel(dev.write_blocks(BlockIndex::new(32), &[42; 16]));
        suspend.store(false, std::sync::atomic::Ordering::Relaxed);
        block_on(dev.read_blocks(BlockIndex::new(32), &mut buf)).unwrap();
        assert_eq!(buf, [42; 16]);
    }

    #[test]
    fn writes_invalidate_only_the_windows_they_overlap() {
        let mut dev = ReadAhead::new(
            Yielding {
                inner: image(),
                suspend: Default::default(),
                reads: 0,
            },
            16,
        );
        let mut buf = [0; 16];
        for block in [0, 1, 2] {
            block_on(dev.read_blocks(BlockIndex::new(block), &mut buf)).unwrap();
        }
        assert_eq!(dev.get_ref().reads, 2);
        block_on(dev.write_blocks(BlockIndex::new(200), &[7; 16])).unwrap();
        block_on(dev.read_blocks(BlockIndex::new(3), &mut buf)).unwrap();
        assert_eq!((buf, dev.get_ref().reads), ([3; 16], 2));
        block_on(dev.write_blocks(BlockIndex::new(4), &[7; 16])).unwrap();
        block_on(dev.read_blocks(BlockIndex::new(4), &mut buf)).unwrap();
        assert_eq!((buf, dev.get_ref().reads), ([7; 16], 3));
    }
}
