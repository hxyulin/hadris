#![allow(dead_code)]

//! Dropping `async` operations part way through: a device whose every
//! transfer yields once, and a runner that stops polling after a budget.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

use hadris_io::ErrorType;
use hadris_storage::r#async::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice, OutOfRange, WriteError};

/// A memory device whose reads and writes each return `Pending` once
/// before they happen.
pub struct YieldDev(pub MemDevice<Vec<u8>>);

struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

impl ErrorType for YieldDev {
    type Error = OutOfRange;
}

impl BlockDevice for YieldDev {
    fn block_size(&self) -> BlockSize {
        BlockDevice::block_size(&self.0)
    }

    fn block_count(&self) -> u64 {
        BlockDevice::block_count(&self.0)
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), OutOfRange> {
        YieldOnce(false).await;
        BlockDevice::read_blocks(&mut self.0, first, buf).await
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), WriteError<OutOfRange>> {
        YieldOnce(false).await;
        BlockDevice::write_blocks(&mut self.0, first, buf).await
    }
}

/// Polls `future` at most `polls` times. `None` when it was dropped
/// unfinished.
pub fn run_for<F: Future>(future: F, polls: usize) -> Option<F::Output> {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    for _ in 0..polls {
        if let Poll::Ready(out) = future.as_mut().poll(&mut context) {
            return Some(out);
        }
    }
    None
}

/// A small xorshift generator, so runs repeat.
pub struct Rng(pub u64);

impl Rng {
    pub fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}
