use hadris_fat_raw::io::{BlockBuf, Fat, sync as io};
use hadris_fat_raw::layout::{self, BootFields, Request};
use hadris_fat_raw::{FatKind, Geometry};
use hadris_fs::DateTime;
use hadris_io::{Error, ErrorKind, ErrorType};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

struct Device {
    inner: MemDevice<Vec<u8>>,
    budget: Option<usize>,
}

impl ErrorType for Device {
    type Error = core::convert::Infallible;
}

impl hadris_storage::sync::BlockDevice for Device {
    fn block_size(&self) -> BlockSize {
        hadris_storage::sync::BlockDevice::block_size(&self.inner)
    }

    fn block_count(&self) -> u64 {
        hadris_storage::sync::BlockDevice::block_count(&self.inner)
    }

    fn writable(&self) -> bool {
        true
    }

    fn read_blocks(&mut self, first: BlockIndex, out: &mut [u8]) -> Result<(), Error<Self::Error>> {
        hadris_storage::sync::BlockDevice::read_blocks(&mut self.inner, first, out)
    }

    fn write_blocks(&mut self, first: BlockIndex, data: &[u8]) -> Result<(), Error<Self::Error>> {
        if let Some(left) = self.budget.as_mut() {
            if *left == 0 {
                self.budget = None;
                return Err(Error::new(ErrorKind::Io, "injected write failure"));
            }
            *left -= 1;
        }
        hadris_storage::sync::BlockDevice::write_blocks(&mut self.inner, first, data)
    }
}

fn fixture(sector: u32, size: u32, copies: u8, parity: u32) -> (Vec<u8>, Geometry, Fat, u32, u32) {
    let image = vec![0; 2 * 1024 * 1024];
    let mut dev = MemDevice::new(image, BlockSize::new(size).unwrap());
    let mut block = BlockBuf::<[u8; 4096]>::new(size as usize).unwrap();
    let plan = layout::plan(
        &Request::new(sector, 2 * 1024 * 1024 / sector as u64)
            .with_kind(FatKind::Fat12)
            .with_fat_count(copies),
    )
    .unwrap();
    let geo = io::mkfs(
        &mut dev,
        &mut block,
        &plan,
        &BootFields::new(),
        DateTime::from_unix_seconds(1_700_000_000).unwrap(),
    )
    .unwrap();
    let mut fat = io::read_fat(&mut dev, &mut block, geo).unwrap();
    io::count_free(&mut dev, &mut block, &mut fat).unwrap();
    let boundary = (3..geo.max_cluster()).find(|&c| {
        (geo.fat_copy(0) + c as u64 * 3 / 2) % size as u64 == size as u64 - 1 && c % 2 == parity
    });
    let Some(boundary) = boundary else {
        return (dev.into_inner(), geo, fat, 0, 0);
    };
    let victim = if boundary % 2 == 0 { 255 } else { 15 };
    assert!(![boundary - 1, boundary, boundary + 1].contains(&victim));
    for c in [victim, boundary - 1, boundary + 1] {
        io::set(&mut dev, &mut block, &mut fat, c, 0xFFF).unwrap();
    }
    (dev.into_inner(), geo, fat, boundary, victim)
}

fn stored(image: &[u8], geo: Geometry, copy: u8, cluster: u32) -> u32 {
    let at = (geo.fat_copy(copy) + cluster as u64 * 3 / 2) as usize;
    let pair = u16::from_le_bytes([image[at], image[at + 1]]) as u32;
    if cluster % 2 == 0 {
        pair & 0xFFF
    } else {
        pair >> 4
    }
}

fn check(
    dev: Device,
    geo: Geometry,
    fat: Fat,
    boundary: u32,
    victim: u32,
    expected: u32,
    free: u32,
) {
    assert_eq!(fat.free_clusters(), Some(free));
    assert_eq!(fat.unmirrored(), None);
    let image = dev.inner.into_inner();
    for copy in 0..geo.fat_count() {
        for c in 2..=geo.max_cluster() {
            let want = if c == boundary {
                expected
            } else if [victim, boundary - 1, boundary + 1].contains(&c) {
                0xFFF
            } else {
                0
            };
            assert_eq!(
                stored(&image, geo, copy, c),
                want,
                "copy {copy}, cluster {c}"
            );
        }
    }
}

#[test]
fn split_entries_repair_active_before_mirroring_and_adjust_free_once() {
    let mut covered = [false; 2];
    for sector in [512, 4096] {
        for size in [512, 4096] {
            for copies in [1, 2] {
                for parity in [0, 1] {
                    let (image, geo, fat, boundary, victim) = fixture(sector, size, copies, parity);
                    if boundary == 0 {
                        continue;
                    }
                    covered[parity as usize] = true;
                    let original_free = fat.free_clusters().unwrap();
                    for value in [0xFFF, victim, 0] {
                        for budget in 0..=4 * copies as usize {
                            let mut dev = Device {
                                inner: MemDevice::new(image.clone(), BlockSize::new(size).unwrap()),
                                budget: None,
                            };
                            let mut block = BlockBuf::<[u8; 4096]>::new(size as usize).unwrap();
                            let mut fat = fat;
                            if value == 0 {
                                io::set(&mut dev, &mut block, &mut fat, boundary, 0xFFF).unwrap();
                            }
                            dev.budget = Some(budget);
                            let result = io::set(&mut dev, &mut block, &mut fat, boundary, value);
                            dev.budget = None;
                            let pending = fat.unmirrored().is_some();
                            io::mirror(&mut dev, &mut block, &mut fat).unwrap();
                            let expected = if result.is_ok() || pending { value } else { 0 };
                            check(
                                dev,
                                geo,
                                fat,
                                boundary,
                                victim,
                                expected,
                                original_free - u32::from(expected != 0),
                            );
                        }
                    }
                }
            }
        }
    }
    assert_eq!(covered, [true, true]);
}

#[cfg(feature = "async")]
mod asynchronous {
    use super::*;
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};
    use hadris_fat_raw::io::r#async as aio;

    async fn yield_once() {
        let mut pending = true;
        core::future::poll_fn(|cx| {
            if pending {
                pending = false;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await
    }

    impl hadris_storage::r#async::BlockDevice for Device {
        fn block_size(&self) -> BlockSize {
            hadris_storage::sync::BlockDevice::block_size(self)
        }
        fn block_count(&self) -> u64 {
            hadris_storage::sync::BlockDevice::block_count(self)
        }
        fn writable(&self) -> bool {
            true
        }
        async fn read_blocks(
            &mut self,
            first: BlockIndex,
            out: &mut [u8],
        ) -> Result<(), Error<Self::Error>> {
            yield_once().await;
            hadris_storage::sync::BlockDevice::read_blocks(self, first, out)
        }
        async fn write_blocks(
            &mut self,
            first: BlockIndex,
            data: &[u8],
        ) -> Result<(), Error<Self::Error>> {
            yield_once().await;
            hadris_storage::sync::BlockDevice::write_blocks(self, first, data)
        }
    }

    fn run<F: Future>(future: F, budget: usize) -> Option<F::Output> {
        let mut cx = Context::from_waker(Waker::noop());
        let mut future = pin!(future);
        for _ in 0..budget {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return Some(value);
            }
        }
        None
    }

    #[test]
    fn batched_allocations_recover_after_every_cancelled_await() {
        use hadris_fat_raw::io::Held;
        for sector in [512, 4096] {
            for size in [512, 4096] {
                for copies in [1, 2] {
                    for parity in [0, 1] {
                        let (image, geo, fat, boundary, victim) =
                            fixture(sector, size, copies, parity);
                        if boundary == 0 {
                            continue;
                        }
                        let free = fat.free_clusters().unwrap();
                        let mut complete = false;
                        for budget in 0..320 {
                            let mut dev = Device {
                                inner: MemDevice::new(image.clone(), BlockSize::new(size).unwrap()),
                                budget: None,
                            };
                            let mut block = BlockBuf::<[u8; 4096]>::new(size as usize).unwrap();
                            let mut fat = fat;
                            let mut held = Held::NONE;
                            let result = run(
                                aio::allocate_run(
                                    &mut dev,
                                    &mut block,
                                    &mut fat,
                                    Some(&mut held),
                                    free - 1,
                                ),
                                budget,
                            );
                            run(aio::mirror(&mut dev, &mut block, &mut fat), usize::MAX)
                                .unwrap()
                                .unwrap();
                            reclaim(&mut dev, &mut block, &mut fat, held.extra());
                            reclaim(&mut dev, &mut block, &mut fat, held.head());
                            check(dev, geo, fat, boundary, victim, 0, free);
                            if let Some(result) = result {
                                result.unwrap();
                                complete = true;
                                break;
                            }
                        }
                        assert!(complete);
                    }
                }
            }
        }
    }

    #[test]
    fn split_entry_and_repair_can_be_cancelled_at_every_await() {
        let mut covered = [false; 2];
        for sector in [512, 4096] {
            for size in [512, 4096] {
                for copies in [1, 2] {
                    for parity in [0, 1] {
                        let (image, geo, fat, boundary, victim) =
                            fixture(sector, size, copies, parity);
                        if boundary == 0 {
                            continue;
                        }
                        covered[parity as usize] = true;
                        let free = fat.free_clusters().unwrap();
                        for value in [0xFFF, victim, 0] {
                            for budget in 0..24 {
                                for repair_budget in 0..24 {
                                    let mut dev = Device {
                                        inner: MemDevice::new(
                                            image.clone(),
                                            BlockSize::new(size).unwrap(),
                                        ),
                                        budget: None,
                                    };
                                    let mut block =
                                        BlockBuf::<[u8; 4096]>::new(size as usize).unwrap();
                                    let mut fat = fat;
                                    if value == 0 {
                                        io::set(&mut dev, &mut block, &mut fat, boundary, 0xFFF)
                                            .unwrap();
                                    }
                                    let _ = run(
                                        aio::set(&mut dev, &mut block, &mut fat, boundary, value),
                                        budget,
                                    );
                                    let _ = run(
                                        aio::mirror(&mut dev, &mut block, &mut fat),
                                        repair_budget,
                                    );
                                    run(aio::mirror(&mut dev, &mut block, &mut fat), usize::MAX)
                                        .unwrap()
                                        .unwrap();
                                    let expected = stored(dev.inner.get_ref(), geo, 0, boundary);
                                    let initial = if value == 0 { 0xFFF } else { 0 };
                                    assert!(
                                        [initial, value].contains(&expected),
                                        "partial link {expected}"
                                    );
                                    check(
                                        dev,
                                        geo,
                                        fat,
                                        boundary,
                                        victim,
                                        expected,
                                        free - u32::from(expected != 0),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(covered, [true, true]);
    }
}

fn reclaim(dev: &mut Device, block: &mut BlockBuf, fat: &mut Fat, first: u32) {
    let mut cluster = first;
    for _ in 0..fat.geometry().max_cluster() {
        if cluster == 0 {
            return;
        }
        let value = io::get(dev, block, fat, cluster).unwrap();
        if value == 0 {
            return;
        }
        let next = FatKind::Fat12
            .next(value, fat.geometry().max_cluster())
            .unwrap();
        io::set(dev, block, fat, cluster, 0).unwrap();
        cluster = next.unwrap_or(0);
    }
    panic!("cyclic held allocation");
}

#[test]
fn batched_allocations_recover_from_each_failed_write_without_touching_live_entries() {
    use hadris_fat_raw::io::Held;
    for sector in [512, 4096] {
        for size in [512, 4096] {
            for copies in [1, 2] {
                for parity in [0, 1] {
                    let (image, geo, fat, boundary, victim) = fixture(sector, size, copies, parity);
                    if boundary == 0 {
                        continue;
                    }
                    let free = fat.free_clusters().unwrap();
                    let mut complete = false;
                    for budget in 0..100 {
                        let mut dev = Device {
                            inner: MemDevice::new(image.clone(), BlockSize::new(size).unwrap()),
                            budget: Some(budget),
                        };
                        let mut block = BlockBuf::<[u8; 4096]>::new(size as usize).unwrap();
                        let mut fat = fat;
                        let mut held = Held::NONE;
                        let result = io::allocate_run(
                            &mut dev,
                            &mut block,
                            &mut fat,
                            Some(&mut held),
                            free - 1,
                        );
                        dev.budget = None;
                        io::mirror(&mut dev, &mut block, &mut fat).unwrap();
                        reclaim(&mut dev, &mut block, &mut fat, held.extra());
                        reclaim(&mut dev, &mut block, &mut fat, held.head());
                        check(dev, geo, fat, boundary, victim, 0, free);
                        if result.is_ok() {
                            complete = true;
                            break;
                        }
                        assert_eq!(result.unwrap_err().kind(), ErrorKind::Io);
                    }
                    assert!(complete);
                }
            }
        }
    }
}

#[test]
fn fragmented_batches_preserve_neighbors_wrap_and_reject_full_requests_before_writing() {
    use hadris_fat_raw::io::Held;
    for size in [512, 4096] {
        for copies in [1, 2] {
            let (image, geo, mut fat, _, _) = fixture(512, size, copies, 1);
            let mut dev = Device {
                inner: MemDevice::new(image, BlockSize::new(size).unwrap()),
                budget: None,
            };
            let mut block = BlockBuf::<[u8; 4096]>::new(size as usize).unwrap();
            let take = fat.free_clusters().unwrap() - 3;
            io::allocate_run(&mut dev, &mut block, &mut fat, None, take).unwrap();
            for c in 2..=geo.max_cluster() {
                if io::get(&mut dev, &mut block, &fat, c).unwrap() != 0 {
                    io::set(&mut dev, &mut block, &mut fat, c, 0xFFF).unwrap();
                }
            }
            for c in [2, 4, 6, 8, 10, 12, 16, 18] {
                io::set(&mut dev, &mut block, &mut fat, c, 0).unwrap();
            }
            io::set(&mut dev, &mut block, &mut fat, geo.max_cluster() - 1, 0xFFF).unwrap();
            let before = dev.inner.get_ref().clone();
            let free = fat.free_clusters().unwrap();
            let mut held = Held::NONE;
            assert_eq!(
                io::allocate_run(&mut dev, &mut block, &mut fat, Some(&mut held), free + 1)
                    .unwrap_err()
                    .kind(),
                ErrorKind::NoSpace
            );
            assert_eq!(held, Held::NONE);
            assert_eq!(dev.inner.get_ref(), &before);
            let mut order = (fat.next_free()..=geo.max_cluster())
                .chain(2..fat.next_free())
                .filter(|&c| stored(&before, geo, 0, c) == 0);
            let expected: Vec<_> = order.by_ref().take(8).collect();
            assert_eq!(expected.len(), 8);
            assert!(
                expected.windows(2).any(|pair| pair[1] < pair[0]),
                "allocation must wrap"
            );
            let first =
                io::allocate_run(&mut dev, &mut block, &mut fat, Some(&mut held), 8).unwrap();
            assert_eq!(first, expected[0]);
            assert_eq!(held.head(), first);
            assert_eq!(held.extra(), 0);
            assert_eq!(fat.free_clusters(), Some(free - 8));
            for copy in 0..copies {
                for c in 2..=geo.max_cluster() {
                    let want = expected
                        .iter()
                        .position(|&allocated| allocated == c)
                        .map_or_else(
                            || stored(&before, geo, copy, c),
                            |at| expected.get(at + 1).copied().unwrap_or(0xFF8),
                        );
                    assert_eq!(
                        stored(dev.inner.get_ref(), geo, copy, c),
                        want,
                        "copy {copy}, cluster {c}"
                    );
                }
            }
            io::allocate_run(&mut dev, &mut block, &mut fat, None, free - 8).unwrap();
            let mut held = Held::NONE;
            assert_eq!(
                io::allocate_run(&mut dev, &mut block, &mut fat, Some(&mut held), 2)
                    .unwrap_err()
                    .kind(),
                ErrorKind::NoSpace
            );
            assert_eq!(held, Held::NONE);
            assert_eq!(fat.free_clusters(), Some(0));
        }
    }
}
