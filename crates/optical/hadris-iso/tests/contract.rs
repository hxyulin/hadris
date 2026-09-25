//! Every tree of an image passes the read-only `FileSystem` contract kit, in
//! each mode and tier.

mod common;

use common::{image, sample};
use hadris_fs::MountOptions;
use hadris_iso::sync::IsoFs;
use hadris_iso::{IsoOptions, Namespace};

fn options() -> IsoOptions {
    IsoOptions::default()
        .with_joliet()
        .with_rock_ridge()
        .with_iso1999()
}

#[test]
fn sync_raw_and_shared_tiers() {
    let tree = sample(true, true);
    let mut iso = image(&tree, &options());
    for ns in [
        Namespace::RockRidge,
        Namespace::Joliet,
        Namespace::Enhanced,
        Namespace::Primary,
    ] {
        let mut view = IsoFs::mount_namespace(&mut iso, MountOptions::new(), ns).unwrap();
        hadris_fs::sync::contract::check_read_only(&mut view)
            .unwrap_or_else(|err| panic!("{ns:?}: {err}"));
    }
    let view = IsoFs::mount_namespace(iso, MountOptions::new(), Namespace::Preferred).unwrap();
    let vol = hadris_fs::sync::Volume::new(view);
    hadris_fs::sync::contract::check_read_only(&mut *vol.lock()).unwrap();
}

#[test]
fn async_modes() {
    let tree = sample(true, true);
    let bytes = image(&tree, &options());
    common::block_on(async {
        let dev = hadris_storage::MemDevice::new(bytes.get_ref().as_slice(), common::SECTOR);
        let mut view = hadris_iso::r#async::IsoFs::mount_namespace(
            dev,
            MountOptions::new(),
            Namespace::RockRidge,
        )
        .await
        .map_err(|_| ())
        .unwrap();
        hadris_fs::r#async::contract::check_read_only(&mut view)
            .await
            .unwrap();
        let view = hadris_iso::r#async::IsoFs::mount_namespace(
            bytes,
            MountOptions::new(),
            Namespace::Joliet,
        )
        .await
        .unwrap();
        let vol = hadris_fs::r#async::Volume::new(view);
        hadris_fs::r#async::contract::check_read_only(&mut *vol.lock().await)
            .await
            .unwrap();
    });
}
