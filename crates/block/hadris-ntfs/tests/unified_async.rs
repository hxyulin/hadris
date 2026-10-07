#![cfg(feature = "async")]
#[path = "support/image.rs"]
mod image;
use core::future::Future;
use core::marker::PhantomData;
use core::task::{Context, Poll, Waker};
use hadris_io::{Error, ErrorType};
use hadris_storage::async_::{BlockDevice, SendBlockDevice};
use hadris_storage::{BlockIndex, BlockSize, MemBuffer, MemDevice};
use std::rc::Rc;

fn run<F: Future>(future: F) -> F::Output {
    let mut cx = Context::from_waker(Waker::noop());
    let mut future = core::pin::pin!(future);
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(&mut cx) {
            return result;
        }
    }
}
trait Progress: Default + Unpin {
    fn pending(&mut self) -> &mut bool;
}
impl Progress for bool {
    fn pending(&mut self) -> &mut bool {
        self
    }
}
#[derive(Default)]
struct LocalState {
    pending: bool,
    _local: Rc<()>,
}
impl Progress for LocalState {
    fn pending(&mut self) -> &mut bool {
        &mut self.pending
    }
}
struct LocalBytes(Rc<Vec<u8>>);
impl MemBuffer for LocalBytes {
    fn bytes(&self) -> &[u8] {
        &self.0
    }
    fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        None
    }
    fn writable(&self) -> bool {
        false
    }
}
struct Device<B, S> {
    inner: MemDevice<B>,
    _state: PhantomData<fn() -> S>,
}
impl<B> Device<B, bool> {
    fn new(inner: MemDevice<B>) -> Self {
        Self {
            inner,
            _state: PhantomData,
        }
    }
}
impl<B, S> ErrorType for Device<B, S> {
    type Error = core::convert::Infallible;
}
impl<B: MemBuffer, S: Progress> BlockDevice for Device<B, S> {
    type State = S;
    fn block_size(&self) -> BlockSize {
        self.inner.block_size()
    }
    fn block_count(&self) -> u64 {
        BlockDevice::block_count(&self.inner)
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
        first: BlockIndex,
        bytes: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if !*state.pending() {
            *state.pending() = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        self.inner.poll_read_blocks(&mut (), cx, first, bytes)
    }
    fn cancel(&mut self, _: &mut S) {}
}

async fn read_image<D: BlockDevice>(device: D) -> Vec<u8> {
    let mut fs = hadris_ntfs::async_::NtfsFs::mount(device, hadris_fs::MountOptions::new())
        .await
        .unwrap();
    fn local_contract(_: &impl hadris_fs::local::FileSystem) {}
    local_contract(&fs);
    let root = fs.root();
    let file = fs
        .lookup(root, hadris_fs::Name::new("HELLO.TXT"))
        .await
        .unwrap();
    let mut bytes = vec![0; 64];
    let n = fs.read(file, 0, &mut bytes).await.unwrap();
    bytes.truncate(n);
    bytes
}
fn send_read<D: SendBlockDevice>(device: D) -> impl Future<Output = Vec<u8>> + Send {
    read_image(device)
}
#[test]
fn local_rc_devices_and_send_devices_with_local_state_use_one_filesystem() {
    let bytes = image::base_image();
    let local = Device::new(MemDevice::new(
        LocalBytes(Rc::new(bytes.clone())),
        BlockSize::new(512).unwrap(),
    ));
    assert_eq!(run(read_image(local)), b"hello ntfs".to_vec());
    let with_local_state = Device::<_, LocalState> {
        inner: MemDevice::new(bytes, BlockSize::new(512).unwrap()),
        _state: PhantomData,
    };
    fn send_value(_: &impl Send) {}
    send_value(&with_local_state);
    assert_eq!(run(read_image(with_local_state)), b"hello ntfs".to_vec());
}
#[test]
fn borrowed_send_future_runs_on_a_scoped_thread() {
    let bytes = image::base_image();
    let mut device = Device::new(MemDevice::new(
        bytes.as_slice(),
        BlockSize::new(512).unwrap(),
    ));
    let future = send_read(&mut device);
    let result = std::thread::scope(|scope| scope.spawn(move || run(future)).join().unwrap());
    assert_eq!(result, b"hello ntfs".to_vec());
}

#[test]
fn default_methods_are_unambiguous_and_send_with_both_traits_in_scope() {
    fn check<D: SendBlockDevice>(fs: &mut hadris_ntfs::async_::NtfsFs<D>) {
        use hadris_fs::{Name, RenameMode, Resolve, SetAttr};
        #[allow(unused_imports)]
        use hadris_fs::{async_::FileSystem as _, local::FileSystem as _};
        fn require_send(_: impl Future + Send) {}
        let root = hadris_ntfs::async_::NtfsFs::root(fs);
        let attrs = SetAttr::new();
        let name = Name::new("new");
        require_send(fs.resolve(b"/", Resolve::Follow));
        require_send(fs.setattr(root, &attrs));
        require_send(fs.write(root, 0, &[]));
        require_send(fs.truncate(root, 0));
        require_send(fs.fsync(root));
        require_send(fs.create(root, name, &attrs));
        require_send(fs.mkdir(root, name, &attrs));
        require_send(fs.unlink(root, name));
        require_send(fs.rmdir(root, name));
        require_send(fs.rename(root, name, root, name, RenameMode::Replace));
        require_send(fs.sync());
    }
    let device = MemDevice::new(image::base_image(), BlockSize::new(512).unwrap());
    let mut fs = run(hadris_ntfs::async_::NtfsFs::mount(
        device,
        hadris_fs::MountOptions::new(),
    ))
    .unwrap();
    check(&mut fs);
}
