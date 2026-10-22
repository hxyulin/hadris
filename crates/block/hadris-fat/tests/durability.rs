#[path = "common/exfat.rs"]
mod exfat;

use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use hadris_fs::{Durability, MountOptions, Name};
use hadris_io::{Error, ErrorType};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize};

enum Layout {
    Fat(hadris_fat_raw::Geometry, usize),
    Exfat(exfat::Geometry, Vec<usize>),
}

impl Layout {
    fn check(&self, image: &[u8]) {
        let (mut cluster, len, size) = match self {
            Self::Fat(geo, at) => {
                if image[*at] == 0xe5 || image[*at] == 0 {
                    return;
                }
                let first = u16::from_le_bytes(image[at + 26..at + 28].try_into().unwrap()) as u32
                    | if geo.kind() == hadris_fat_raw::FatKind::Fat32 {
                        (u16::from_le_bytes(image[at + 20..at + 22].try_into().unwrap()) as u32)
                            << 16
                    } else {
                        0
                    };
                (
                    first,
                    exfat::le32(image, at + 28) as usize,
                    geo.cluster_size() as usize,
                )
            }
            Self::Exfat(geo, set) => {
                if image[set[0]] & 0x80 == 0 {
                    return;
                }
                (
                    exfat::le32(image, set[1] + 20),
                    u64::from_le_bytes(image[set[1] + 24..set[1] + 32].try_into().unwrap())
                        as usize,
                    geo.cluster,
                )
            }
        };
        let mut remaining = len;
        while remaining > 0 {
            let (at, next) = match self {
                Self::Fat(geo, _) => {
                    assert!((2..=geo.max_cluster()).contains(&cluster));
                    let entry =
                        (geo.fat_start() + geo.kind().entry_offset(cluster as u64)) as usize;
                    let next = geo.kind().decode(
                        cluster as u64,
                        &image[entry..entry + geo.kind().entry_len()],
                    );
                    assert_ne!(
                        next & geo.kind().mask(),
                        0,
                        "published file references a free cluster"
                    );
                    (
                        (geo.data_start() + (cluster - 2) as u64 * size as u64) as usize,
                        next,
                    )
                }
                Self::Exfat(geo, set) => {
                    assert!((2..geo.count + 2).contains(&cluster));
                    assert!(
                        geo.used(image, cluster),
                        "published file references a free cluster"
                    );
                    let next = if image[set[1] + 1] & 2 != 0 {
                        cluster + 1
                    } else {
                        geo.fat(image, cluster)
                    };
                    (geo.at(cluster), next)
                }
            };
            let count = remaining.min(size);
            assert!(
                image[at..at + count].iter().all(|byte| *byte == 0x5a),
                "size published before data"
            );
            remaining -= count;
            cluster = next;
        }
    }
}

struct State {
    visible: Vec<u8>,
    durable: Vec<u8>,
    layout: Layout,
    checks: usize,
    fail_flush: bool,
    enforce_order: bool,
    flushes: usize,
}

struct Buffered(Arc<Mutex<State>>);

impl ErrorType for Buffered {
    type Error = std::io::Error;
}

impl BlockDevice for Buffered {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        self.0.lock().unwrap().visible.len() as u64 / 512
    }
    fn writable(&self) -> bool {
        true
    }
    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        let state = self.0.lock().unwrap();
        let at = first.get() as usize * 512;
        buf.copy_from_slice(&state.visible[at..at + buf.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        let mut state = self.0.lock().unwrap();
        let at = first.get() as usize * 512;
        for (index, block) in buf.chunks_exact(512).enumerate() {
            let start = at + index * 512;
            // This sector may persist while every other unflushed write is lost.
            let mut crash = state.durable.clone();
            crash[start..start + 512].copy_from_slice(block);
            if state.enforce_order {
                state.layout.check(&crash);
            }
            state.checks += 1;
        }
        state.visible[at..at + buf.len()].copy_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        let mut state = self.0.lock().unwrap();
        if std::mem::take(&mut state.fail_flush) {
            return Err(Error::device(
                std::io::Error::other("injected flush failure"),
                "flush failed",
            ));
        }
        state.layout.check(&state.visible);
        state.durable = state.visible.clone();
        state.flushes += 1;
        Ok(())
    }
}

impl hadris_storage::async_::BlockDevice for Buffered {
    type State = bool;
    fn block_size(&self) -> BlockSize {
        BlockDevice::block_size(self)
    }
    fn block_count(&self) -> u64 {
        BlockDevice::block_count(self)
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        _: &mut bool,
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(BlockDevice::read_blocks(self, first, buf))
    }
    fn poll_write_blocks(
        &mut self,
        _: &mut bool,
        _: &mut Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> Poll<Result<(), Error<Self::Error>>> {
        Poll::Ready(BlockDevice::write_blocks(self, first, buf))
    }
    fn poll_flush(
        &mut self,
        state: &mut bool,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Error<Self::Error>>> {
        if !*state {
            *state = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            Poll::Ready(BlockDevice::flush(self))
        }
    }
    fn cancel(&mut self, _: &mut bool) {}
}

fn fixture(kind: usize) -> (Buffered, Arc<Mutex<State>>) {
    let (image, layout) = if kind < 3 {
        use hadris_fat::{FatKind, FatOptions};
        let (kind, size) = [
            (FatKind::Fat12, 2 << 20),
            (FatKind::Fat16, 16 << 20),
            (FatKind::Fat32, 64 << 20),
        ][kind];
        let mut dev = exfat::device(vec![0; size], 512);
        hadris_fat::sync::format(&mut dev, &FatOptions::new().with_kind(kind)).unwrap();
        let mut fs = hadris_fat::sync::FatFs::mount(dev, MountOptions::new()).unwrap();
        let root = fs.root();
        fs.create(root, Name::new("ORDER.BIN"), &hadris_fs::SetAttr::new())
            .unwrap();
        fs.sync().unwrap();
        let image = fs.into_inner().into_inner();
        let at = image
            .windows(11)
            .position(|bytes| bytes == b"ORDER   BIN")
            .unwrap();
        let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
        (image, Layout::Fat(geo, at))
    } else {
        let mut fs = exfat::small(4 << 20, 512);
        let root = fs.root();
        if kind == 4 {
            let mut aligned = false;
            for i in 0..16 {
                let name = format!("F{i}");
                let node = fs
                    .create(root, Name::new(&name), &hadris_fs::SetAttr::new())
                    .unwrap();
                let at = exfat::entry_offset(&fs, node);
                fs.forget(node, 1);
                if (at + 96) % 512 == 480 {
                    aligned = true;
                    break;
                }
            }
            assert!(aligned);
        }
        fs.create(root, Name::new("ORDER.BIN"), &hadris_fs::SetAttr::new())
            .unwrap();
        fs.sync().unwrap();
        let image = exfat::image(fs);
        let geo = exfat::Geometry::of(&image);
        let set = geo.set(&image, geo.root, "ORDER.BIN");
        if kind == 4 {
            assert_ne!(set[0] / 512, set[1] / 512);
        }
        (image, Layout::Exfat(geo, set))
    };
    let state = Arc::new(Mutex::new(State {
        durable: image.clone(),
        visible: image,
        layout,
        checks: 0,
        fail_flush: false,
        enforce_order: true,
        flushes: 0,
    }));
    (Buffered(state.clone()), state)
}

fn exercise(fs: &mut impl hadris_fs::sync::FileSystem<DeviceError = std::io::Error>) {
    let root = fs.root();
    let node = fs.lookup(root, Name::new("ORDER.BIN")).unwrap();
    for len in [1536, 512, 0, 2048] {
        if len > 512 {
            assert_eq!(fs.write(node, 0, &vec![0x5a; len]).unwrap(), len);
        } else {
            fs.truncate(node, len as u64).unwrap();
        }
        fs.sync().unwrap();
    }
    fs.forget(node, 1);
    fs.unlink(root, Name::new("ORDER.BIN")).unwrap();
    fs.sync().unwrap();
}

#[test]
fn reordered_writes_do_not_publish_unallocated_or_unwritten_data() {
    for kind in 0..3 {
        let (dev, state) = fixture(kind);
        exercise(&mut hadris_fat::sync::FatFs::mount(dev, MountOptions::new()).unwrap());
        assert!(state.lock().unwrap().checks > 10);
    }
}

#[test]
fn exfat_reordered_writes_preserve_allocations_and_data_across_entry_blocks() {
    for kind in [3, 4] {
        let (dev, state) = fixture(kind);
        exercise(&mut hadris_fat::exfat::sync::ExFatFs::mount(dev, MountOptions::new()).unwrap());
        assert!(state.lock().unwrap().checks > 10);
    }
}

async fn exercise_async(fs: &mut impl hadris_fs::async_::FileSystem<DeviceError = std::io::Error>) {
    let root = fs.root();
    let node = fs.lookup(root, Name::new("ORDER.BIN")).await.unwrap();
    for len in [1536, 512, 0, 2048] {
        if len > 512 {
            assert_eq!(fs.write(node, 0, &vec![0x5a; len]).await.unwrap(), len);
        } else {
            fs.truncate(node, len as u64).await.unwrap();
        }
        fs.sync().await.unwrap();
    }
    fs.forget(node, 1);
    fs.unlink(root, Name::new("ORDER.BIN")).await.unwrap();
    fs.sync().await.unwrap();
}

#[test]
fn async_barriers_survive_pending_flushes() {
    exfat::block_on(async {
        for kind in [0, 3, 4] {
            let (dev, state) = fixture(kind);
            if kind >= 3 {
                exercise_async(
                    &mut hadris_fat::exfat::async_::ExFatFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap(),
                )
                .await;
            } else {
                exercise_async(
                    &mut hadris_fat::async_::FatFs::mount(dev, MountOptions::new())
                        .await
                        .unwrap(),
                )
                .await;
            }
            assert!(state.lock().unwrap().checks > 10);
        }
    });
}

fn raw_exfat(
    dev: &mut Buffered,
) -> (
    hadris_fat_raw::io::BlockBuf<[u8; 512]>,
    hadris_fat_raw::exfat::io::ExFat,
) {
    use hadris_fat_raw::exfat::io::{Upcase, sync as io};
    let mut block = hadris_fat_raw::io::BlockBuf::new(512).unwrap();
    let (geo, _) = io::read_boot(dev, &mut block).unwrap();
    let vol = io::read_volume(dev, &mut block, geo, &mut Upcase::new()).unwrap();
    (block, vol)
}

#[test]
fn exfat_retries_a_failed_dirty_marker_flush() {
    use hadris_fat_raw::exfat::io::sync::begin_write;
    let (mut dev, state) = fixture(3);
    let (mut block, mut vol) = raw_exfat(&mut dev);
    state.lock().unwrap().fail_flush = true;
    assert!(begin_write(&mut dev, &mut block, &mut vol).is_err());
    assert_eq!(state.lock().unwrap().durable[106] & 2, 0);
    begin_write(&mut dev, &mut block, &mut vol).unwrap();
    assert_ne!(state.lock().unwrap().durable[106] & 2, 0);
}

#[test]
fn exfat_retries_a_cancelled_dirty_marker_flush() {
    use hadris_fat_raw::exfat::io::async_::begin_write;
    use std::future::Future;
    use std::task::Waker;
    let (mut dev, state) = fixture(3);
    let (mut block, mut vol) = raw_exfat(&mut dev);
    {
        let mut future = std::pin::pin!(begin_write(&mut dev, &mut block, &mut vol));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
    }
    assert_eq!(state.lock().unwrap().durable[106] & 2, 0);
    exfat::block_on(begin_write(&mut dev, &mut block, &mut vol)).unwrap();
    assert_ne!(state.lock().unwrap().durable[106] & 2, 0);
}

#[test]
fn embedded_fat_orders_publication_and_reclamation() {
    use hadris_fat::embedded::{MountToken, sync::Fat};
    use hadris_fs::OpenOptions;
    let (dev, state) = fixture(0);
    let mut token = MountToken::new();
    let mut fs: Fat<'_, _, 4> = Fat::mount(dev, &mut token).unwrap();
    let root = fs.root();
    let file = fs
        .open(root, "ORDER.BIN", OpenOptions::new().write())
        .unwrap();
    assert_eq!(fs.write(&file, &[0x5a; 1536]).unwrap(), 1536);
    fs.sync().unwrap();
    fs.set_len(&file, 512).unwrap();
    fs.sync().unwrap();
    fs.set_len(&file, 0).unwrap();
    fs.close(file).unwrap();
    fs.remove_file(root, "ORDER.BIN").unwrap();
    fs.sync().unwrap();
    assert!(state.lock().unwrap().checks > 10);
}

fn failed_barrier(
    fs: &mut impl hadris_fs::sync::FileSystem<DeviceError = std::io::Error>,
    state: &Arc<Mutex<State>>,
) {
    let node = fs.lookup(fs.root(), Name::new("ORDER.BIN")).unwrap();
    fs.write(node, 0, &[0x5a; 1536]).unwrap();
    fs.sync().unwrap();
    fs.write(node, 0, &[0x5a; 32]).unwrap();
    let before = state.lock().unwrap().visible.clone();
    state.lock().unwrap().fail_flush = true;
    assert_eq!(
        fs.truncate(node, 0).unwrap_err().kind(),
        hadris_fs::ErrorKind::Io
    );
    assert_eq!(state.lock().unwrap().visible, before);
    fs.sync().unwrap();
}

#[test]
fn a_failed_barrier_stops_dependent_metadata_writes() {
    for kind in [0, 3] {
        let (dev, state) = fixture(kind);
        if kind == 3 {
            failed_barrier(
                &mut hadris_fat::exfat::sync::ExFatFs::mount(dev, MountOptions::new()).unwrap(),
                &state,
            );
        } else {
            failed_barrier(
                &mut hadris_fat::sync::FatFs::mount(dev, MountOptions::new()).unwrap(),
                &state,
            );
        }
    }
}

fn explicit_syncs(
    fs: &mut impl hadris_fs::sync::FileSystem<DeviceError = std::io::Error>,
    state: &Arc<Mutex<State>>,
) -> usize {
    let node = fs.lookup(fs.root(), Name::new("ORDER.BIN")).unwrap();
    fs.write(node, 0, &[0x5a; 1536]).unwrap();
    fs.close(node).unwrap();
    let flushes = state.lock().unwrap().flushes;
    fs.fsync(node).unwrap();
    {
        let state = state.lock().unwrap();
        assert_eq!(state.visible, state.durable);
    }
    fs.truncate(node, 512).unwrap();
    state.lock().unwrap().fail_flush = true;
    assert_eq!(fs.sync().unwrap_err().kind(), hadris_fs::ErrorKind::Io);
    fs.sync().unwrap();
    {
        let state = state.lock().unwrap();
        assert_eq!(state.visible, state.durable);
    }
    fs.truncate(node, 0).unwrap();
    flushes
}

#[test]
fn buffered_reduces_barriers_but_preserves_explicit_sync_and_unmount() {
    for kind in 0..4 {
        let mut counts = Vec::new();
        for durability in [Durability::Ordered, Durability::Buffered] {
            let (dev, state) = fixture(kind);
            state.lock().unwrap().enforce_order = durability == Durability::Ordered;
            let options = MountOptions::new().with_durability(durability);
            if kind == 3 {
                let mut fs = hadris_fat::exfat::sync::ExFatFs::mount(dev, options).unwrap();
                counts.push(explicit_syncs(&mut fs, &state));
                fs.unmount().unwrap();
            } else {
                let mut fs = hadris_fat::sync::FatFs::mount(dev, options).unwrap();
                counts.push(explicit_syncs(&mut fs, &state));
                fs.unmount().unwrap();
            }
            let state = state.lock().unwrap();
            assert_eq!(state.visible, state.durable);
        }
        assert!(counts[1] < counts[0], "{kind}: {counts:?}");
        assert_eq!(
            counts[1],
            usize::from(kind != 0),
            "only dirty-marker flushes remain"
        );
    }
}

async fn explicit_syncs_async(
    fs: &mut impl hadris_fs::async_::FileSystem<DeviceError = std::io::Error>,
    state: &Arc<Mutex<State>>,
) {
    let node = fs.lookup(fs.root(), Name::new("ORDER.BIN")).await.unwrap();
    fs.write(node, 0, &[0x5a; 1536]).await.unwrap();
    fs.close(node).await.unwrap();
    assert_eq!(state.lock().unwrap().flushes, 1);
    fs.fsync(node).await.unwrap();
    {
        let state = state.lock().unwrap();
        assert_eq!(state.visible, state.durable);
    }
    fs.truncate(node, 512).await.unwrap();
    state.lock().unwrap().fail_flush = true;
    assert_eq!(
        fs.sync().await.unwrap_err().kind(),
        hadris_fs::ErrorKind::Io
    );
    fs.sync().await.unwrap();
    {
        let state = state.lock().unwrap();
        assert_eq!(state.visible, state.durable);
    }
    fs.truncate(node, 0).await.unwrap();
}

#[test]
fn buffered_async_honors_pending_and_failed_explicit_flushes() {
    exfat::block_on(async {
        for kind in [1, 3] {
            let (dev, state) = fixture(kind);
            state.lock().unwrap().enforce_order = false;
            let options = MountOptions::new().with_durability(Durability::Buffered);
            if kind == 3 {
                let mut fs = hadris_fat::exfat::async_::ExFatFs::mount(dev, options)
                    .await
                    .unwrap();
                explicit_syncs_async(&mut fs, &state).await;
                fs.unmount().await.unwrap();
            } else {
                let mut fs = hadris_fat::async_::FatFs::mount(dev, options)
                    .await
                    .unwrap();
                explicit_syncs_async(&mut fs, &state).await;
                fs.unmount().await.unwrap();
            }
            let state = state.lock().unwrap();
            assert_eq!(state.visible, state.durable);
        }
    });
}

#[test]
fn buffered_embedded_preserves_explicit_flush_and_unmount() {
    use hadris_fat::embedded::{MountToken, Options, sync::Fat};
    use hadris_fs::OpenOptions;
    let (dev, state) = fixture(1);
    state.lock().unwrap().enforce_order = false;
    let mut token = MountToken::new();
    let mut fs: Fat<'_, _, 4> = Fat::mount_with(
        dev,
        &mut token,
        Options::new().with_durability(Durability::Buffered),
    )
    .unwrap();
    let file = fs
        .open(fs.root(), "ORDER.BIN", OpenOptions::new().write())
        .unwrap();
    fs.write(&file, &[0x5a; 1536]).unwrap();
    assert_eq!(state.lock().unwrap().flushes, 1);
    fs.flush(&file).unwrap();
    {
        let state = state.lock().unwrap();
        assert_eq!(state.visible, state.durable);
    }
    fs.set_len(&file, 0).unwrap();
    fs.unmount().unwrap();
    let state = state.lock().unwrap();
    assert_eq!(state.visible, state.durable);
}

#[test]
fn durability_policy_does_not_write_on_read_only_mounts() {
    fn reject_write(fs: &mut impl hadris_fs::sync::FileSystem<DeviceError = std::io::Error>) {
        assert_eq!(
            fs.create(fs.root(), Name::new("NEW"), &hadris_fs::SetAttr::new())
                .unwrap_err()
                .kind(),
            hadris_fs::ErrorKind::ReadOnly
        );
        fs.sync().unwrap();
    }
    for durability in [Durability::Ordered, Durability::Buffered] {
        for kind in [1, 3] {
            let (dev, state) = fixture(kind);
            let options = MountOptions::new().read_only().with_durability(durability);
            if kind == 3 {
                let mut fs = hadris_fat::exfat::sync::ExFatFs::mount(dev, options).unwrap();
                reject_write(&mut fs);
                fs.unmount().unwrap();
            } else {
                let mut fs = hadris_fat::sync::FatFs::mount(dev, options).unwrap();
                reject_write(&mut fs);
                fs.unmount().unwrap();
            }
            let state = state.lock().unwrap();
            assert_eq!(state.flushes, 0);
            assert_eq!(state.checks, 0);
            assert_eq!(state.visible, state.durable);
        }
    }
}
