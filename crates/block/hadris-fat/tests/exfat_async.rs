//! `ExFatFs` operations dropped part way through in the `async` mode.

#[path = "common/exfat.rs"]
mod common;
use hadris_fs::r#async::FileSystem;

#[path = "common/cancel.rs"]
mod cancel;

use hadris_fat::exfat::r#async::{ExFatFs, check};
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
    use hadris_fs::sync::FileSystem as _;
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
