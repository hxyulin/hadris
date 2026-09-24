//! `FatFs` passes the `hadris-fs` driver contract kit on every FAT width,
//! directly and through a `Volume`, in both modes, and the image is clean
//! afterwards.

#[path = "common/fatfs.rs"]
mod common;

use common::{CASES, block_on, fsck};
use hadris_fat::MountOptions;
use hadris_fs::HeapTable;

#[test]
fn sync_raw_tier() {
    for case in CASES {
        let mut fs =
            hadris_fat::sync::FatFs::open(common::device(case, common::blank(case))).unwrap();
        hadris_fs::sync::contract::check(&mut fs)
            .unwrap_or_else(|err| panic!("{}: {err}", case.name));
        assert_eq!(fs.open_nodes(), 1, "{}", case.name);
        fsck(&fs.into_inner().into_inner(), case.name);
    }
}

#[test]
fn sync_through_a_volume() {
    let case = CASES[1];
    let fs = hadris_fat::sync::FatFs::open_with(
        common::device(case, common::blank(case)),
        MountOptions::new().with_table(HeapTable::new()),
    )
    .unwrap();
    let vol = hadris_fs::sync::Volume::new(fs);
    hadris_fs::sync::contract::check(&mut *vol.lock()).unwrap();
    assert_eq!(vol.into_inner().unwrap().open_nodes(), 1);
}

#[test]
fn async_modes() {
    let case = CASES[2];
    block_on(async {
        let mut fs = hadris_fat::r#async::FatFs::open(common::device(case, common::blank(case)))
            .await
            .unwrap();
        hadris_fs::r#async::contract::check(&mut fs).await.unwrap();
        let fs = hadris_fat::r#async::FatFs::open(common::device(case, common::blank(case)))
            .await
            .unwrap();
        let vol = hadris_fs::r#async::Volume::new(fs);
        hadris_fs::r#async::contract::check(&mut *vol.lock().await)
            .await
            .unwrap();
    });
}
