mod common;

use core::convert::Infallible;
use core::future::Future;
use core::task::{Context, Poll, Waker};
use std::cell::Cell;
use std::rc::Rc;

use hadris_fs::{Content, MountOptions, Name, Node, Resolve, Tree};
use hadris_io::{Error, ErrorKind, ErrorType};
use hadris_storage::async_::{BlockDevice, Cache, ReadAhead, SendBlockDevice};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};
use hadris_udf::UdfOptions;
use hadris_udf::async_::{UdfFs, write, write_bridge};

struct LocalDevice {
    inner: MemDevice<Vec<u8>>,
    cancelled: Rc<Cell<usize>>,
    polls: Rc<Cell<usize>>,
    active: bool,
    fail_write: bool,
    fail_flush: bool,
}
impl ErrorType for LocalDevice {
    type Error = Infallible;
}
impl LocalDevice {
    fn pause(&mut self, state: &mut bool, cx: &mut Context<'_>) -> bool {
        self.polls.set(self.polls.get() + 1);
        if !*state {
            assert!(!self.active);
            self.active = true;
            *state = true;
            cx.waker().wake_by_ref();
            true
        } else {
            self.active = false;
            false
        }
    }
}
impl BlockDevice for LocalDevice {
    type State = bool;
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        self.inner.get_ref().len() as u64 / 512
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut bool,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        if self.pause(state, cx) {
            return Poll::Pending;
        }
        Poll::Ready(hadris_storage::sync::BlockDevice::read_blocks(
            &mut self.inner,
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
    ) -> Poll<Result<(), Error<Infallible>>> {
        if self.pause(state, cx) {
            return Poll::Pending;
        }
        Poll::Ready(if self.fail_write {
            Err(Error::new(ErrorKind::Unsupported, "injected write failure"))
        } else {
            hadris_storage::sync::BlockDevice::write_blocks(&mut self.inner, first, buf)
        })
    }
    fn poll_flush(
        &mut self,
        state: &mut bool,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Infallible>>> {
        if self.pause(state, cx) {
            return Poll::Pending;
        }
        Poll::Ready(if self.fail_flush {
            Err(Error::new(ErrorKind::Unsupported, "injected flush failure"))
        } else {
            Ok(())
        })
    }
    fn cancel(&mut self, _: &mut bool) {
        self.active = false;
        self.cancelled.set(self.cancelled.get() + 1);
    }
}
fn local(size: usize) -> LocalDevice {
    LocalDevice {
        inner: MemDevice::new(vec![0; size], BlockSize::new(512).unwrap()),
        cancelled: Rc::default(),
        polls: Rc::default(),
        active: false,
        fail_write: false,
        fail_flush: false,
    }
}
fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("hello.txt", Node::file(Content::bytes("hello world\n")))
        .unwrap();
    tree.insert("link", Node::symlink("hello.txt")).unwrap();
    tree
}
fn require_send(_: impl Future + Send) {}
fn send_contract<D: SendBlockDevice>(fs: &mut UdfFs<D>, buf: &mut [u8]) {
    require_send(fs.resolve(b"/hello.txt", Resolve::Follow));
    require_send(fs.read(fs.root(), 0, buf));
    require_send(fs.fsync(fs.root()));
    require_send(fs.sync());
}
async fn read_file<F: hadris_fs::r#async::FileSystem<DeviceError = Infallible>>(
    fs: &mut F,
) -> [u8; 12] {
    let node = fs.resolve(b"/hello.txt", Resolve::Follow).await.unwrap();
    let mut bytes = [0; 12];
    fs.read(node, 0, &mut bytes).await.unwrap();
    fs.forget(node, 1);
    bytes
}

#[test]
fn send_reader_writer_and_borrowed_adapters_share_the_canonical_namespace() {
    #[allow(unused_imports)]
    use hadris_fs::{r#async::FileSystem as _, local::FileSystem as _};
    let tree = tree();
    let options = UdfOptions::new();
    let size = hadris_udf::plan(&tree, &options).unwrap().size() as usize;
    let mut disk = MemDevice::new(vec![0xA5; size + 4096], BlockSize::new(512).unwrap());
    let mut cached = Cache::new(Partition::new(&mut disk, 2048, size as u64), 8);
    require_send(write(&mut cached, &tree, &options));
    std::thread::scope(|scope| {
        let future = write(&mut cached, &tree, &options);
        scope
            .spawn(move || common::block_on(future))
            .join()
            .unwrap()
            .unwrap();
    });
    drop(cached);
    assert_eq!(&disk.get_ref()[..2048], &[0xA5; 2048]);
    assert_eq!(&disk.get_ref()[size + 2048..], &[0xA5; 2048]);
    let part = Partition::new(&mut disk, 2048, size as u64);
    let mut fs: hadris_udf::r#async::UdfFs<_> = common::block_on(UdfFs::mount(
        ReadAhead::new(Cache::new(part, 8), 8),
        MountOptions::new(),
    ))
    .unwrap();
    let mut buf = [0; 12];
    send_contract(&mut fs, &mut buf);
    std::thread::scope(|scope| {
        let future = read_file(&mut fs);
        assert_eq!(
            scope
                .spawn(move || common::block_on(future))
                .join()
                .unwrap(),
            *b"hello world\n"
        );
    });
}

#[test]
fn local_writers_and_bridge_match_send_images_and_local_volume_reads_symlinks() {
    let tree = tree();
    let options = UdfOptions::new();
    let iso_options = hadris_iso::IsoOptions::new().with_joliet();
    for bridge in [false, true] {
        let size = if bridge {
            hadris_udf::plan_bridge(&tree, &iso_options, &options)
                .unwrap()
                .size()
        } else {
            hadris_udf::plan(&tree, &options).unwrap().size()
        } as usize;
        let mut send = MemDevice::new(vec![0; size], BlockSize::new(512).unwrap());
        let mut device = local(size);
        if bridge {
            require_send(write_bridge(&mut send, &tree, &iso_options, &options));
            common::block_on(write_bridge(&mut send, &tree, &iso_options, &options)).unwrap();
            common::block_on(write_bridge(
                Cache::new(&mut device, 8),
                &tree,
                &iso_options,
                &options,
            ))
            .unwrap();
        } else {
            common::block_on(write(&mut send, &tree, &options)).unwrap();
            common::block_on(write(Cache::new(&mut device, 8), &tree, &options)).unwrap();
        }
        assert_eq!(device.inner.get_ref(), send.get_ref());
        let fs = common::block_on(UdfFs::mount(&mut device, MountOptions::new())).unwrap();
        let volume = hadris_fs::local::Volume::with_resolve(fs, Resolve::Follow);
        common::block_on(async {
            use hadris_io::local::Read as _;
            let mut file = volume
                .open("/link", hadris_fs::OpenOptions::new().read())
                .await
                .unwrap();
            let mut bytes = [0; 12];
            file.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"hello world\n");
            file.close().await.unwrap();
        });
    }
}

#[test]
fn local_cancellation_and_write_flush_errors_release_the_operation() {
    let tree = tree();
    let options = UdfOptions::new();
    let size = hadris_udf::plan(&tree, &options).unwrap().size() as usize;
    let mut device = local(size);
    {
        let mut future = core::pin::pin!(write(&mut device, &tree, &options));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(device.cancelled.get(), 1);
    assert!(!device.active);
    for flush in [false, true] {
        device.fail_write = !flush;
        device.fail_flush = flush;
        assert_eq!(
            common::block_on(write(&mut device, &tree, &options))
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
        assert!(!device.active);
    }
    device.fail_write = false;
    device.fail_flush = false;
    common::block_on(write(&mut device, &tree, &options)).unwrap();
    let mut fs = common::block_on(UdfFs::mount(&mut device, MountOptions::new())).unwrap();
    let node = common::block_on(fs.lookup(fs.root(), Name::new("hello.txt"))).unwrap();
    let mut bytes = [0; 12];
    common::block_on(fs.read(node, 0, &mut bytes)).unwrap();
    assert_eq!(&bytes, b"hello world\n");
}
