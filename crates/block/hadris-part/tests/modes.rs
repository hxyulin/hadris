//! The `sync` and `r#async` modes read, scan, write and open
//! the same disks the same way.

use core::future::Future;
use core::ops::ControlFlow;
use core::task::{Context, Poll};
use std::sync::Arc;
use std::task::{Wake, Waker};

use hadris_part::gpt::types;
use hadris_part::{
    Alignment, Disk, DiskLayout, Guid, MbrType, Partition, PartitionSpec, Size, TableKind,
};
use hadris_storage::{BlockSize, MemDevice};

struct ThreadWaker(std::thread::Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

fn assert_send<T: Send>(value: T) -> T {
    value
}

const B512: BlockSize = BlockSize::new(512).unwrap();

fn layouts() -> Vec<DiskLayout> {
    let guid = Guid::from_bytes([0x3C; 16]);
    let mut mbr = DiskLayout::mbr().with_alignment(Alignment::Blocks(8));
    for kind in [
        MbrType::FAT32_LBA,
        MbrType::LINUX,
        MbrType::LINUX_SWAP,
        MbrType::NTFS,
        MbrType::LINUX,
    ] {
        mbr = mbr.partition(PartitionSpec::new(kind, Size::Blocks(100)));
    }
    vec![
        mbr,
        DiskLayout::gpt(guid)
            .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::KiB(512)).with_name("EFI"))
            .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining)),
        DiskLayout::hybrid(guid)
            .partition(
                PartitionSpec::new(types::EFI_SYSTEM, Size::KiB(512))
                    .with_mirror(MbrType::EFI_SYSTEM),
            )
            .partition(PartitionSpec::new(types::BASIC_DATA, Size::Remaining)),
    ]
}

fn scan_sync(dev: &mut MemDevice<Vec<u8>>) -> (TableKind, Vec<Partition>) {
    let mut found = Vec::new();
    let kind = hadris_part::sync::scan(dev, |p| {
        found.push(p);
        ControlFlow::Continue(())
    })
    .unwrap();
    (kind, found)
}

#[test]
fn every_mode_reads_scans_writes_and_opens_alike() {
    for layout in layouts() {
        let mut sync_dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
        let disk: Disk = hadris_part::sync::create(&mut sync_dev, &layout).unwrap();
        let (kind, listed) = scan_sync(&mut sync_dev);
        assert_eq!(kind, disk.table().kind());
        assert_eq!(listed, disk.partitions().collect::<Vec<_>>());

        block_on(async {
            let mut dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
            let created = hadris_part::r#async::create(&mut dev, &layout)
                .await
                .unwrap();
            assert_eq!(created, disk);
            assert_eq!(dev.get_ref(), sync_dev.get_ref());
            assert_eq!(hadris_part::r#async::read(&mut dev).await.unwrap(), disk);
            let mut found = Vec::new();
            let scanned = hadris_part::r#async::scan(&mut dev, |p| {
                found.push(p);
                ControlFlow::Continue(())
            })
            .await
            .unwrap();
            assert_eq!((scanned, found), (kind, listed.clone()));
            let slice = hadris_part::r#async::open(&mut dev, &listed[0]).unwrap();
            assert_eq!(
                hadris_storage::r#async::BlockDevice::block_count(&slice),
                listed[0].len()
            );
        });

        let mut dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
        let write = assert_send(hadris_part::r#async::write(&mut dev, &disk));
        block_on(write).unwrap();
        assert_eq!(dev.get_ref(), sync_dev.get_ref());
        let read = assert_send(hadris_part::r#async::read(&mut dev));
        assert_eq!(block_on(read).unwrap(), disk);
        let mut count = 0;
        let scan = assert_send(hadris_part::r#async::scan(&mut dev, |_| {
            count += 1;
            ControlFlow::Continue(())
        }));
        assert_eq!(block_on(scan).unwrap(), kind);
        assert_eq!(count, listed.len());
        let slice = hadris_part::r#async::open(&mut dev, &listed[1]).unwrap();
        assert_eq!(
            hadris_storage::r#async::BlockDevice::block_count(&slice),
            listed[1].len()
        );
    }
}

#[test]
fn every_mode_falls_back_to_the_backup_gpt() {
    let layout = &layouts()[1];
    let mut dev = MemDevice::new(vec![0u8; 8192 * 512], B512);
    let disk = hadris_part::sync::create(&mut dev, layout).unwrap();
    dev.get_mut()[512 + 8] ^= 0xFF;
    let recovered = hadris_part::sync::read(&mut dev).unwrap();
    assert_eq!(
        recovered.partitions().collect::<Vec<_>>(),
        disk.partitions().collect::<Vec<_>>()
    );
    let from_async = block_on(hadris_part::r#async::read(&mut dev)).unwrap();
    assert_eq!(from_async, recovered);
}

trait Delay: Default + Unpin {
    fn ready(&mut self) -> bool;
}
impl Delay for bool {
    fn ready(&mut self) -> bool {
        core::mem::replace(self, true)
    }
}
impl Delay for std::rc::Rc<core::cell::Cell<bool>> {
    fn ready(&mut self) -> bool {
        self.replace(true)
    }
}

struct PollDevice<M, S> {
    inner: MemDevice<Vec<u8>>,
    marker: M,
    state: core::marker::PhantomData<fn() -> S>,
    cancellations: usize,
}
impl<M, S> PollDevice<M, S> {
    fn new(marker: M) -> Self {
        Self {
            inner: MemDevice::new(vec![0; 8192 * 512], B512),
            marker,
            state: core::marker::PhantomData,
            cancellations: 0,
        }
    }
}
impl<M, S> hadris_io::ErrorType for PollDevice<M, S> {
    type Error = core::convert::Infallible;
}
impl<M, S: Delay> hadris_storage::async_::BlockDevice for PollDevice<M, S> {
    type State = S;
    fn block_size(&self) -> BlockSize {
        let _ = &self.marker;
        B512
    }
    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(&self.inner)
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), hadris_io::Error<Self::Error>>> {
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
        first: hadris_storage::BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), hadris_io::Error<Self::Error>>> {
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
        _: &mut S,
        _: &mut Context<'_>,
    ) -> Poll<Result<(), hadris_io::Error<Self::Error>>> {
        Poll::Ready(Ok(()))
    }
    fn cancel(&mut self, _: &mut S) {
        self.cancellations += 1;
    }
}

fn check_local_table<D: hadris_storage::async_::BlockDevice>(
    dev: &mut D,
    layout: &DiskLayout,
    expected: &Disk,
) {
    block_on(async {
        let disk = hadris_part::async_::create(dev, layout).await.unwrap();
        assert_eq!(&disk, expected);
        assert_eq!(hadris_part::async_::read(dev).await.unwrap(), disk);
        let found = std::rc::Rc::new(core::cell::RefCell::new(Vec::new()));
        let collected = found.clone();
        hadris_part::async_::scan(dev, move |p| {
            collected.borrow_mut().push(p);
            ControlFlow::Continue(())
        })
        .await
        .unwrap();
        assert_eq!(&*found.borrow(), &disk.partitions().collect::<Vec<_>>());
        let partition = disk.partition(0).unwrap();
        let mut view = hadris_part::async_::open(dev, &partition).unwrap();
        use hadris_storage::async_::BlockDevice;
        view.write_blocks(hadris_storage::BlockIndex::new(0), &[0x5A; 512])
            .await
            .unwrap();
        let mut bytes = [0; 512];
        view.read_blocks(hadris_storage::BlockIndex::new(0), &mut bytes)
            .await
            .unwrap();
        assert_eq!(bytes, [0x5A; 512]);
    });
}

#[test]
fn local_devices_and_local_state_read_write_gpt_mbr_and_hybrid_tables() {
    for layout in layouts() {
        let mut baseline = MemDevice::new(vec![0; 8192 * 512], B512);
        let expected = hadris_part::sync::create(&mut baseline, &layout).unwrap();
        let mut local = PollDevice::<_, bool>::new(std::rc::Rc::new(()));
        check_local_table(&mut local, &layout, &expected);
        let mut local_state = PollDevice::<_, std::rc::Rc<core::cell::Cell<bool>>>::new(());
        assert_send(&local_state);
        check_local_table(&mut local_state, &layout, &expected);
    }
}

#[test]
fn borrowed_send_table_futures_run_on_scoped_threads_and_cancel_cleanly() {
    for layout in layouts() {
        let mut dev = PollDevice::<_, bool>::new(());
        {
            let mut future = std::pin::pin!(hadris_part::async_::create(&mut dev, &layout));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(future.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(dev.cancellations, 1);
        assert!(dev.inner.get_ref().iter().all(|&byte| byte == 0));
        let create = assert_send(hadris_part::async_::create(&mut dev, &layout));
        let disk =
            std::thread::scope(|scope| scope.spawn(|| block_on(create)).join().unwrap()).unwrap();
        let read = assert_send(hadris_part::async_::read(&mut dev));
        assert_eq!(
            std::thread::scope(|scope| scope.spawn(|| block_on(read)).join().unwrap()).unwrap(),
            disk
        );
        let write = assert_send(hadris_part::async_::write(&mut dev, &disk));
        std::thread::scope(|scope| scope.spawn(|| block_on(write)).join().unwrap()).unwrap();
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
    _local: std::rc::Rc<()>,
}
impl Progress for LocalState {
    fn pending(&mut self) -> &mut bool {
        &mut self.pending
    }
}
struct Device<S, L = ()> {
    inner: MemDevice<Vec<u8>>,
    _state: core::marker::PhantomData<fn() -> S>,
    _local: L,
}
impl<S, L: Default> Device<S, L> {
    fn new() -> Self {
        Self {
            inner: MemDevice::new(vec![0; 8192 * 512], B512),
            _state: core::marker::PhantomData,
            _local: L::default(),
        }
    }
}
impl<S, L> hadris_io::ErrorType for Device<S, L> {
    type Error = core::convert::Infallible;
}
impl<S: Progress, L> hadris_storage::async_::BlockDevice for Device<S, L> {
    type State = S;
    fn block_size(&self) -> BlockSize {
        B512
    }
    fn block_count(&self) -> u64 {
        8192
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), hadris_io::Error<Self::Error>>> {
        if !*state.pending() {
            *state.pending() = true;
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
        first: hadris_storage::BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), hadris_io::Error<Self::Error>>> {
        if !*state.pending() {
            *state.pending() = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(hadris_storage::sync::BlockDevice::write_blocks(
            &mut self.inner,
            first,
            buf,
        ))
    }
    fn cancel(&mut self, state: &mut S) {
        *state.pending() = false;
    }
}
async fn roundtrip<D: hadris_storage::async_::BlockDevice>(
    device: &mut D,
    layout: &DiskLayout,
) -> Disk {
    let created = hadris_part::async_::create(device, layout).await.unwrap();
    assert_eq!(hadris_part::async_::read(device).await.unwrap(), created);
    let mut found = Vec::new();
    hadris_part::async_::scan(device, |part| {
        found.push(part);
        ControlFlow::Continue(())
    })
    .await
    .unwrap();
    assert_eq!(found, created.partitions().collect::<Vec<_>>());
    created
}
#[test]
fn unified_partition_tables_support_local_devices_and_local_operation_state() {
    for layout in layouts() {
        let mut send = Device::<bool>::new();
        let expected = block_on(assert_send(roundtrip(&mut send, &layout)));
        let mut local = Device::<bool, std::rc::Rc<()>>::new();
        assert_eq!(block_on(roundtrip(&mut local, &layout)), expected);
        assert_eq!(local.inner.get_ref(), send.inner.get_ref());
        let mut local_state = Device::<LocalState>::new();
        assert_eq!(block_on(roundtrip(&mut local_state, &layout)), expected);
        assert_eq!(local_state.inner.get_ref(), send.inner.get_ref());
    }
}
#[test]
fn borrowed_send_partition_table_writes_execute_on_a_scoped_thread() {
    let layout = &layouts()[1];
    let mut device = Device::<bool>::new();
    std::thread::scope(|scope| {
        let future = roundtrip(&mut device, layout);
        let disk = scope.spawn(move || block_on(future)).join().unwrap();
        assert_eq!(disk.table().kind(), TableKind::Gpt);
    });
}
