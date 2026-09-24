#![cfg(feature = "alloc")]

use hadris_storage::{BlockIndex, BlockSize, MemDevice};

const BLOCK: usize = 16;
const BLOCKS: usize = 64;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn device() -> MemDevice<Vec<u8>> {
    MemDevice::new(
        (0..BLOCKS * BLOCK).map(|i| i as u8).collect(),
        BlockSize::new(BLOCK as u32).unwrap(),
    )
}

#[allow(unused_macros)]
macro_rules! model_test {
    ($block_on:expr) => {{
        for capacity in [1, 2, 3, 7, 16, 100] {
            for seed in 1..40u64 {
                let mut model = device().get_ref().clone();
                let mut cache = Cache::new(device(), capacity);
                let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
                for step in 0..300 {
                    let first = rng.below(BLOCKS);
                    let count = 1 + rng.below((BLOCKS - first).min(20));
                    let range = first * BLOCK..(first + count) * BLOCK;
                    match rng.below(5) {
                        0 | 1 => {
                            let fill = (rng.next() as u8) | 1;
                            let data = vec![fill; count * BLOCK];
                            $block_on(cache.write_blocks(BlockIndex::new(first as u64), &data))
                                .unwrap();
                            model[range].copy_from_slice(&data);
                        }
                        2 | 3 => {
                            let mut buf = vec![0; count * BLOCK];
                            $block_on(cache.read_blocks(BlockIndex::new(first as u64), &mut buf))
                                .unwrap();
                            assert_eq!(
                                buf, model[range],
                                "capacity {capacity} seed {seed} step {step}"
                            );
                        }
                        _ => {
                            $block_on(cache.flush()).unwrap();
                            assert!(!cache.is_dirty());
                            assert_eq!(cache.get_ref().get_ref(), &model);
                        }
                    }
                }
                let device = $block_on(cache.finish()).unwrap();
                assert_eq!(device.get_ref(), &model, "capacity {capacity} seed {seed}");
            }
        }
    }};
}

/// Counts the device calls a cache makes.
#[allow(dead_code)]
struct Counting {
    inner: MemDevice<Vec<u8>>,
    reads: usize,
    writes: usize,
}

impl hadris_io::ErrorType for Counting {
    type Error = core::convert::Infallible;
}

#[cfg(feature = "sync")]
mod sync {
    use super::*;
    use hadris_io::Error;
    use hadris_storage::sync::{BlockDevice, Cache};

    impl BlockDevice for Counting {
        fn block_size(&self) -> BlockSize {
            self.inner.block_size()
        }

        fn block_count(&self) -> u64 {
            self.inner.block_count()
        }

        fn read_blocks(
            &mut self,
            first: BlockIndex,
            buf: &mut [u8],
        ) -> Result<(), Error<Self::Error>> {
            self.reads += 1;
            self.inner.read_blocks(first, buf)
        }

        fn write_blocks(
            &mut self,
            first: BlockIndex,
            buf: &[u8],
        ) -> Result<(), Error<Self::Error>> {
            self.writes += 1;
            self.inner.write_blocks(first, buf)
        }
    }

    fn run<T>(value: T) -> T {
        value
    }

    #[test]
    fn cache_matches_a_plain_device() {
        model_test!(run);
    }

    #[test]
    fn flush_writes_each_dirty_run_once() {
        let counting = Counting {
            inner: device(),
            reads: 0,
            writes: 0,
        };
        let mut cache = Cache::new(counting, 32);
        cache.write_blocks(BlockIndex::new(0), &[1; BLOCK]).unwrap();
        for index in (4..12).chain(20..23) {
            cache
                .write_blocks(BlockIndex::new(index), &[2; BLOCK])
                .unwrap();
        }
        let before = cache.get_ref().writes;
        cache.flush().unwrap();
        assert_eq!(cache.get_ref().writes - before, 2);
        assert!(!cache.is_dirty());
    }

    #[test]
    fn misses_are_read_in_runs_and_large_requests_bypass() {
        let counting = Counting {
            inner: device(),
            reads: 0,
            writes: 0,
        };
        let mut cache = Cache::new(counting, 16);
        let mut buf = [0; 8 * BLOCK];
        cache
            .read_blocks(BlockIndex::new(0), &mut buf[..BLOCK])
            .unwrap();
        cache
            .read_blocks(BlockIndex::new(4), &mut buf[..BLOCK])
            .unwrap();
        let before = cache.get_ref().reads;
        cache.read_blocks(BlockIndex::new(0), &mut buf).unwrap();
        assert_eq!(cache.get_ref().reads - before, 2);

        cache.write_blocks(BlockIndex::new(0), &[9; BLOCK]).unwrap();
        cache.write_blocks(BlockIndex::new(3), &[7; BLOCK]).unwrap();
        let mut big = [0; 16 * BLOCK];
        let before = cache.get_ref().reads;
        cache.read_blocks(BlockIndex::new(0), &mut big).unwrap();
        assert_eq!(cache.get_ref().reads - before, 1);
        assert_eq!(&big[3 * BLOCK..4 * BLOCK], &[7; BLOCK]);

        let before = cache.get_ref().writes;
        cache
            .write_blocks(BlockIndex::new(0), &[5; 16 * BLOCK])
            .unwrap();
        assert_eq!(cache.get_ref().writes - before, 1);
        assert!(!cache.is_dirty());
        cache
            .read_blocks(BlockIndex::new(3), &mut buf[..BLOCK])
            .unwrap();
        assert_eq!(&buf[..BLOCK], &[5; BLOCK]);
    }
}

#[cfg(feature = "async")]
mod r#async {
    use super::*;
    use hadris_storage::r#async::{BlockDevice, Cache};

    fn block_on<F: core::future::Future>(future: F) -> F::Output {
        let mut context = core::task::Context::from_waker(core::task::Waker::noop());
        let mut future = core::pin::pin!(future);
        loop {
            if let core::task::Poll::Ready(out) = future.as_mut().poll(&mut context) {
                return out;
            }
        }
    }

    #[test]
    fn cache_matches_a_plain_device() {
        model_test!(block_on);
    }
}
