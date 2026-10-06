mod common;

use std::cell::Cell;
use std::convert::Infallible;
use std::future::{Future, poll_fn};
use std::rc::Rc;
use std::task::Poll;

use hadris_fs::{MountOptions, Name, OpenOptions};
use hadris_io::{Error, ErrorType};
use hadris_iso::IsoOptions;
use hadris_iso::async_::IsoFs;
use hadris_storage::async_::Local;
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};

struct RcDevice {
    inner: MemDevice<Vec<u8>>,
    polls: Rc<Cell<usize>>,
}

static_assertions::assert_not_impl_any!(RcDevice: Send, Sync);
static_assertions::assert_not_impl_any!(IsoFs<Local<RcDevice>>: Send, Sync);

impl ErrorType for RcDevice {
    type Error = Infallible;
}
impl hadris_storage::async_::BlockDevice for RcDevice {
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 2048
    }
    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Infallible>> {
        let polls = Rc::clone(&self.polls);
        let mut pending = true;
        poll_fn(move |cx| {
            polls.set(polls.get() + 1);
            if pending {
                pending = false;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        hadris_storage::sync::BlockDevice::read_blocks(&mut self.inner, first, buf)
    }
}

impl hadris_storage::local::BlockDevice for RcDevice {
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 2048
    }
    async fn read_blocks(
        &mut self,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), Error<Infallible>> {
        hadris_storage::async_::BlockDevice::read_blocks(self, first, buf).await
    }
}

#[test]
fn canonical_local_device_mounts_without_an_adapter() {
    let inner = common::image(
        &common::sample(false, false),
        &IsoOptions::new().with_joliet(),
    );
    let polls = Rc::new(Cell::new(0));
    let device = RcDevice {
        inner,
        polls: Rc::clone(&polls),
    };
    common::block_on(async {
        let mut fs = IsoFs::mount(device, MountOptions::new()).await.unwrap();
        hadris_fs::local::contract::check_read_only(&mut fs)
            .await
            .unwrap();
        let node = fs.lookup(fs.root(), Name::new("readme.txt")).await.unwrap();
        let mut bytes = [0; 12];
        assert_eq!(fs.read(node, 0, &mut bytes).await.unwrap(), 12);
        assert_eq!(&bytes, b"hello world\n");
        fs.forget(node, 1);
    });
    assert!(polls.get() > 0);
}

fn disk() -> (MemDevice<Vec<u8>>, u64) {
    let image = common::image(
        &common::sample(false, false),
        &IsoOptions::new().with_joliet(),
    );
    let blocks = image.get_ref().len() as u64 / 2048;
    let mut bytes = vec![0xA5; 4 * 2048];
    bytes.extend_from_slice(image.get_ref());
    bytes.extend_from_slice(&[0xA5; 3 * 2048]);
    (MemDevice::new(bytes, common::SECTOR), blocks)
}

fn require_send(_: impl Future + Send) {}

async fn generic_send_read<F: hadris_fs::r#async::FileSystem<DeviceError = Infallible>>(
    fs: &mut F,
) -> Vec<u8> {
    let node = fs.lookup(fs.root(), Name::new("readme.txt")).await.unwrap();
    let mut buf = vec![0; 12];
    assert_eq!(fs.read(node, 0, &mut buf).await.unwrap(), 12);
    fs.forget(node, 1);
    buf
}

#[test]
fn borrowed_send_partition_cache_and_old_path_use_one_reader() {
    let (mut disk, blocks) = disk();
    let part = Partition::new(&mut disk, 4 * 2048, blocks * 2048);
    let cache = hadris_storage::async_::Cache::new(part, 4);
    let ahead = hadris_storage::async_::ReadAhead::new(cache, 8);
    require_send(IsoFs::mount(ahead, MountOptions::new()));
    let part = Partition::new(&mut disk, 4 * 2048, blocks * 2048);
    let cache = hadris_storage::async_::Cache::new(part, 4);
    let ahead = hadris_storage::async_::ReadAhead::new(cache, 8);
    let mut fs: hadris_iso::r#async::IsoFs<_> =
        common::block_on(IsoFs::mount(ahead, MountOptions::new())).unwrap();
    assert_eq!(
        hadris_storage::async_::BlockDevice::disk_offset(fs.device()),
        4 * 2048
    );
    let node = common::block_on(fs.lookup(fs.root(), Name::new("readme.txt"))).unwrap();
    let mut data = [0; 12];
    require_send(fs.read(node, 0, &mut data));
    require_send(generic_send_read(&mut fs));
    std::thread::scope(|scope| {
        let future = generic_send_read(&mut fs);
        assert_eq!(
            scope
                .spawn(move || common::block_on(future))
                .join()
                .unwrap(),
            b"hello world\n"
        );
    });
    fs.forget(node, 1);
}

#[test]
fn borrowed_local_partition_cache_and_volume_use_the_same_reader() {
    let (inner, blocks) = disk();
    let polls = Rc::new(Cell::new(0));
    let mut disk = RcDevice {
        inner,
        polls: Rc::clone(&polls),
    };
    let part = Partition::new(&mut disk, 4 * 2048, blocks * 2048);
    let cache = hadris_storage::local::Cache::new(part, 4);
    let ahead = hadris_storage::local::ReadAhead::new(cache, 8);
    common::block_on(async {
        let mut fs = IsoFs::mount(Local::new(ahead), MountOptions::new())
            .await
            .unwrap();
        hadris_fs::local::contract::check_read_only(&mut fs)
            .await
            .unwrap();
        let vol = hadris_fs::local::Volume::new(fs);
        let mut file = vol
            .open("/readme.txt", OpenOptions::new().read())
            .await
            .unwrap();
        use hadris_io::local::Read as _;
        let mut bytes = [0; 12];
        file.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"hello world\n");
        file.close().await.unwrap();
        assert!(vol.into_inner().await.is_ok());
    });
    assert!(polls.get() > 0);
}

#[test]
fn unified_reader_still_works_with_send_volume() {
    let (mut disk, blocks) = disk();
    let part = Partition::new(&mut disk, 4 * 2048, blocks * 2048);
    common::block_on(async {
        let fs = IsoFs::mount(part, MountOptions::new()).await.unwrap();
        let vol = hadris_fs::r#async::Volume::new(fs);
        let mut file = vol
            .open("/readme.txt", OpenOptions::new().read())
            .await
            .unwrap();
        use hadris_io::r#async::Read as _;
        let mut bytes = [0; 12];
        require_send(file.read_exact(&mut bytes));
        file.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"hello world\n");
        file.close().await.unwrap();
    });
}
