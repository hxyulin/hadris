//! `ExFatFs` operations dropped part way through in the `async` mode.

#[path = "common/exfat.rs"]
mod common;
use hadris_fs::r#async::FileSystem;

#[path = "common/cancel.rs"]
mod cancel;

use hadris_fat::exfat::MountOptions;
use hadris_fat::exfat::r#async::{ExFatFs, check};
use hadris_fs::{ErrorKind, HeapTable};

#[test]
fn dropped_operations_leave_whole_entry_sets_and_no_lost_clusters() {
    for (size, cluster) in [(8 << 20, 512), (16 << 20, 4096)] {
        let image = common::image(common::small(size, cluster));
        let dev = cancel::YieldDev(common::device(image, 512));
        let options = MountOptions::new().with_table(HeapTable::new());
        let mut fs = cancel::run_for(ExFatFs::open_with(dev, options), usize::MAX)
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
