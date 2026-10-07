use core::convert::Infallible;
use core::future::Future;
use core::marker::PhantomData;
use core::task::{Context, Poll, Waker};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use hadris_fs::{Content, MountOptions, Node, Resolve, Tree};
use hadris_io::{Error, ErrorKind, ErrorType};
use hadris_iso::async_::{IsoFs, Session, write};
use hadris_iso::{BootEntry, ElTorito, Hybrid, IsoOptions, SessionMode};
use hadris_storage::async_::{BlockDevice, Cache, SendBlockDevice};
use hadris_storage::{BlockIndex, BlockSize, MemDevice, Partition};

#[derive(Default)]
struct WriteState(bool);
#[derive(Default)]
struct LocalWriteState {
    pending: bool,
    _local: Rc<()>,
}
trait WriteStateAccess: Default + Unpin {
    fn pending(&mut self) -> &mut bool;
}
impl WriteStateAccess for WriteState {
    fn pending(&mut self) -> &mut bool {
        &mut self.0
    }
}
impl WriteStateAccess for LocalWriteState {
    fn pending(&mut self) -> &mut bool {
        &mut self.pending
    }
}
#[derive(Default)]
struct WriteAudit {
    writes: usize,
    flushes: usize,
    cancelled: usize,
    active: Option<&'static str>,
}
struct WriteDevice<S, L = ()> {
    inner: MemDevice<Vec<u8>>,
    audit: Arc<Mutex<WriteAudit>>,
    fail_write: bool,
    fail_flush: bool,
    read_only: bool,
    _state: PhantomData<fn() -> S>,
    _local: L,
}
impl<S, L> ErrorType for WriteDevice<S, L> {
    type Error = Infallible;
}
impl<S: WriteStateAccess, L> BlockDevice for WriteDevice<S, L> {
    type State = S;
    fn block_size(&self) -> BlockSize {
        hadris_storage::sync::BlockDevice::block_size(&self.inner)
    }
    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(&self.inner)
    }
    fn writable(&self) -> bool {
        !self.read_only
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut S,
        cx: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Infallible>>> {
        if self.pause(state, cx, "read") {
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
    ) -> Poll<Result<(), Error<Infallible>>> {
        if self.pause(state, cx, "write") {
            return Poll::Pending;
        }
        self.audit.lock().unwrap().writes += 1;
        if self.read_only {
            return Poll::Ready(Err(Error::new(ErrorKind::ReadOnly, "read-only fixture")));
        }
        if self.fail_write {
            return Poll::Ready(Err(Error::new(
                ErrorKind::Unsupported,
                "injected writer failure",
            )));
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
    ) -> Poll<Result<(), Error<Infallible>>> {
        if self.pause(state, cx, "flush") {
            return Poll::Pending;
        }
        self.audit.lock().unwrap().flushes += 1;
        Poll::Ready(if self.fail_flush {
            Err(Error::new(ErrorKind::Unsupported, "injected flush failure"))
        } else {
            Ok(())
        })
    }
    fn cancel(&mut self, _: &mut S) {
        let mut audit = self.audit.lock().unwrap();
        if audit.active.take().is_some() {
            audit.cancelled += 1;
        }
    }
}
impl<S: WriteStateAccess, L> WriteDevice<S, L> {
    fn pause(&mut self, state: &mut S, cx: &mut Context<'_>, kind: &'static str) -> bool {
        let mut audit = self.audit.lock().unwrap();
        if !*state.pending() {
            assert!(audit.active.is_none());
            audit.active = Some(kind);
            *state.pending() = true;
            cx.waker().wake_by_ref();
            true
        } else {
            assert_eq!(audit.active.take(), Some(kind));
            false
        }
    }
}
fn write_device<S, L: Default>(mut bytes: Vec<u8>, size: u32) -> WriteDevice<S, L> {
    bytes.resize(2 << 20, 0);
    WriteDevice {
        inner: MemDevice::new(bytes, BlockSize::new(size).unwrap()),
        audit: Arc::default(),
        fail_write: false,
        fail_flush: false,
        read_only: false,
        _state: PhantomData,
        _local: L::default(),
    }
}
fn write_block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}
fn write_require_send(_: impl Future + Send) {}
fn writer_send_witness<D: SendBlockDevice>(device: D, tree: &Tree, options: &IsoOptions) {
    write_require_send(write(device, tree, options));
}
fn session_send_witness<D: SendBlockDevice, O: SendBlockDevice>(
    session: &mut Session<D>,
    output: O,
    options: &IsoOptions,
) {
    write_require_send(session.write(options, SessionMode::Rewrite));
    write_require_send(session.export(output, options));
}
fn write_tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("hello.txt", Node::file(Content::bytes("hello world\n")))
        .unwrap();
    tree.insert("remove.txt", Node::file(Content::bytes("gone")))
        .unwrap();
    tree.insert(
        "boot/bios.img",
        Node::file(Content::bytes(vec![0x42; 4096])),
    )
    .unwrap();
    tree.insert("boot/efi.img", Node::file(Content::bytes(vec![0xEF; 4096])))
        .unwrap();
    tree
}
fn write_options() -> IsoOptions {
    IsoOptions::new()
        .with_joliet()
        .with_rock_ridge()
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("boot/bios.img"))
                .with_entry(BootEntry::uefi("boot/efi.img")),
        )
        .with_hybrid(Hybrid::gpt_hybrid_mbr())
}
fn write_initial() -> Vec<u8> {
    let mut device = MemDevice::new(vec![0; 2 << 20], BlockSize::new(2048).unwrap());
    let options = hadris_iso::IsoOptions::new()
        .with_joliet()
        .with_rock_ridge()
        .with_el_torito(
            hadris_iso::ElTorito::new()
                .with_entry(hadris_iso::BootEntry::bios("boot/bios.img"))
                .with_entry(hadris_iso::BootEntry::uefi("boot/efi.img")),
        )
        .with_hybrid(hadris_iso::Hybrid::gpt_hybrid_mbr());
    hadris_iso::sync::write(&mut device, &write_tree(), &options).unwrap();
    device.into_inner()
}
async fn verify_session_output<D: BlockDevice>(device: D) {
    let mut fs = IsoFs::mount(device, MountOptions::new()).await.unwrap();
    let node = fs.resolve(b"/added.txt", Resolve::Follow).await.unwrap();
    let mut bytes = [0; 3];
    assert_eq!(fs.read(node, 0, &mut bytes).await.unwrap(), 3);
    assert_eq!(&bytes, b"new");
    assert!(fs.resolve(b"/remove.txt", Resolve::Follow).await.is_err());
    assert!(fs.boot_catalog(&mut [0; 2048]).await.unwrap().is_some());
}
fn edit_session<D: BlockDevice>(session: &mut Session<D>) {
    session
        .tree_mut()
        .insert("added.txt", Node::file(Content::bytes("new")))
        .unwrap();
    session.tree_mut().remove("remove.txt").unwrap();
}

#[test]
fn actual_writer_send_borrowed_partition_cache_preserves_boundaries_and_boot_data() {
    let tree = write_tree();
    let options = write_options();
    let mut disk = MemDevice::new(vec![0xA5; (2 << 20) + 4096], BlockSize::new(512).unwrap());
    let partition = Partition::new(&mut disk, 2048, 2 << 20);
    writer_send_witness(Cache::new(partition, 8), &tree, &options);
    let partition = Partition::new(&mut disk, 2048, 2 << 20);
    let future = write(Cache::new(partition, 8), &tree, &options);
    std::thread::scope(|scope| {
        scope
            .spawn(move || write_block_on(future))
            .join()
            .unwrap()
            .unwrap();
    });
    assert_eq!(&disk.get_ref()[..2048], &[0xA5; 2048]);
    assert_eq!(&disk.get_ref()[(2 << 20) + 2048..], &[0xA5; 2048]);
    let mut iso = write_block_on(IsoFs::mount(
        Partition::new(&mut disk, 2048, 2 << 20),
        MountOptions::new(),
    ))
    .unwrap();
    let node = write_block_on(iso.resolve(b"/hello.txt", Resolve::Follow)).unwrap();
    let mut bytes = [0; 12];
    write_block_on(iso.read(node, 0, &mut bytes)).unwrap();
    assert_eq!(&bytes, b"hello world\n");
    assert!(
        write_block_on(iso.boot_catalog(&mut [0; 2048]))
            .unwrap()
            .is_some()
    );
}

#[test]
fn actual_writer_local_state_and_device_matches_send_output() {
    let tree = write_tree();
    let options = write_options();
    let mut send = write_device::<WriteState, ()>(Vec::new(), 512);
    let mut state_local = write_device::<LocalWriteState, ()>(Vec::new(), 512);
    fn require_send_device(_: &impl Send) {}
    require_send_device(&state_local);
    let mut local = write_device::<LocalWriteState, Rc<()>>(Vec::new(), 512);
    write_block_on(write(&mut send, &tree, &options)).unwrap();
    write_block_on(write(Cache::new(&mut local, 8), &tree, &options)).unwrap();
    write_block_on(write(&mut state_local, &tree, &options)).unwrap();
    assert_eq!(state_local.inner.get_ref(), send.inner.get_ref());
    assert_eq!(local.inner.get_ref(), send.inner.get_ref());
    assert!(local.audit.lock().unwrap().writes > 0);
    assert_eq!(local.audit.lock().unwrap().flushes, 1);
}

#[test]
fn actual_sessions_send_and_local_append_rewrite_and_export_hybrid_images() {
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let initial = write_initial();
        let mut send = write_device::<WriteState, ()>(initial.clone(), 512);
        let mut local = write_device::<LocalWriteState, Rc<()>>(initial, 512);
        let send_partition = Partition::new(&mut send, 0, 2 << 20);
        let mut send_session =
            write_block_on(Session::open(Cache::new(send_partition, 8))).unwrap();
        let local_partition = Partition::new(&mut local, 0, 2 << 20);
        let mut local_session =
            write_block_on(Session::open(Cache::new(local_partition, 8))).unwrap();
        edit_session(&mut send_session);
        edit_session(&mut local_session);
        let options = send_session.options();
        let mut unused_output = write_device::<WriteState, ()>(Vec::new(), 2048);
        session_send_witness(&mut send_session, &mut unused_output, &options);
        write_block_on(send_session.write(&options, mode)).unwrap();
        write_block_on(local_session.write(&options, mode)).unwrap();
        let export_options = write_options();
        let mut send_output = write_device::<WriteState, ()>(Vec::new(), 2048);
        let mut local_output = write_device::<LocalWriteState, Rc<()>>(Vec::new(), 2048);
        write_block_on(send_session.export(&mut send_output, &export_options)).unwrap();
        write_block_on(local_session.export(&mut local_output, &export_options)).unwrap();
        assert_eq!(send_output.inner.get_ref(), local_output.inner.get_ref());
        write_block_on(verify_session_output(&mut local_output));
        let mut reopened = write_block_on(Session::open(&mut local_output)).unwrap();
        let kept_options = reopened.options();
        write_block_on(reopened.write(&kept_options, SessionMode::Rewrite)).unwrap();
        drop(reopened);
        assert_eq!(&local_output.inner.get_ref()[512..520], b"EFI PART");
        write_block_on(verify_session_output(&mut local_output));
        drop(send_session);
        drop(local_session);
        assert_eq!(send.inner.get_ref(), local.inner.get_ref());
        write_block_on(verify_session_output(&mut local));
    }
}

#[test]
fn actual_writer_pending_write_and_flush_cancel_and_errors_propagate() {
    let tree = write_tree();
    let options = IsoOptions::new().with_joliet();
    for target in ["write", "flush"] {
        let mut device = write_device::<LocalWriteState, Rc<()>>(Vec::new(), 512);
        let audit = Arc::clone(&device.audit);
        {
            let mut future = std::pin::pin!(write(&mut device, &tree, &options));
            let mut cx = Context::from_waker(Waker::noop());
            loop {
                assert!(future.as_mut().poll(&mut cx).is_pending());
                if audit.lock().unwrap().active == Some(target) {
                    break;
                }
            }
        }
        assert_eq!(audit.lock().unwrap().cancelled, 1);
        assert!(audit.lock().unwrap().active.is_none());
        write_block_on(write(&mut device, &tree, &options)).unwrap();
    }
    for flush in [false, true] {
        let mut device = write_device::<WriteState, ()>(Vec::new(), 512);
        device.fail_write = !flush;
        device.fail_flush = flush;
        let error = write_block_on(write(&mut device, &tree, &options)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(
            error.message(),
            if flush {
                "injected flush failure"
            } else {
                "injected writer failure"
            }
        );
        assert!(device.audit.lock().unwrap().active.is_none());
    }
}

#[test]
fn actual_writer_and_session_export_grow_empty_vec_without_boxed_futures() {
    let tree = write_tree();
    let options = IsoOptions::new().with_joliet().with_rock_ridge();
    writer_send_witness(Vec::<u8>::new(), &tree, &options);
    let mut source = Vec::<u8>::new();
    write_block_on(write(&mut source, &tree, &options)).unwrap();
    assert!(!source.is_empty());
    let mut session = write_block_on(Session::open(source)).unwrap();
    edit_session(&mut session);
    let mut output = Vec::<u8>::new();
    session_send_witness(&mut session, Vec::<u8>::new(), &options);
    write_block_on(session.export(&mut output, &options)).unwrap();
    assert!(!output.is_empty());
    let mut fs = write_block_on(IsoFs::mount(output, MountOptions::new())).unwrap();
    let node = write_block_on(fs.resolve(b"/added.txt", Resolve::Follow)).unwrap();
    let mut bytes = [0; 3];
    write_block_on(fs.read(node, 0, &mut bytes)).unwrap();
    assert_eq!(&bytes, b"new");
}

#[test]
fn actual_session_write_cancellation_and_io_errors_do_not_commit_metadata() {
    for target in ["write", "flush"] {
        let mut device = write_device::<LocalWriteState, Rc<()>>(write_initial(), 512);
        let audit = Arc::clone(&device.audit);
        let mut session = write_block_on(Session::open(&mut device)).unwrap();
        edit_session(&mut session);
        let before = session.volume_blocks();
        let options = session.options();
        {
            let mut future = std::pin::pin!(session.write(&options, SessionMode::Append));
            let mut cx = Context::from_waker(Waker::noop());
            loop {
                assert!(future.as_mut().poll(&mut cx).is_pending());
                if audit.lock().unwrap().active == Some(target) {
                    break;
                }
            }
        }
        assert_eq!(audit.lock().unwrap().cancelled, 1);
        assert!(audit.lock().unwrap().active.is_none());
        assert_eq!(session.volume_blocks(), before);
        assert!(
            session
                .tree()
                .get("added.txt")
                .unwrap()
                .content()
                .unwrap()
                .stored_extents()
                .is_none()
        );
    }
    for flush in [false, true] {
        let mut device = write_device::<WriteState, ()>(write_initial(), 512);
        device.fail_write = !flush;
        device.fail_flush = flush;
        let mut session = write_block_on(Session::open(&mut device)).unwrap();
        edit_session(&mut session);
        let before = session.volume_blocks();
        let options = session.options();
        let error = write_block_on(session.write(&options, SessionMode::Append)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert_eq!(session.volume_blocks(), before);
    }
}

#[test]
fn actual_session_export_cancellation_and_io_errors_leave_source_unchanged() {
    let initial = write_initial();
    let mut source = write_device::<LocalWriteState, Rc<()>>(initial.clone(), 512);
    let mut session = write_block_on(Session::open(&mut source)).unwrap();
    edit_session(&mut session);
    let options = session.options();
    for target in ["write", "flush"] {
        let mut output = write_device::<LocalWriteState, Rc<()>>(Vec::new(), 2048);
        let audit = Arc::clone(&output.audit);
        {
            let mut future = std::pin::pin!(session.export(&mut output, &options));
            let mut cx = Context::from_waker(Waker::noop());
            loop {
                assert!(future.as_mut().poll(&mut cx).is_pending());
                if audit.lock().unwrap().active == Some(target) {
                    break;
                }
            }
        }
        assert_eq!(audit.lock().unwrap().cancelled, 1);
        assert!(audit.lock().unwrap().active.is_none());
    }
    for flush in [false, true] {
        let mut output = write_device::<WriteState, ()>(Vec::new(), 2048);
        output.fail_write = !flush;
        output.fail_flush = flush;
        let error = write_block_on(session.export(&mut output, &options)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
    }
    drop(session);
    assert_eq!(source.inner.get_ref(), &initial);
}

#[test]
fn actual_writer_undersized_preflight_and_readonly_failures_preserve_output() {
    let tree = write_tree();
    let options = IsoOptions::new().with_joliet();
    let mut small = write_device::<WriteState, ()>(Vec::new(), 512);
    small.inner = MemDevice::new(vec![0xA5; 4096], BlockSize::new(512).unwrap());
    let before = small.inner.get_ref().clone();
    let error = write_block_on(write(&mut small, &tree, &options)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NoSpace);
    assert_eq!(small.audit.lock().unwrap().writes, 0);
    assert_eq!(small.audit.lock().unwrap().cancelled, 0);
    assert!(small.audit.lock().unwrap().active.is_none());
    assert_eq!(small.inner.get_ref(), &before);

    let mut readonly = write_device::<LocalWriteState, Rc<()>>(vec![0xA5; 2 << 20], 512);
    readonly.read_only = true;
    let before = readonly.inner.get_ref().clone();
    let error = write_block_on(write(&mut readonly, &tree, &options)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ReadOnly);
    assert_eq!(readonly.audit.lock().unwrap().writes, 1);
    assert!(readonly.audit.lock().unwrap().active.is_none());
    assert_eq!(readonly.inner.get_ref(), &before);
}
