//! Volumes pass the read-only `FileSystem` contract kit, in each mode and
//! tier.

mod common;

use common::{SECTOR, image, sample};
use hadris_storage::MemDevice;
use hadris_udf::{UdfOptions, UdfRevision};

#[test]
fn sync_raw_and_shared_tiers() {
    for revision in [UdfRevision::V1_02, UdfRevision::V2_01] {
        let bytes = image(&sample(), &UdfOptions::default().with_revision(revision));
        let mut udf =
            hadris_udf::sync::UdfFs::open(MemDevice::new(bytes.as_slice(), SECTOR)).unwrap();
        hadris_fs::sync::contract::check_read_only(&mut udf).unwrap();
        let vol = hadris_fs::sync::Volume::new(
            hadris_udf::sync::UdfFs::open(MemDevice::new(bytes, SECTOR)).unwrap(),
        );
        hadris_fs::sync::contract::check_read_only(&mut *vol.lock()).unwrap();
    }
}

#[test]
fn async_modes() {
    let bytes = image(&sample(), &UdfOptions::default());
    common::block_on(async {
        let mut udf = hadris_udf::r#async::UdfFs::open(MemDevice::new(bytes.as_slice(), SECTOR))
            .await
            .map_err(|_| ())
            .unwrap();
        hadris_fs::r#async::contract::check_read_only(&mut udf)
            .await
            .unwrap();
        let udf = hadris_udf::r#async::UdfFs::open(MemDevice::new(bytes, SECTOR))
            .await
            .unwrap();
        let vol = hadris_fs::r#async::Volume::new(udf);
        hadris_fs::r#async::contract::check_read_only(&mut *vol.lock().await)
            .await
            .unwrap();
    });
}
