use hadris_fat_raw::io::{BlockBuf, sync as io};
use hadris_io::{Error, ErrorKind, ErrorType, Location};
use hadris_storage::{BlockIndex, BlockSize};

type Result<T> = std::result::Result<T, Error<std::io::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Read(u64, usize),
    Write(u64, usize),
}

#[derive(Clone)]
struct Device {
    bytes: Vec<u8>,
    size: usize,
    base: u64,
    calls: Vec<Call>,
    fail_at: Option<usize>,
}

impl Device {
    fn new(size: usize, base: u64) -> Self {
        Self {
            bytes: (0..32 * size).map(|i| (i as u8).wrapping_mul(17)).collect(),
            size,
            base,
            calls: Vec::new(),
            fail_at: None,
        }
    }

    fn range(&self, first: BlockIndex, len: usize) -> Result<std::ops::Range<usize>> {
        assert_eq!(len % self.size, 0);
        let at = first
            .get()
            .checked_sub(self.base)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| index.checked_mul(self.size));
        match at.and_then(|at| at.checked_add(len).map(|end| at..end)) {
            Some(range) if range.end <= self.bytes.len() => Ok(range),
            _ => Err(Error::new(ErrorKind::InvalidInput, "outside test device")
                .with_location(Location::Block(first.get()))),
        }
    }

    fn failure(&self, first: BlockIndex) -> Result<()> {
        if self.fail_at == Some(self.calls.len() - 1) {
            Err(Error::device(
                std::io::Error::from_raw_os_error(5),
                "injected transfer failure",
            )
            .with_location(Location::Block(first.get())))
        } else {
            Ok(())
        }
    }

    fn read(&mut self, first: BlockIndex, out: &mut [u8]) -> Result<()> {
        self.calls.push(Call::Read(first.get(), out.len()));
        let range = self.range(first, out.len())?;
        let result = self.failure(first);
        let n = if result.is_ok() {
            out.len()
        } else {
            out.len() / 2
        };
        out[..n].copy_from_slice(&self.bytes[range.start..range.start + n]);
        result
    }

    fn write(&mut self, first: BlockIndex, data: &[u8]) -> Result<()> {
        self.calls.push(Call::Write(first.get(), data.len()));
        let range = self.range(first, data.len())?;
        let result = self.failure(first);
        let n = if result.is_ok() {
            data.len()
        } else {
            data.len() / 2
        };
        self.bytes[range.start..range.start + n].copy_from_slice(&data[..n]);
        result
    }
}

impl ErrorType for Device {
    type Error = std::io::Error;
}

impl hadris_storage::sync::BlockDevice for Device {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(self.size as u32).unwrap()
    }

    fn block_count(&self) -> u64 {
        self.base + (self.bytes.len() / self.size) as u64
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, out: &mut [u8]) -> Result<()> {
        self.read(first, out)
    }

    fn write_blocks(&mut self, first: BlockIndex, data: &[u8]) -> Result<()> {
        self.write(first, data)
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Read,
    Write,
    Zero,
}

fn perform(
    op: Op,
    dev: &mut Device,
    block: &mut BlockBuf,
    offset: u64,
    data: &[u8],
    out: &mut [u8],
) -> Result<()> {
    match op {
        Op::Read => io::read_bytes(dev, block, offset, out),
        Op::Write => io::write_bytes(dev, block, offset, data),
        Op::Zero => io::write_zeros(dev, block, offset, data.len()),
    }
}

fn assert_cache(dev: &Device, block: &BlockBuf) {
    if let Some(index) = block.cached() {
        let at = ((index - dev.base) as usize) * dev.size;
        assert_eq!(block.contents(), &dev.bytes[at..at + dev.size]);
    }
}

fn expected_bytes(op: Op, before: &[u8], offset: usize, data: &[u8]) -> Vec<u8> {
    let mut bytes = before.to_vec();
    match op {
        Op::Read => {}
        Op::Write => bytes[offset..offset + data.len()].copy_from_slice(data),
        Op::Zero => bytes[offset..offset + data.len()].fill(0),
    }
    bytes
}

fn check_ranges<const N: usize>() {
    for size in [1, 3, 7, 16, 31, 512, 1000, 4096]
        .into_iter()
        .filter(|&size| size <= N)
    {
        for base in [0, (1u64 << 40) / size as u64, u64::MAX / size as u64 - 32] {
            for offset in [0, 1, size - 1, size, size + 1, 2 * size - 1] {
                for len in [
                    0,
                    1,
                    size - 1,
                    size,
                    size + 1,
                    3 * size - 1,
                    3 * size + 1,
                    10 * size + 7,
                ] {
                    for warm in [false, true] {
                        for op in [Op::Read, Op::Write, Op::Zero] {
                            let mut dev = Device::new(size, base);
                            let before = dev.bytes.clone();
                            let mut block = BlockBuf::<[u8; N]>::new(size).unwrap();
                            if warm {
                                io::load(&mut dev, &mut block, base + (offset / size) as u64)
                                    .unwrap();
                                dev.calls.clear();
                            }
                            let cached = block.cached();
                            let data: Vec<_> =
                                (0..len).map(|i| (i as u8).wrapping_add(93)).collect();
                            let mut out = vec![0; len];
                            perform(
                                op,
                                &mut dev,
                                &mut block,
                                base * size as u64 + offset as u64,
                                &data,
                                &mut out,
                            )
                            .unwrap();
                            assert_eq!(
                                dev.bytes,
                                expected_bytes(op, &before, offset, &data),
                                "{op:?}: size {size}, offset {offset}, len {len}"
                            );
                            if matches!(op, Op::Read) {
                                assert_eq!(out, before[offset..offset + len]);
                            }
                            if len == 0 {
                                assert!(dev.calls.is_empty());
                                assert_eq!(block.cached(), cached);
                            }
                            assert_cache(&dev, &block);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn byte_ranges_preserve_neighbors_for_arbitrary_block_sizes_and_large_offsets() {
    check_ranges::<17>();
    check_ranges::<512>();
    check_ranges::<4096>();
}

#[test]
fn transfers_keep_bulk_requests_and_partial_block_cache_hits() {
    use Call::{Read as R, Write as W};
    for (op, expected) in [
        (Op::Read, vec![R(0, 7), R(1, 21), R(4, 7)]),
        (
            Op::Write,
            vec![R(0, 7), W(0, 7), W(1, 21), R(4, 7), W(4, 7)],
        ),
        (
            Op::Zero,
            vec![R(0, 7), W(0, 7), W(1, 14), W(3, 7), R(4, 7), W(4, 7)],
        ),
    ] {
        for warm in [false, true] {
            let mut dev = Device::new(7, 0);
            let mut block = BlockBuf::<[u8; 17]>::new(7).unwrap();
            if warm {
                io::load(&mut dev, &mut block, 0).unwrap();
                dev.calls.clear();
            }
            perform(op, &mut dev, &mut block, 3, &[42; 27], &mut [0; 27]).unwrap();
            assert_eq!(dev.calls, expected[usize::from(warm)..]);
            assert_eq!(block.cached(), Some(4));
            assert_cache(&dev, &block);
        }
    }
}

#[test]
fn bulk_writes_invalidate_only_overlapping_cached_blocks() {
    for cached in 0..5 {
        let mut dev = Device::new(7, 0);
        let mut block = BlockBuf::<[u8; 17]>::new(7).unwrap();
        io::load(&mut dev, &mut block, cached).unwrap();
        dev.calls.clear();
        io::write_bytes(&mut dev, &mut block, 7, &[42; 21]).unwrap();
        assert_eq!(dev.calls, [Call::Write(1, 21)]);
        assert_eq!(
            block.cached(),
            if (1..4).contains(&cached) {
                None
            } else {
                Some(cached)
            }
        );
        assert_cache(&dev, &block);
    }
}

#[test]
fn empty_byte_operations_preserve_the_cache_even_at_the_maximum_offset() {
    let mut dev = Device::new(7, 0);
    let mut block = BlockBuf::<[u8; 17]>::new(7).unwrap();
    io::load(&mut dev, &mut block, 4).unwrap();
    dev.calls.clear();
    for op in [Op::Read, Op::Write, Op::Zero] {
        perform(op, &mut dev, &mut block, u64::MAX, &[], &mut []).unwrap();
        assert!(dev.calls.is_empty());
        assert_eq!(block.cached(), Some(4));
        assert_cache(&dev, &block);
    }
}

#[test]
fn out_of_range_transfers_stop_at_the_first_refusal() {
    for op in [Op::Read, Op::Write, Op::Zero] {
        let mut dev = Device::new(7, 0);
        let mut block = BlockBuf::<[u8; 17]>::new(7).unwrap();
        let error =
            perform(op, &mut dev, &mut block, 31 * 7 + 3, &[42; 8], &mut [0; 8]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert_eq!(error.location(), Some(Location::Block(32)));
        assert_eq!(dev.calls.last(), Some(&Call::Read(32, 7)));
        assert_eq!(dev.calls.len(), if matches!(op, Op::Read) { 2 } else { 3 });
        assert_eq!(block.cached(), None);
        assert_cache(&dev, &block);
    }
}

#[test]
fn failed_transfers_never_leave_a_stale_cached_block() {
    for op in [Op::Read, Op::Write, Op::Zero] {
        for warm in [None, Some(0), Some(2), Some(8)] {
            for offset in [3, 7] {
                for failed in 0..7 {
                    let mut dev = Device::new(7, 0);
                    let mut block = BlockBuf::<[u8; 17]>::new(7).unwrap();
                    if let Some(index) = warm {
                        io::load(&mut dev, &mut block, index).unwrap();
                        dev.calls.clear();
                    }
                    dev.fail_at = Some(failed);
                    let result = perform(op, &mut dev, &mut block, offset, &[42; 27], &mut [0; 27]);
                    assert_cache(&dev, &block);
                    if let Err(error) = result {
                        assert_eq!(dev.calls.len(), failed + 1);
                        assert_eq!(error.device_error().unwrap().raw_os_error(), Some(5));
                        let index = match dev.calls[failed] {
                            Call::Read(index, _) | Call::Write(index, _) => index,
                        };
                        assert_eq!(error.location(), Some(Location::Block(index)));
                    } else {
                        assert!(dev.calls.len() <= failed);
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(feature = "async")]
mod asynchronous {
    use super::*;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    #[derive(Default)]
    pub(crate) struct Transfer {
        phase: u8,
        result: Option<Result<()>>,
    }

    impl hadris_storage::async_::BlockDevice for Device {
        type State = Transfer;
        fn block_size(&self) -> BlockSize {
            BlockSize::new(self.size as u32).unwrap()
        }
        fn block_count(&self) -> u64 {
            self.base + (self.bytes.len() / self.size) as u64
        }
        fn writable(&self) -> bool {
            true
        }
        fn poll_read_blocks(
            &mut self,
            state: &mut Transfer,
            cx: &mut Context<'_>,
            first: BlockIndex,
            out: &mut [u8],
        ) -> Poll<Result<()>> {
            match state.phase {
                0 => {
                    state.phase = 1;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                1 => {
                    state.result = Some(self.read(first, out));
                    state.phase = 2;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                _ => Poll::Ready(state.result.take().unwrap()),
            }
        }
        fn poll_write_blocks(
            &mut self,
            state: &mut Transfer,
            cx: &mut Context<'_>,
            first: BlockIndex,
            data: &[u8],
        ) -> Poll<Result<()>> {
            match state.phase {
                0 => {
                    state.phase = 1;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                1 => {
                    state.result = Some(self.write(first, data));
                    state.phase = 2;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                _ => Poll::Ready(state.result.take().unwrap()),
            }
        }
        fn poll_flush(&mut self, _: &mut Transfer, _: &mut Context<'_>) -> Poll<Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn cancel(&mut self, state: &mut Transfer) {
            *state = Transfer::default();
        }
    }

    fn run_for<F: Future>(future: F, polls: usize) -> Option<F::Output> {
        let mut cx = Context::from_waker(Waker::noop());
        let mut future = pin!(future);
        for _ in 0..polls {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return Some(value);
            }
        }
        None
    }

    async fn perform_async(
        local: bool,
        op: Op,
        dev: &mut Device,
        block: &mut BlockBuf,
        offset: u64,
        data: &[u8],
        out: &mut [u8],
    ) -> Result<()> {
        use hadris_fat_raw::io::{r#async as a, local as l};
        match (local, op) {
            (false, Op::Read) => a::read_bytes(dev, block, offset, out).await,
            (false, Op::Write) => a::write_bytes(dev, block, offset, data).await,
            (false, Op::Zero) => a::write_zeros(dev, block, offset, data.len()).await,
            (true, Op::Read) => l::read_bytes(dev, block, offset, out).await,
            (true, Op::Write) => l::write_bytes(dev, block, offset, data).await,
            (true, Op::Zero) => l::write_zeros(dev, block, offset, data.len()).await,
        }
    }

    #[test]
    fn dropped_transfers_preserve_cache_and_can_be_retried_in_both_async_modes() {
        for local in [false, true] {
            for size in [3, 7, 512] {
                for op in [Op::Read, Op::Write, Op::Zero] {
                    for offset in [1, size as u64] {
                        let initial = Device::new(size, 0);
                        let data = vec![42; size * 5 + 1];
                        let mut expected_dev = initial.clone();
                        let mut expected_block = BlockBuf::<[u8; 1024]>::new(size).unwrap();
                        perform(
                            op,
                            &mut expected_dev,
                            &mut expected_block,
                            offset,
                            &data,
                            &mut vec![0; data.len()],
                        )
                        .unwrap();
                        for warm in [false, true] {
                            for polls in 0..=2 * expected_dev.calls.len() + 1 {
                                let mut dev = initial.clone();
                                let mut block = BlockBuf::<[u8; 1024]>::new(size).unwrap();
                                if warm {
                                    io::load(&mut dev, &mut block, offset / size as u64).unwrap();
                                    dev.calls.clear();
                                }
                                let mut out = vec![0; data.len()];
                                let result = run_for(
                                    perform_async(
                                        local, op, &mut dev, &mut block, offset, &data, &mut out,
                                    ),
                                    polls,
                                );
                                assert_cache(&dev, &block);
                                let skip = usize::from(warm && offset == 1);
                                assert_eq!(
                                    dev.calls,
                                    expected_dev.calls[skip..skip + dev.calls.len()]
                                );
                                let mut prefix = initial.bytes.clone();
                                for call in &dev.calls {
                                    if let Call::Write(index, len) = *call {
                                        let start = (index as usize * size).max(offset as usize);
                                        let end = (index as usize * size + len)
                                            .min(offset as usize + data.len());
                                        match op {
                                            Op::Write => prefix[start..end].copy_from_slice(
                                                &data[start - offset as usize
                                                    ..end - offset as usize],
                                            ),
                                            Op::Zero => prefix[start..end].fill(0),
                                            Op::Read => unreachable!(),
                                        }
                                    }
                                }
                                assert_eq!(dev.bytes, prefix);
                                if let Some(result) = result {
                                    result.unwrap();
                                }
                                run_for(
                                    perform_async(
                                        local, op, &mut dev, &mut block, offset, &data, &mut out,
                                    ),
                                    1000,
                                )
                                .unwrap()
                                .unwrap();
                                assert_eq!(dev.bytes, expected_dev.bytes);
                                if matches!(op, Op::Read) {
                                    assert_eq!(
                                        out,
                                        initial.bytes
                                            [offset as usize..offset as usize + data.len()]
                                    );
                                }
                                assert_cache(&dev, &block);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn failed_transfers_preserve_device_errors_in_both_async_modes() {
        for local in [false, true] {
            for op in [Op::Read, Op::Write, Op::Zero] {
                for cached in [0, 2, 8] {
                    for offset in [3, 7] {
                        for failed in 0..7 {
                            let mut dev = Device::new(7, 0);
                            let mut block = BlockBuf::<[u8; 17]>::new(7).unwrap();
                            io::load(&mut dev, &mut block, cached).unwrap();
                            dev.calls.clear();
                            dev.fail_at = Some(failed);
                            let result = run_for(
                                perform_async(
                                    local,
                                    op,
                                    &mut dev,
                                    &mut block,
                                    offset,
                                    &[42; 27],
                                    &mut [0; 27],
                                ),
                                1000,
                            )
                            .unwrap();
                            assert_cache(&dev, &block);
                            if let Err(error) = result {
                                assert_eq!(dev.calls.len(), failed + 1);
                                assert_eq!(error.device_error().unwrap().raw_os_error(), Some(5));
                                let index = match dev.calls[failed] {
                                    Call::Read(index, _) | Call::Write(index, _) => index,
                                };
                                assert_eq!(error.location(), Some(Location::Block(index)));
                            } else {
                                assert!(dev.calls.len() <= failed);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}
