//! A panic inside a driver call fails the `Volume` for good: later calls
//! return `Detail::VolumeFailed` instead of reaching a driver left
//! mid-change, and `into_inner` still gives the filesystem back.

#[path = "common/fatfs.rs"]
mod common;

use core::pin::pin;
use core::task::{Context, Poll, Waker};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use common::{CASES, blank, block_on};
use hadris_fs::{Detail, ErrorKind, MountOptions, OpenOptions};
use hadris_io::{Error, ErrorType};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

const INJECTED: &str = "injected device panic";

/// A memory device that panics on a chosen write. `left` counts the writes
/// still allowed; a negative count never panics.
struct Panicky {
    inner: MemDevice<Vec<u8>>,
    left: Arc<AtomicI64>,
}

impl Panicky {
    fn new() -> (Self, Arc<AtomicI64>) {
        let case = CASES[0];
        let left = Arc::new(AtomicI64::new(-1));
        let inner = MemDevice::new(blank(case), BlockSize::new(case.block).unwrap());
        (
            Self {
                inner,
                left: Arc::clone(&left),
            },
            left,
        )
    }

    fn before_write(&self) {
        if self.left.fetch_sub(1, Ordering::SeqCst) == 0 {
            std::panic::panic_any(INJECTED);
        }
    }
}

impl ErrorType for Panicky {
    type Error = core::convert::Infallible;
}

impl hadris_storage::sync::BlockDevice for Panicky {
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        self.inner.read_blocks(first, buf)
    }

    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        self.before_write();
        self.inner.write_blocks(first, buf)
    }
}

/// With `yields` set, every transfer returns `Pending` once first.
struct AsyncPanicky {
    dev: Panicky,
    yields: bool,
}

impl ErrorType for AsyncPanicky {
    type Error = core::convert::Infallible;
}

impl hadris_storage::async_::BlockDevice for AsyncPanicky {
    fn block_size(&self) -> BlockSize {
        self.dev.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.dev.inner.block_count()
    }

    fn writable(&self) -> bool {
        true
    }

    type State = bool;

    fn poll_read_blocks(
        &mut self,
        state: &mut bool,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if self.yields && !*state {
            *state = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(hadris_storage::sync::BlockDevice::read_blocks(
            &mut self.dev,
            first,
            buf,
        ))
    }

    fn poll_write_blocks(
        &mut self,
        state: &mut bool,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if self.yields && !*state {
            *state = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(hadris_storage::sync::BlockDevice::write_blocks(
            &mut self.dev,
            first,
            buf,
        ))
    }

    fn poll_flush(
        &mut self,
        _: &mut bool,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(Ok(()))
    }

    fn cancel(&mut self, _: &mut bool) {}
}

/// Silences the injected panics, leaving others to the default hook.
fn quiet_injected_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().downcast_ref::<&str>() != Some(&INJECTED) {
                default(info);
            }
        }));
    });
}

fn assert_failed<T: std::fmt::Debug, E: std::fmt::Debug>(result: Result<T, hadris_fs::Error<E>>) {
    let err = result.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Io);
    assert_eq!(Detail::of(&err), Some(Detail::VolumeFailed));
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Create,
    Rename,
}

mod sync {
    use super::*;
    use hadris_fat::sync::FatFs;
    use hadris_fs::sync::Volume;

    fn setup() -> (Volume<FatFs<Panicky>>, Arc<AtomicI64>) {
        let (dev, left) = Panicky::new();
        let vol = Volume::new(FatFs::mount(dev, MountOptions::new()).unwrap());
        vol.write("/old.txt", b"old").unwrap();
        vol.create_dir("/dir").unwrap();
        (vol, left)
    }

    fn run(
        vol: &Volume<FatFs<Panicky>>,
        op: Op,
    ) -> hadris_fs::FsResult<(), core::convert::Infallible> {
        match op {
            Op::Create => vol.write("/dir/new.txt", b"new contents"),
            Op::Rename => vol.rename("/old.txt", "/dir/moved.txt"),
        }
    }

    #[test]
    fn a_panic_in_a_driver_call_fails_the_volume() {
        quiet_injected_panics();
        for op in [Op::Create, Op::Rename] {
            for nth in 0.. {
                let (vol, left) = setup();
                let open = vol.open("/old.txt", OpenOptions::new().read()).unwrap();
                left.store(nth, Ordering::SeqCst);
                let outcome = catch_unwind(AssertUnwindSafe(|| run(&vol, op)));
                left.store(-1, Ordering::SeqCst);
                if let Ok(done) = outcome {
                    done.unwrap();
                    assert!(!vol.is_failed(), "{op:?}");
                    assert!(nth > 0, "{op:?} wrote nothing");
                    break;
                }
                assert!(vol.is_failed(), "{op:?} write {nth}");
                assert_failed(vol.metadata("/old.txt"));
                assert_failed(vol.write("/other.txt", b"x"));
                assert_failed(vol.rename("/dir", "/dir2"));
                assert_failed(vol.create_dir("/again"));
                assert_failed(vol.read_dir("/").map(drop));
                let mut buf = [0u8; 4];
                let mut file = open;
                assert_failed(file.read(&mut buf));
                assert_failed(file.metadata());
                drop(file);
                assert_failed(vol.lock().map(drop));
                let fs = vol.into_inner().unwrap();
                let image = fs.into_inner().inner.into_inner();
                assert_eq!(image.len() as u64, CASES[0].size);
            }
        }
    }
}

mod asynchronous {
    use super::*;
    use hadris_fat::async_::FatFs;
    use hadris_fs::async_::Volume;

    type Vol = Volume<FatFs<AsyncPanicky>>;

    fn setup(yields: bool) -> (Vol, Arc<AtomicI64>) {
        let (dev, left) = Panicky::new();
        let dev = AsyncPanicky { dev, yields };
        let vol = Volume::new(block_on(FatFs::mount(dev, MountOptions::new())).unwrap());
        block_on(async {
            vol.write("/old.txt", b"old").await.unwrap();
            vol.create_dir("/dir").await.unwrap();
        });
        (vol, left)
    }

    async fn run(vol: &Vol, op: Op) -> hadris_fs::FsResult<(), core::convert::Infallible> {
        match op {
            Op::Create => vol.write("/dir/new.txt", b"new contents").await,
            Op::Rename => vol.rename("/old.txt", "/dir/moved.txt").await,
        }
    }

    fn assert_volume_failed(vol: Vol) {
        assert!(vol.is_failed());
        block_on(async {
            assert_failed(vol.metadata("/old.txt").await);
            assert_failed(vol.write("/other.txt", b"x").await);
            assert_failed(vol.rename("/dir", "/dir2").await);
            assert_failed(vol.read_dir("/").await.map(drop));
        });
        assert_failed(block_on(vol.lock()).map(drop));
        let fs = block_on(vol.into_inner()).unwrap();
        let image = fs.into_inner().dev.inner.into_inner();
        assert_eq!(image.len() as u64, CASES[0].size);
    }

    #[test]
    fn a_panic_unwinding_through_the_executor_fails_the_volume() {
        quiet_injected_panics();
        for op in [Op::Create, Op::Rename] {
            for nth in 0.. {
                let (vol, left) = setup(false);
                left.store(nth, Ordering::SeqCst);
                let outcome = catch_unwind(AssertUnwindSafe(|| block_on(run(&vol, op))));
                left.store(-1, Ordering::SeqCst);
                if let Ok(done) = outcome {
                    done.unwrap();
                    assert!(!vol.is_failed());
                    assert!(nth > 0, "{op:?} wrote nothing");
                    break;
                }
                assert_volume_failed(vol);
            }
        }
    }

    /// As `FutureExt::catch_unwind` does: the panicked future is dropped
    /// only after the unwind ended.
    #[test]
    fn a_panic_caught_around_a_poll_fails_the_volume() {
        quiet_injected_panics();
        for op in [Op::Create, Op::Rename] {
            for nth in 0.. {
                let (vol, left) = setup(true);
                left.store(nth, Ordering::SeqCst);
                let mut cx = Context::from_waker(Waker::noop());
                let mut call = Box::pin(run(&vol, op));
                let outcome = loop {
                    match catch_unwind(AssertUnwindSafe(|| call.as_mut().poll(&mut cx))) {
                        Ok(Poll::Pending) => continue,
                        Ok(Poll::Ready(done)) => break Ok(done),
                        Err(_) => break Err(()),
                    }
                };
                left.store(-1, Ordering::SeqCst);
                if let Ok(done) = outcome {
                    drop(call);
                    done.unwrap();
                    assert!(!vol.is_failed());
                    assert!(nth > 0, "{op:?} wrote nothing");
                    break;
                }
                assert!(vol.is_failed(), "{op:?} write {nth}");
                drop(call);
                assert_volume_failed(vol);
            }
        }
    }

    #[test]
    fn a_cancelled_call_does_not_fail_the_volume() {
        let (vol, _) = setup(true);
        let mut cx = Context::from_waker(Waker::noop());
        {
            let mut call = pin!(run(&vol, Op::Create));
            assert!(call.as_mut().poll(&mut cx).is_pending());
        }
        assert!(!vol.is_failed());
        block_on(async {
            vol.write("/after.txt", b"ok").await.unwrap();
            assert_eq!(vol.read("/after.txt").await.unwrap(), b"ok");
        });
    }
}
