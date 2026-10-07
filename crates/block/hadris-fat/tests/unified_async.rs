use core::cell::Cell;
use core::marker::PhantomData;
use core::task::{Context, Poll};
use std::rc::Rc;

#[path = "common/fatfs.rs"]
mod fat;

use hadris_fs::{MountOptions, Name, SetAttr};
use hadris_io::{Error, ErrorType};
use hadris_storage::async_::{BlockDevice, SendBlockDevice};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

fn exfat_image() -> Vec<u8> {
    let mut dev = MemDevice::new(vec![0; 8 << 20], BlockSize::new(512).unwrap());
    hadris_fat::exfat::sync::format(
        &mut dev,
        &hadris_fat::exfat::ExFatOptions::new().with_cluster_size(512),
    )
    .unwrap();
    dev.into_inner()
}

trait Delay: Default + Unpin {
    fn ready(&mut self) -> bool;
}
impl Delay for bool {
    fn ready(&mut self) -> bool {
        core::mem::replace(self, true)
    }
}
impl Delay for Rc<Cell<bool>> {
    fn ready(&mut self) -> bool {
        self.replace(true)
    }
}
struct Device<M, S> {
    inner: MemDevice<Vec<u8>>,
    _marker: M,
    _state: PhantomData<fn() -> S>,
}
impl<M, S> Device<M, S> {
    fn new(image: Vec<u8>, marker: M) -> Self {
        Self {
            inner: MemDevice::new(image, BlockSize::new(512).unwrap()),
            _marker: marker,
            _state: PhantomData,
        }
    }
}
impl<M, S> core::fmt::Debug for Device<M, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Device")
    }
}
impl<M, S> ErrorType for Device<M, S> {
    type Error = core::convert::Infallible;
}
impl<M, S: Delay> BlockDevice for Device<M, S> {
    type State = S;
    fn block_size(&self) -> BlockSize {
        BlockDevice::block_size(&self.inner)
    }
    fn block_count(&self) -> u64 {
        BlockDevice::block_count(&self.inner)
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if !state.ready() {
            cx.waker().wake_by_ref();
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
        state: &mut S,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if !state.ready() {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(hadris_storage::sync::BlockDevice::write_blocks(
            &mut self.inner,
            first,
            buf,
        ))
    }
    fn poll_flush(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if !state.ready() {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(Ok(()))
    }
    fn cancel(&mut self, _: &mut S) {}
}

fn is_send<T: Send>(_: &T) {}
fn send_filesystem<T: hadris_fs::async_::FileSystem>(_: &T) {}

macro_rules! cache {
    (FatFs, $fs:expr) => {
        $fs.with_cache(hadris_fat::CacheOptions::new())
    };
    (ExFatFs, $fs:expr) => {
        $fs
    };
}
macro_rules! roundtrip {
    ($module:path, $driver:ident, $device:expr) => {{
        use $module as driver;
        let mut fs = cache!(
            $driver,
            fat::block_on(driver::$driver::mount($device, MountOptions::new())).unwrap()
        );
        let root = fs.root();
        let node =
            fat::block_on(fs.create(root, Name::new("unified.bin"), &SetAttr::new())).unwrap();
        assert_eq!(
            fat::block_on(fs.write(node, 513, b"poll devices")).unwrap(),
            12
        );
        fat::block_on(fs.close(node)).unwrap();
        let mut output = [0; 525];
        assert_eq!(fat::block_on(fs.read(node, 0, &mut output)).unwrap(), 525);
        assert_eq!(&output[..513], &[0; 513]);
        assert_eq!(&output[513..], b"poll devices");
        fat::block_on(fs.sync()).unwrap();
        fs
    }};
}

#[test]
fn fat_and_exfat_accept_local_devices_and_local_state() {
    let image = fat::build(fat::CASES[0]);
    roundtrip!(
        hadris_fat::async_,
        FatFs,
        Device::<_, bool>::new(image.clone(), Rc::new(()))
    );
    let state_local = Device::<_, Rc<Cell<bool>>>::new(image, ());
    is_send(&state_local);
    roundtrip!(hadris_fat::async_, FatFs, state_local);

    let image = exfat_image();
    roundtrip!(
        hadris_fat::exfat::async_,
        ExFatFs,
        Device::<_, bool>::new(image.clone(), Rc::new(()))
    );
    let state_local = Device::<_, Rc<Cell<bool>>>::new(image, ());
    is_send(&state_local);
    roundtrip!(hadris_fat::exfat::async_, ExFatFs, state_local);
}

#[test]
fn borrowed_send_devices_keep_send_driver_futures() {
    let mut device = Device::<_, bool>::new(fat::build(fat::CASES[0]), ());
    fn marker<D: SendBlockDevice>(_: &D) {}
    marker(&device);
    let mount = hadris_fat::async_::FatFs::mount(&mut device, MountOptions::new());
    is_send(&mount);
    let mut fs = fat::block_on(mount)
        .unwrap()
        .with_cache(hadris_fat::CacheOptions::new());
    send_filesystem(&fs);
    let resolve = fs.resolve(b"/README.TXT", hadris_fs::Resolve::default());
    is_send(&resolve);
    fat::block_on(resolve).unwrap();
    let root = fs.root();
    let read = fs.lookup(root, Name::new("README.TXT"));
    is_send(&read);
    let node =
        std::thread::scope(|scope| scope.spawn(|| fat::block_on(read)).join().unwrap()).unwrap();
    let write = fs.write(node, 0, b"unified");
    is_send(&write);
    std::thread::scope(|scope| scope.spawn(|| fat::block_on(write)).join().unwrap()).unwrap();
    let sync = fs.sync();
    is_send(&sync);
    fat::block_on(sync).unwrap();

    let mut device = Device::<_, bool>::new(exfat_image(), ());
    let mut fs = roundtrip!(hadris_fat::exfat::async_, ExFatFs, &mut device);
    send_filesystem(&fs);
    let sync = fs.sync();
    is_send(&sync);
    std::thread::scope(|scope| scope.spawn(|| fat::block_on(sync)).join().unwrap()).unwrap();
}

#[test]
fn local_tree_writers_and_embedded_readers_roundtrip() {
    use hadris_fat::embedded::MountToken;
    use hadris_fs::{Content, Node, OpenOptions, Tree};
    let mut tree = Tree::new();
    tree.insert("LOCAL.BIN", Node::file(Content::bytes(b"tree contents")))
        .unwrap();
    let mut dev = Device::<_, Rc<Cell<bool>>>::new(vec![0; 2 << 20], Rc::new(()));
    fat::block_on(hadris_fat::async_::write(
        &mut dev,
        &tree,
        &hadris_fat::FatOptions::new().with_kind(hadris_fat::FatKind::Fat12),
    ))
    .unwrap();
    let mut token = MountToken::new();
    let mut fs = fat::block_on(hadris_fat::embedded::async_::Fat::<_, 4>::mount(
        dev, &mut token,
    ))
    .unwrap();
    let root = fs.root();
    let file = fat::block_on(fs.open(root, "LOCAL.BIN", OpenOptions::new().read())).unwrap();
    let mut out = [0; 13];
    assert_eq!(fat::block_on(fs.read(&file, &mut out)).unwrap(), 13);
    assert_eq!(&out, b"tree contents");
    fat::block_on(fs.close(file)).unwrap();
    let file =
        fat::block_on(fs.open(root, "CREATED.BIN", OpenOptions::new().write().create())).unwrap();
    assert_eq!(fat::block_on(fs.write(&file, b"embedded")).unwrap(), 8);
    fat::block_on(fs.close(file)).unwrap();
    let dev = fat::block_on(fs.unmount()).unwrap();
    let mut hosted =
        fat::block_on(hadris_fat::async_::FatFs::mount(dev, MountOptions::new())).unwrap();
    let node = fat::block_on(hosted.lookup(hosted.root(), Name::new("CREATED.BIN"))).unwrap();
    let mut out = [0; 8];
    assert_eq!(fat::block_on(hosted.read(node, 0, &mut out)).unwrap(), 8);
    assert_eq!(&out, b"embedded");

    let mut dev = Device::<_, Rc<Cell<bool>>>::new(vec![0; 8 << 20], Rc::new(()));
    fat::block_on(hadris_fat::exfat::async_::write(
        &mut dev,
        &tree,
        &hadris_fat::exfat::ExFatOptions::new(),
    ))
    .unwrap();
    let mut token = MountToken::new();
    let mut fs = fat::block_on(hadris_fat::exfat::embedded::async_::ExFat::<_, 4>::mount(
        dev, &mut token,
    ))
    .unwrap();
    let root = fs.root();
    let file = fat::block_on(fs.open(root, "LOCAL.BIN", OpenOptions::new().read())).unwrap();
    let mut out = [0; 13];
    assert_eq!(fat::block_on(fs.read(&file, &mut out)).unwrap(), 13);
    assert_eq!(&out, b"tree contents");
}
