//! Single-threaded filesystem access using an Rc-backed device and suspending futures.

use std::cell::Cell;
use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use hadris_fs::local::{FileSystem, Volume};
use hadris_fs::{Content, MountOptions, Node, OpenOptions, Tree};
use hadris_io::local::Read as _;
use hadris_io::{Error, ErrorType};
use hadris_storage::local::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

struct RcDevice {
    inner: MemDevice<Vec<u8>>,
    reads: Rc<Cell<usize>>,
}

static_assertions::assert_not_impl_any!(RcDevice: Send, Sync);
static_assertions::assert_not_impl_any!(hadris_iso::local::IsoFs<RcDevice>: Send, Sync);

impl RcDevice {
    fn new(inner: MemDevice<Vec<u8>>) -> Self {
        Self {
            inner,
            reads: Rc::new(Cell::new(0)),
        }
    }

    async fn suspend(&self) {
        let state = Rc::clone(&self.reads);
        let mut pending = true;
        poll_fn(move |cx| {
            state.set(state.get() + 1);
            if pending {
                pending = false;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
    }
}

impl ErrorType for RcDevice {
    type Error = Infallible;
}

impl BlockDevice for RcDevice {
    fn block_size(&self) -> BlockSize {
        hadris_storage::sync::BlockDevice::block_size(&self.inner)
    }

    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(&self.inner)
    }

    fn writable(&self) -> bool {
        true
    }

    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Infallible>> {
        self.suspend().await;
        hadris_storage::sync::BlockDevice::read_blocks(&mut self.inner, first, buf)
    }

    async fn write_blocks(
        &mut self,
        first: BlockIndex,
        buf: &[u8],
    ) -> Result<(), Error<Infallible>> {
        self.suspend().await;
        hadris_storage::sync::BlockDevice::write_blocks(&mut self.inner, first, buf)
    }

    async fn flush(&mut self) -> Result<(), Error<Infallible>> {
        self.suspend().await;
        Ok(())
    }
}

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("caf\u{e9}.txt", Node::file(Content::bytes("local async")))
        .unwrap();
    tree
}

async fn read_file<F: FileSystem<DeviceError = Infallible>>(vol: &Volume<F>) {
    let mut file = vol
        .open("/caf\u{e9}.txt", OpenOptions::new().read())
        .await
        .unwrap();
    let mut bytes = [0u8; 11];
    file.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"local async");
    file.close().await.unwrap();
    let mut entries = vol.read_dir("/").await.unwrap();
    let mut found = false;
    while let Some(entry) = entries.next_entry().await {
        found |= entry.unwrap().name().as_bytes() == "caf\u{e9}.txt".as_bytes();
    }
    assert!(found);
}

fn fat_roundtrip(exfat: bool) {
    block_on(async {
        let mut dev = RcDevice::new(MemDevice::new(
            vec![0; 32 * 1024 * 1024],
            BlockSize::new(512).unwrap(),
        ));
        let reads = Rc::clone(&dev.reads);
        if exfat {
            hadris_fat::exfat::local::format(&mut dev, &hadris_fat::exfat::ExFatOptions::new())
                .await
                .unwrap();
            let fs = hadris_fat::exfat::local::ExFatFs::mount(dev, MountOptions::new())
                .await
                .unwrap();
            write_file(fs).await;
        } else {
            hadris_fat::local::format(&mut dev, &hadris_fat::FatOptions::new())
                .await
                .unwrap();
            let fs = hadris_fat::local::FatFs::mount(dev, MountOptions::new())
                .await
                .unwrap();
            write_file(fs).await;
        }
        assert!(reads.get() > 0);
    });
}

async fn write_file<F: FileSystem<DeviceError = Infallible>>(fs: F) {
    let vol = Volume::new(fs);
    let mut file = vol
        .open("/caf\u{e9}.txt", OpenOptions::new().write().create())
        .await
        .unwrap();
    assert_eq!(file.write(b"local async").await.unwrap(), 11);
    file.close().await.unwrap();
    read_file(&vol).await;
    let mut fs = vol.into_inner().await.ok().unwrap();
    fs.sync().await.unwrap();
}

fn optical_roundtrip(udf: bool) {
    let tree = tree();
    if udf {
        let opts = hadris_udf::UdfOptions::new();
        let size = hadris_udf::plan(&tree, &opts).unwrap().size() as usize;
        let mut dev = MemDevice::new(vec![0; size], BlockSize::new(2048).unwrap());
        hadris_udf::sync::write(&mut dev, &tree, &opts).unwrap();
        block_on(async {
            let mut fs = hadris_udf::local::UdfFs::mount(RcDevice::new(dev), MountOptions::new())
                .await
                .unwrap();
            hadris_fs::local::contract::check_read_only(&mut fs)
                .await
                .unwrap();
            read_file(&Volume::new(fs)).await;
        });
    } else {
        let opts = hadris_iso::IsoOptions::new().with_joliet();
        let size = hadris_iso::plan(&tree, &opts).unwrap().size() as usize;
        let mut dev = MemDevice::new(vec![0; size], BlockSize::new(2048).unwrap());
        hadris_iso::sync::write(&mut dev, &tree, &opts).unwrap();
        block_on(async {
            let mut fs = hadris_iso::local::IsoFs::mount(RcDevice::new(dev), MountOptions::new())
                .await
                .unwrap();
            hadris_fs::local::contract::check_read_only(&mut fs)
                .await
                .unwrap();
            read_file(&Volume::new(fs)).await;
        });
    }
}

fn main() {
    fat_roundtrip(false);
    fat_roundtrip(true);
    optical_roundtrip(false);
    optical_roundtrip(true);
    println!("Local FAT, exFAT, ISO and UDF access passed.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fat_non_send_device_and_future() {
        fat_roundtrip(false);
    }
    #[test]
    fn exfat_non_send_device_and_future() {
        fat_roundtrip(true);
    }
    #[test]
    fn iso_non_send_device_and_future() {
        optical_roundtrip(false);
    }
    #[test]
    fn udf_non_send_device_and_future() {
        optical_roundtrip(true);
    }

    #[test]
    fn cancelled_local_open_releases_the_volume_lock() {
        let opts = hadris_iso::IsoOptions::new().with_joliet();
        let tree = tree();
        let size = hadris_iso::plan(&tree, &opts).unwrap().size() as usize;
        let mut dev = MemDevice::new(vec![0; size], BlockSize::new(2048).unwrap());
        hadris_iso::sync::write(&mut dev, &tree, &opts).unwrap();
        let fs = block_on(hadris_iso::local::IsoFs::mount(
            RcDevice::new(dev),
            MountOptions::new(),
        ))
        .unwrap();
        let vol = Volume::new(fs);
        let clone = vol.clone();
        let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut pending = Box::pin(vol.open("/caf\u{e9}.txt", OpenOptions::new().read()));
        assert!(pending.as_mut().poll(&mut cx).is_pending());
        drop(pending);
        block_on(read_file(&clone));
        drop(clone);
        assert!(block_on(vol.into_inner()).is_ok());
    }

    #[test]
    fn existing_async_tier_retains_send_futures() {
        fn require_send(_: impl Future + Send) {}
        let dev = MemDevice::new(vec![0; 2048], BlockSize::new(2048).unwrap());
        require_send(hadris_iso::r#async::IsoFs::mount(dev, MountOptions::new()));
    }
}
