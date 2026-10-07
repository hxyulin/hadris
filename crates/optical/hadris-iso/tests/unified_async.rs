mod common;

use std::cell::Cell;
use std::convert::Infallible;
use std::future::Future;
use std::rc::Rc;
use std::task::Poll;

use hadris_fs::{MountOptions, Name, OpenOptions};
use hadris_io::{Error, ErrorType};
use hadris_iso::IsoOptions;
use hadris_iso::async_::IsoFs;
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};

struct RcDevice {
    inner: MemDevice<Vec<u8>>,
    polls: Rc<Cell<usize>>,
}

static_assertions::assert_not_impl_any!(RcDevice: Send, Sync);
static_assertions::assert_not_impl_any!(IsoFs<RcDevice>: Send, Sync);

impl ErrorType for RcDevice {
    type Error = Infallible;
}
impl hadris_storage::async_::BlockDevice for RcDevice {
    type State = bool;
    fn block_size(&self) -> BlockSize {
        common::SECTOR
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 2048
    }
    fn cancel(&mut self, _: &mut bool) {}
    fn poll_read_blocks(
        &mut self,
        pending: &mut bool,
        cx: &mut core::task::Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        self.polls.set(self.polls.get() + 1);
        if !*pending {
            *pending = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(hadris_storage::sync::BlockDevice::read_blocks(
                &mut self.inner,
                first,
                buf,
            ))
        }
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
    let cache = hadris_storage::async_::Cache::new(part, 4);
    let ahead = hadris_storage::async_::ReadAhead::new(cache, 8);
    common::block_on(async {
        let mut fs = IsoFs::mount(ahead, MountOptions::new()).await.unwrap();
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

fn send_defaults<D: hadris_storage::async_::SendBlockDevice>(fs: &mut IsoFs<D>) {
    use hadris_fs::{RenameMode, Resolve, SetAttr};
    let root = fs.root();
    let name = Name::new("new.txt");
    let attrs = SetAttr::new();
    require_send(fs.resolve(b"/readme.txt", Resolve::Follow));
    require_send(fs.setattr(root, &attrs));
    require_send(fs.write(root, 0, b"new"));
    require_send(fs.truncate(root, 0));
    require_send(fs.create(root, name, &attrs));
    require_send(fs.mkdir(root, name, &attrs));
    require_send(fs.unlink(root, name));
    require_send(fs.rmdir(root, name));
    require_send(fs.rename(
        root,
        name,
        root,
        Name::new("renamed.txt"),
        RenameMode::Replace,
    ));
    require_send(fs.fsync(root));
    require_send(fs.sync());
}

#[test]
fn defaults_remain_send_and_unambiguous_with_both_filesystem_contracts_imported() {
    use hadris_fs::{ErrorKind, Resolve};
    #[allow(unused_imports)]
    use hadris_fs::{r#async::FileSystem as _, local::FileSystem as _};
    let image = common::image(
        &common::sample(false, false),
        &IsoOptions::new().with_joliet(),
    );
    let mut fs = common::block_on(IsoFs::mount(image, MountOptions::new())).unwrap();
    send_defaults(&mut fs);
    let node = common::block_on(fs.resolve(b"/readme.txt", Resolve::Follow)).unwrap();
    assert_eq!(
        common::block_on(fs.write(node, 0, b"new"))
            .unwrap_err()
            .kind(),
        ErrorKind::ReadOnly
    );
    common::block_on(fs.fsync(node)).unwrap();
    common::block_on(fs.sync()).unwrap();
    fs.forget(node, 1);
}
