#[path = "common/cancel.rs"]
mod cancel;
#[path = "common/fatfs.rs"]
mod common;

use hadris_fat_raw::{FatKind, Geometry};
use hadris_fs::{MountOptions, Name, OpenOptions, SetAttr};
use hadris_io::{Error, ErrorType};
use hadris_storage::{BlockIndex, BlockSize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    Write(u64, Vec<u32>),
    Flush(Vec<u32>),
}

#[derive(Debug)]
struct State {
    image: Vec<u8>,
    geo: Geometry,
    events: Vec<Event>,
    fail_write: Option<u64>,
    fail_flush: Option<usize>,
    flushes: usize,
}

impl State {
    fn flags(&self) -> Vec<u32> {
        (0..self.geo.fat_count())
            .map(|copy| {
                let kind = self.geo.kind();
                let at = (self.geo.fat_copy(copy) + kind.entry_offset(1)) as usize;
                kind.decode(1, &self.image[at..at + kind.entry_len()])
            })
            .collect()
    }
    fn assert_clean(&self, clean: bool) {
        for step in 0..self.geo.copies() {
            assert_eq!(
                self.flags()[(self.geo.active_fat() ^ step) as usize] & self.geo.kind().clean_bit()
                    != 0,
                clean
            );
        }
    }
    fn assert_ordered(&self) {
        let mut dirty_flushed = false;
        for event in &self.events {
            match event {
                Event::Flush(flags) => {
                    dirty_flushed = flags.iter().all(|f| f & self.geo.kind().clean_bit() == 0)
                }
                Event::Write(at, flags) => {
                    let marker = (0..self.geo.fat_count())
                        .any(|copy| *at == self.geo.fat_copy(copy) / 512 * 512);
                    if !marker {
                        assert!(
                            dirty_flushed,
                            "data write before dirty marker flush: {event:?}, geometry {:?}, log {:?}",
                            self.geo, self.events
                        );
                        assert!(flags.iter().all(|f| f & self.geo.kind().clean_bit() == 0));
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
struct Device(Arc<Mutex<State>>);
impl Device {
    fn new(image: Vec<u8>) -> (Self, Arc<Mutex<State>>) {
        let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
        let state = Arc::new(Mutex::new(State {
            image,
            geo,
            events: Vec::new(),
            fail_write: None,
            fail_flush: None,
            flushes: 0,
        }));
        (Self(state.clone()), state)
    }
}
impl ErrorType for Device {
    type Error = std::io::Error;
}
fn fault() -> Error<std::io::Error> {
    Error::device(std::io::Error::other("injected"), "injected")
}
impl hadris_storage::sync::BlockDevice for Device {
    fn block_size(&self) -> BlockSize {
        BlockSize::new(512).unwrap()
    }
    fn block_count(&self) -> u64 {
        self.0.lock().unwrap().image.len() as u64 / 512
    }
    fn writable(&self) -> bool {
        true
    }
    fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Error<Self::Error>> {
        let state = self.0.lock().unwrap();
        let at = first.get() as usize * 512;
        buf.copy_from_slice(&state.image[at..at + buf.len()]);
        Ok(())
    }
    fn write_blocks(&mut self, first: BlockIndex, buf: &[u8]) -> Result<(), Error<Self::Error>> {
        let mut state = self.0.lock().unwrap();
        let at = first.get() * 512;
        if state.fail_write == Some(at) {
            state.fail_write = None;
            return Err(fault());
        }
        state.image[at as usize..at as usize + buf.len()].copy_from_slice(buf);
        let flags = state.flags();
        state.events.push(Event::Write(at, flags));
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error<Self::Error>> {
        let mut state = self.0.lock().unwrap();
        state.flushes += 1;
        if state.fail_flush == Some(state.flushes) {
            state.fail_flush = None;
            return Err(fault());
        }
        let flags = state.flags();
        state.events.push(Event::Flush(flags));
        Ok(())
    }
}
impl hadris_storage::async_::BlockDevice for Device {
    type State = bool;
    fn block_size(&self) -> BlockSize {
        hadris_storage::sync::BlockDevice::block_size(self)
    }
    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(self)
    }
    fn writable(&self) -> bool {
        true
    }
    fn poll_read_blocks(
        &mut self,
        state: &mut bool,
        cx: &mut core::task::Context<'_>,
        first: BlockIndex,
        buf: &mut [u8],
    ) -> core::task::Poll<Result<(), Error<Self::Error>>> {
        if !*state {
            *state = true;
            cx.waker().wake_by_ref();
            return core::task::Poll::Pending;
        }
        core::task::Poll::Ready(hadris_storage::sync::BlockDevice::read_blocks(
            self, first, buf,
        ))
    }
    fn poll_write_blocks(
        &mut self,
        state: &mut bool,
        cx: &mut core::task::Context<'_>,
        first: BlockIndex,
        buf: &[u8],
    ) -> core::task::Poll<Result<(), Error<Self::Error>>> {
        if !*state {
            *state = true;
            cx.waker().wake_by_ref();
            return core::task::Poll::Pending;
        }
        core::task::Poll::Ready(hadris_storage::sync::BlockDevice::write_blocks(
            self, first, buf,
        ))
    }
    fn poll_flush(
        &mut self,
        state: &mut bool,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Result<(), Error<Self::Error>>> {
        if !*state {
            *state = true;
            cx.waker().wake_by_ref();
            return core::task::Poll::Pending;
        }
        core::task::Poll::Ready(hadris_storage::sync::BlockDevice::flush(self))
    }
    fn cancel(&mut self, _: &mut bool) {}
}

macro_rules! run_sync { ($($body:tt)*) => { common::block_on(hadris_macros::strip_async! { $($body)* }) }; }
macro_rules! run_async {
    ($body:expr) => {
        common::block_on($body)
    };
}

macro_rules! driver_test {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                for case in [common::CASES[1], common::CASES[2]] {
                    for dirty_at_mount in [false, true] {
                        let original = flag_image(case, dirty_at_mount, false);
                        for embedded in [false, true] {
                            let (dev, state) = Device::new(original.clone());
                            let flags = state.lock().unwrap().flags();
                            if embedded {
                                let mut token = hadris_fat::embedded::MountToken::new();
                                let mut fs = hadris_fat::embedded::$mode::Fat::<_, 4>::mount(
                                    dev, &mut token,
                                )
                                .await
                                .unwrap();
                                fs.sync().await.unwrap();
                                assert!(
                                    state
                                        .lock()
                                        .unwrap()
                                        .events
                                        .iter()
                                        .all(|e| matches!(e, Event::Flush(_)))
                                );
                                let root = fs.root();
                                let file = fs
                                    .open(root, "A.TXT", OpenOptions::new().write().create())
                                    .await
                                    .unwrap();
                                fs.write(&file, b"first").await.unwrap();
                                state.lock().unwrap().assert_clean(false);
                                fs.sync().await.unwrap();
                                state.lock().unwrap().assert_clean(!dirty_at_mount);
                                fs.write(&file, b"second").await.unwrap();
                                state.lock().unwrap().assert_clean(false);
                                fs.close(file).await.unwrap();
                                fs.unmount().await.unwrap();
                            } else {
                                let mut fs =
                                    hadris_fat::$mode::FatFs::mount(dev, MountOptions::new())
                                        .await
                                        .unwrap();
                                fs.sync().await.unwrap();
                                assert!(
                                    state
                                        .lock()
                                        .unwrap()
                                        .events
                                        .iter()
                                        .all(|e| matches!(e, Event::Flush(_)))
                                );
                                let root = fs.root();
                                let file = fs
                                    .create(root, Name::new("A.TXT"), &SetAttr::new())
                                    .await
                                    .unwrap();
                                fs.write(file, 0, b"first").await.unwrap();
                                state.lock().unwrap().assert_clean(false);
                                fs.sync().await.unwrap();
                                state.lock().unwrap().assert_clean(!dirty_at_mount);
                                fs.write(file, 0, b"second").await.unwrap();
                                state.lock().unwrap().assert_clean(false);
                                fs.sync().await.unwrap();
                            }
                            let state = state.lock().unwrap();
                            assert_eq!(state.flags(), flags);
                            state.assert_ordered();
                        }
                    }
                }
            });
        }
    };
}
driver_test!(sync, driver_clean_flags_sync, run_sync);
driver_test!(r#async, driver_clean_flags_async, run_async);

fn flag_image(case: common::Case, dirty: bool, active_only: bool) -> Vec<u8> {
    let mut image = common::blank(case);
    if active_only {
        for at in [40, 6 * 512 + 40] {
            image[at..at + 2].copy_from_slice(&0x81u16.to_le_bytes());
        }
    }
    let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
    for copy in 0..geo.fat_count() {
        let kind = geo.kind();
        let at = (geo.fat_copy(copy) + kind.entry_offset(1)) as usize;
        let mut value = kind.decode(1, &image[at..at + kind.entry_len()]);
        value &= !(kind.clean_bit() >> 1);
        if kind == FatKind::Fat32 {
            value |= 0xa000_0000;
        }
        if dirty {
            value &= !kind.clean_bit();
        }
        image[at..at + kind.entry_len()].copy_from_slice(&value.to_le_bytes()[..kind.entry_len()]);
    }
    image
}

macro_rules! raw_test {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                use hadris_fat_raw::io::{BlockBuf, $mode as io};
                for case in [common::CASES[1], common::CASES[2]] {
                    for (dirty, active_only) in [
                        (false, false),
                        (true, false),
                        (false, case.kind == FatKind::Fat32),
                    ] {
                        let (mut dev, state) = Device::new(flag_image(case, dirty, active_only));
                        let geo = state.lock().unwrap().geo;
                        let before = state.lock().unwrap().flags();
                        let mut block = BlockBuf::<[u8; 512]>::new(512).unwrap();
                        let mut fat = io::read_fat(&mut dev, &mut block, geo).await.unwrap();
                        assert_eq!(fat.was_dirty(), dirty);
                        io::set(
                            &mut dev,
                            &mut block,
                            &mut fat,
                            geo.max_cluster(),
                            case.kind.end_of_chain(),
                        )
                        .await
                        .unwrap();
                        state.lock().unwrap().assert_clean(false);
                        io::clear_dirty(&mut dev, &mut block, &mut fat)
                            .await
                            .unwrap();
                        assert_eq!(state.lock().unwrap().flags(), before);
                        assert!(!fat.needs_sync());
                    }
                    for size in [1, 3, 5, 512, 4096] {
                        let original = flag_image(case, false, false);
                        let geo = hadris_fat_raw::parse_boot(original[..512].try_into().unwrap())
                            .unwrap();
                        let mut expected = original.clone();
                        for copy in 0..geo.fat_count() {
                            let kind = geo.kind();
                            let at = (geo.fat_copy(copy) + kind.entry_offset(1)) as usize;
                            let value = kind.decode(1, &expected[at..at + kind.entry_len()])
                                & !kind.clean_bit();
                            kind.encode(1, value, &mut expected[at..at + kind.entry_len()]);
                        }
                        let mut dev = hadris_storage::MemDevice::new(
                            original.clone(),
                            BlockSize::new(size).unwrap(),
                        );
                        let mut block = BlockBuf::<[u8; 4096]>::new(size as usize).unwrap();
                        let mut fat = io::read_fat(&mut dev, &mut block, geo).await.unwrap();
                        io::begin_write(&mut dev, &mut block, &mut fat)
                            .await
                            .unwrap();
                        assert_eq!(dev.get_ref(), &expected, "{} block size {size}", case.name);
                        io::clear_dirty(&mut dev, &mut block, &mut fat)
                            .await
                            .unwrap();
                        assert_eq!(
                            dev.into_inner(),
                            original,
                            "{} block size {size}",
                            case.name
                        );
                    }
                    for failure in 0..5 {
                        let (mut dev, state) = Device::new(common::blank(case));
                        let geo = state.lock().unwrap().geo;
                        let mut block = BlockBuf::<[u8; 512]>::new(512).unwrap();
                        let mut fat = io::read_fat(&mut dev, &mut block, geo).await.unwrap();
                        if failure < 2 {
                            {
                                let mut s = state.lock().unwrap();
                                if failure == 0 {
                                    s.fail_write = Some(geo.fat_copy(1) / 512 * 512);
                                } else {
                                    s.fail_flush = Some(1);
                                }
                            }
                            assert!(
                                io::begin_write(&mut dev, &mut block, &mut fat)
                                    .await
                                    .is_err()
                            );
                        } else {
                            io::begin_write(&mut dev, &mut block, &mut fat)
                                .await
                                .unwrap();
                            {
                                let mut s = state.lock().unwrap();
                                if failure == 2 {
                                    s.fail_write = Some(geo.fat_copy(1) / 512 * 512);
                                } else {
                                    s.fail_flush =
                                        Some(s.flushes + if failure == 3 { 2 } else { 1 });
                                }
                            }
                            assert!(
                                io::clear_dirty(&mut dev, &mut block, &mut fat)
                                    .await
                                    .is_err()
                            );
                        }
                        assert!(fat.needs_sync());
                        io::begin_write(&mut dev, &mut block, &mut fat)
                            .await
                            .unwrap();
                        state.lock().unwrap().assert_clean(false);
                        assert!(matches!(
                            state.lock().unwrap().events.last(),
                            Some(Event::Flush(_))
                        ));
                        io::clear_dirty(&mut dev, &mut block, &mut fat)
                            .await
                            .unwrap();
                        state.lock().unwrap().assert_clean(true);
                        fat.preserve_dirty();
                        io::clear_dirty(&mut dev, &mut block, &mut fat)
                            .await
                            .unwrap();
                        state.lock().unwrap().assert_clean(false);
                        io::clear_dirty(&mut dev, &mut block, &mut fat)
                            .await
                            .unwrap();
                        state.lock().unwrap().assert_clean(false);
                    }
                }
            });
        }
    };
}
raw_test!(sync, raw_clean_flags_sync, run_sync);
raw_test!(r#async, raw_clean_flags_async, run_async);

#[test]
fn cancelled_marker_transitions_are_retried() {
    use hadris_fat_raw::io::{BlockBuf, r#async as io};
    let image = common::blank(common::CASES[1]);
    for clearing in [false, true] {
        let mut completed = false;
        for polls in 0..30 {
            let (mut dev, state) = Device::new(image.clone());
            let geo = state.lock().unwrap().geo;
            let mut block = BlockBuf::<[u8; 512]>::new(512).unwrap();
            let mut fat = common::block_on(io::read_fat(&mut dev, &mut block, geo)).unwrap();
            if clearing {
                common::block_on(io::begin_write(&mut dev, &mut block, &mut fat)).unwrap();
            }
            let result = if clearing {
                cancel::run_for(io::clear_dirty(&mut dev, &mut block, &mut fat), polls)
            } else {
                cancel::run_for(io::begin_write(&mut dev, &mut block, &mut fat), polls)
            };
            common::block_on(io::begin_write(&mut dev, &mut block, &mut fat)).unwrap();
            state.lock().unwrap().assert_clean(false);
            assert!(matches!(
                state.lock().unwrap().events.last(),
                Some(Event::Flush(_))
            ));
            common::block_on(io::clear_dirty(&mut dev, &mut block, &mut fat)).unwrap();
            state.lock().unwrap().assert_clean(true);
            if result.is_some() {
                completed = true;
                break;
            }
        }
        assert!(completed);
    }
}

#[test]
fn fs_info_free_count_is_ignored_when_mounted_dirty() {
    let case = common::CASES[2];
    assert_eq!(case.kind, FatKind::Fat32);
    for dirty in [false, true] {
        let mut image = flag_image(case, dirty, false);
        let geo = hadris_fat_raw::parse_boot(image[..512].try_into().unwrap()).unwrap();
        let at = geo.fs_info_sector().unwrap() as usize * 512 + 488;
        image[at..at + 4].copy_from_slice(&7u32.to_le_bytes());
        let actual = common::scan_free(&mut common::device(case, image.clone()));
        let mut fs = common::mount(case, &image);
        let free = fs.statfs().unwrap().free_blocks();
        assert_eq!(free, if dirty { u64::from(actual) } else { 7 });
    }
}
