//! `ExFatFs` operations dropped part way through in the `async` mode.

#[path = "common/exfat.rs"]
mod common;

#[path = "common/cancel.rs"]
mod cancel;

use hadris_fat::exfat::async_::{ExFatFs, check};
use hadris_fs::ErrorKind;
use hadris_fs::MountOptions;

#[test]
fn dropped_operations_leave_whole_entry_sets_and_no_lost_clusters() {
    for (size, cluster) in [(8 << 20, 512), (16 << 20, 4096)] {
        let image = common::image(common::small(size, cluster));
        let dev = cancel::YieldDev(common::device(image, 512));
        let options = MountOptions::new();
        let mut fs = cancel::run_for(ExFatFs::mount(dev, options), usize::MAX)
            .unwrap()
            .unwrap();
        let mut rng = cancel::Rng(11);
        let mut dropped = 0;
        for i in 0..400 {
            let kind = rng.below(5);
            let polls = rng.below(40) as usize + 1;
            match cancel::run_for(cancel::step(&mut fs, kind, i), polls) {
                None => dropped += 1,
                Some(Err(kind @ (ErrorKind::Corrupt | ErrorKind::Io))) => {
                    panic!("{cluster}: step {i}: {kind:?}")
                }
                Some(_) => {}
            }
        }
        assert!(dropped > 50, "{cluster}: {dropped} dropped");
        cancel::run_for(fs.sync(), usize::MAX).unwrap().unwrap();
        let mut dev = fs.into_inner();
        let mut findings = Vec::new();
        let report = cancel::run_for(
            check(&mut dev, &mut [0u8; 8192], |finding| {
                findings.push(finding.to_string())
            }),
            usize::MAX,
        )
        .unwrap()
        .unwrap();
        assert!(report.is_clean(), "{cluster}: {findings:?}");
        common::fsck(&dev.0.into_inner(), "dropped operations");
    }
}

#[test]
fn hinted_append_recovers_when_dropped_at_every_await() {
    use hadris_fs::Name;
    for cluster in [512, 4096] {
        let original = common::payload(131_072, 9);
        let mut fs = common::small(4 << 20, cluster);
        let root = fs.root();
        common::write(&mut fs, root, "hint.bin", &original);
        fs.sync().unwrap();
        let base = common::image(fs);
        for contiguous in [false, true] {
            let mut image = base.clone();
            if contiguous {
                let geo = common::Geometry::of(&image);
                let set = geo.set(&image, geo.root, "hint.bin");
                geo.unchain(&mut image, &set);
            }
            let mut completed = false;
            for polls in 0..600 {
                let dev = cancel::YieldDev(common::device(image.clone(), 512));
                let mut fs = common::block_on(ExFatFs::mount(dev, MountOptions::new())).unwrap();
                let node = common::block_on(fs.lookup(fs.root(), Name::new("hint.bin"))).unwrap();
                common::block_on(fs.read(node, original.len() as u64 - 1, &mut [0])).unwrap();
                let result = cancel::run_for(
                    fs.write(node, original.len() as u64 + 33, &[0x77; 4096]),
                    polls,
                );
                common::block_on(fs.sync()).unwrap();
                let finished = result.is_some();
                if let Some(result) = result {
                    assert_eq!(result.unwrap(), 4096);
                }
                let mut dev = fs.into_inner().0;
                assert!(
                    common::check_dev(&mut dev, 4096).1.is_empty(),
                    "{cluster} {contiguous} {polls}"
                );
                let actual = common::read(&mut common::mount(&dev.into_inner()), "hint.bin");
                assert_eq!(&actual[..original.len()], &original);
                if finished {
                    assert_eq!(actual.len(), original.len() + 33 + 4096);
                    assert!(
                        actual[original.len()..original.len() + 33]
                            .iter()
                            .all(|&b| b == 0)
                    );
                    assert!(actual[original.len() + 33..].iter().all(|&b| b == 0x77));
                    completed = true;
                    break;
                } else {
                    assert_eq!(actual, original);
                }
            }
            assert!(completed);
        }
    }
}

async fn shrink_op<D: hadris_storage::async_::BlockDevice>(
    fs: &mut ExFatFs<D>,
    op: u32,
) -> Result<(), ErrorKind> {
    use hadris_fs::{Name, RenameMode};
    let root = fs.root();
    let result = match op {
        0 => fs.unlink(root, Name::new("victim.bin")).await,
        1 => {
            fs.rename(
                root,
                Name::new("source.bin"),
                root,
                Name::new("target.bin"),
                RenameMode::Replace,
            )
            .await
        }
        _ => {
            let (text, len) =
                [("victim.bin", 513), ("victim.bin", 0), ("chained.bin", 513)][op as usize - 2];
            let node = fs.lookup(root, Name::new(text)).await.unwrap();
            let result = fs.truncate(node, len).await;
            fs.forget(node, 1);
            result
        }
    };
    result.map_err(|err| err.kind())
}

/// Unlinking or replacing a contiguous file, and shrinking a contiguous or
/// chained one, dropped at any await, leave no lost or cross-linked
/// clusters once the next operation has recovered.
#[test]
fn contiguous_frees_and_shrinks_recover_when_dropped_at_every_await() {
    let image = common::shrink_image();
    for op in 0..common::SHRINK_OPS {
        let mut completed = false;
        for polls in 0..200 {
            let what = format!("op {op} polls {polls}");
            let dev = cancel::YieldDev(common::device(image.clone(), 512));
            let mut fs = common::block_on(ExFatFs::mount(dev, MountOptions::new())).unwrap();
            let result = cancel::run_for(shrink_op(&mut fs, op), polls);
            common::block_on(fs.sync()).unwrap();
            let after = fs.into_inner().0.into_inner();
            if let Some(result) = &result {
                assert_eq!(result, &Ok(()), "{what}");
            }
            common::check_shrink_op(&after, op, result.is_some(), &what);
            if result.is_some() {
                completed = true;
                break;
            }
        }
        assert!(completed, "op {op} never completed");
    }
}

fn bad_cluster_image() -> (Vec<u8>, Vec<u32>) {
    let mut image = common::image(common::small(8 << 20, 512));
    let geo = common::Geometry::of(&image);
    let entry = geo.root_entries(&image, 0x81)[0];
    let first = common::le32(&image, entry + 20);
    let bitmap: Vec<u8> = geo
        .chain(&image, first)
        .into_iter()
        .flat_map(|cluster| {
            image[geo.at(cluster)..geo.at(cluster) + geo.cluster]
                .iter()
                .copied()
        })
        .collect();
    let free: Vec<u32> = (2..geo.count + 2)
        .filter(|&cluster| bitmap[(cluster as usize - 2) / 8] & (1 << ((cluster - 2) % 8)) == 0)
        .collect();
    let usable = vec![free[0], free[free.len() / 2], *free.last().unwrap()];
    for cluster in free {
        if !usable.contains(&cluster) {
            geo.set_fat(&mut image, cluster, 0xffff_fff7);
        }
    }
    (image, usable)
}

macro_rules! bad_cluster_allocations {
    ($mode:ident, $name:ident, $run:ident) => {
        #[test]
        fn $name() {
            $run!(async {
                use hadris_fat_raw::exfat::io::{Upcase, $mode as rawio};
                use hadris_fat_raw::io::{BlockBuf, Held};
                for count in [1, 3, 4] {
                    let (image, usable) = bad_cluster_image();
                    let original = image.clone();
                    let mut dev = common::device(image, 512);
                    let mut block = BlockBuf::<[u8; 512]>::new(512).unwrap();
                    let (geo, _) = rawio::read_boot(&mut dev, &mut block).await.unwrap();
                    let mut vol = rawio::read_volume(&mut dev, &mut block, geo, &mut Upcase::new())
                        .await
                        .unwrap();
                    let mut held = Held::NONE;
                    let result = if count == 1 {
                        rawio::allocate(&mut dev, &mut block, &mut vol, Some(&mut held)).await
                    } else {
                        rawio::allocate_run(&mut dev, &mut block, &mut vol, Some(&mut held), count)
                            .await
                    };
                    if count == 4 {
                        assert_eq!(result.unwrap_err().kind(), ErrorKind::NoSpace);
                        assert_eq!(held, Held::NONE);
                        assert_eq!(dev.into_inner(), original);
                        continue;
                    }
                    let head = result.unwrap();
                    assert_eq!(head, usable[0]);
                    assert_eq!(held.head(), head);
                    let mut actual = Vec::new();
                    let mut next = Some(head);
                    while let Some(cluster) = next {
                        actual.push(cluster);
                        next = rawio::next(&mut dev, &mut block, &geo, cluster)
                            .await
                            .unwrap();
                    }
                    assert_eq!(actual, usable[..count as usize]);
                    let bytes = dev.into_inner();
                    let raw = common::Geometry::of(&bytes);
                    for cluster in 2..raw.count + 2 {
                        if raw.fat(&original, cluster) == 0xffff_fff7 {
                            assert_eq!(
                                raw.fat(&bytes, cluster),
                                0xffff_fff7,
                                "bad cluster {cluster}"
                            );
                        }
                    }
                }
            });
        }
    };
}

macro_rules! sync_allocation {
    ($($body:tt)*) => { common::block_on(hadris_macros::strip_async! { $($body)* }) };
}
macro_rules! async_allocation {
    ($body:expr) => {
        common::block_on($body)
    };
}
bad_cluster_allocations!(sync, bad_clusters_are_not_allocated_sync, sync_allocation);
bad_cluster_allocations!(
    async_,
    bad_clusters_are_not_allocated_async,
    async_allocation
);

#[test]
fn allocated_contiguous_overlap_is_rejected_even_after_cancelled_validation() {
    use hadris_fs::{Name, RenameMode};
    let mut fs = common::small(8 << 20, 512);
    let root = fs.root();
    for text in ["c", "a", "b"] {
        let node = common::write(&mut fs, root, text, &common::payload(1500, 2));
        fs.forget(node, 1);
    }
    fs.sync().unwrap();
    let mut image = common::image(fs);
    let geo = common::Geometry::of(&image);
    for text in ["a", "b"] {
        let set = geo.set(&image, geo.root, text);
        geo.unchain(&mut image, &set);
    }
    let set = geo.set(&image, geo.root, "a");
    image[set[1] + 24..set[1] + 32].copy_from_slice(&(6 * 512u64).to_le_bytes());
    geo.reseal(&mut image, &set);
    for operation in 0..5 {
        for polls in [0, 1, 10] {
            let dev = cancel::YieldDev(common::device(image.clone(), 512));
            let mut fs = common::block_on(ExFatFs::mount(dev, MountOptions::new())).unwrap();
            let root = fs.root();
            let node = common::block_on(fs.lookup(root, Name::new("a"))).unwrap();
            assert!(cancel::run_for(fs.truncate(node, 0), polls).is_none());
            let result = common::block_on(async {
                match operation {
                    0 => fs.truncate(node, 0).await,
                    1 => fs.unlink(root, Name::new("a")).await,
                    2 => {
                        fs.rename(
                            root,
                            Name::new("c"),
                            root,
                            Name::new("a"),
                            RenameMode::Replace,
                        )
                        .await
                    }
                    3 => fs.write(node, 0, b"overwrite").await.map(|_| ()),
                    _ => fs.write(node, 6 * 512, b"grow").await.map(|_| ()),
                }
            });
            assert_eq!(result.unwrap_err().kind(), ErrorKind::Corrupt);
            common::block_on(fs.sync()).unwrap();
            assert_eq!(fs.into_inner().0.into_inner(), image);
        }
    }
}
