#![allow(dead_code)]

//! Dropping `async` operations part way through: a device whose every
//! transfer yields once, and a runner that stops polling after a budget.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

use hadris_fs::r#async::FileSystem;
use hadris_fs::{DirCursor, ErrorKind, Name, RenameMode, SetAttr};
use hadris_io::Error;
use hadris_io::ErrorType;
use hadris_storage::r#async::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

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
    type Error = core::convert::Infallible;
}

impl BlockDevice for YieldDev {
    fn block_size(&self) -> BlockSize {
        BlockDevice::block_size(&self.0)
    }

    fn block_count(&self) -> u64 {
        BlockDevice::block_count(&self.0)
    }

    fn writable(&self) -> bool {
        BlockDevice::writable(&self.0)
    }

    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Self::Error>> {
        YieldOnce(false).await;
        BlockDevice::read_blocks(&mut self.0, first, buf).await
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Self::Error>> {
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

fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn text(value: &str) -> &Name {
    Name::new(value)
}

/// One step of a random workload, which may be dropped part way through.
pub async fn step<F: FileSystem>(fs: &mut F, kind: u64, i: u64) -> Result<(), ErrorKind> {
    let root = fs.root();
    let meta = SetAttr::new();
    let dir_name = format!("directory {}", i % 3);
    let file = format!("file number {}.bin", i % 5);
    let dir = match fs.lookup(root, text(&dir_name)).await {
        Ok(dir) => dir,
        Err(_) if kind == 0 => fs
            .mkdir(root, text(&dir_name), &meta)
            .await
            .map_err(|err| err.kind())?,
        Err(err) => return Err(err.kind()),
    };
    let result = match kind {
        0 => {
            let node = match fs.create(dir, text(&file), &meta).await {
                Ok(node) => node,
                Err(_) => fs
                    .lookup(dir, text(&file))
                    .await
                    .map_err(|err| err.kind())?,
            };
            let data = payload(3000 + (i as usize % 7) * 1500, i as u8);
            let written = fs.write(node, 0, &data).await.map(|_| ());
            let published = fs.close(node).await;
            fs.forget(node, 1);
            written.and(published).map_err(|err| err.kind())
        }
        1 => fs.unlink(dir, text(&file)).await.map_err(|err| err.kind()),
        2 => {
            let to_name = format!("directory {}", (i + 1) % 3);
            match fs.lookup(root, text(&to_name)).await {
                Ok(to) => {
                    let moved = fs
                        .rename(
                            dir,
                            text(&file),
                            to,
                            text(&format!("renamed {}.bin", i % 4)),
                            RenameMode::Replace,
                        )
                        .await;
                    fs.forget(to, 1);
                    moved.map_err(|err| err.kind())
                }
                Err(err) => Err(err.kind()),
            }
        }
        3 => match fs.lookup(dir, text(&file)).await {
            Ok(node) => {
                let set = fs.truncate(node, i * 1777 % 20_000).await;
                let published = fs.close(node).await;
                fs.forget(node, 1);
                set.and(published).map_err(|err| err.kind())
            }
            Err(err) => Err(err.kind()),
        },
        _ => {
            let mut cursor = DirCursor::START;
            let mut names = Vec::new();
            while let Some(entry) = fs.readdir(dir, cursor).await.map_err(|err| err.kind())? {
                names.push(entry.name().to_str().unwrap().to_owned());
                cursor = entry.next_cursor();
            }
            for entry in &names {
                fs.unlink(dir, text(entry))
                    .await
                    .map_err(|err| err.kind())?;
            }
            fs.rmdir(root, text(&dir_name))
                .await
                .map_err(|err| err.kind())
        }
    };
    fs.forget(dir, 1);
    result
}
