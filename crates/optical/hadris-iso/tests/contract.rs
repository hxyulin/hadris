//! Every tree of an image passes the read-only `FsDriver` contract kit, in
//! each mode and tier.

mod common;

use common::{image, sample};
use hadris_iso::{IsoOptions, JolietLevel, Namespace, RockRidge};

fn options() -> IsoOptions {
    IsoOptions::default()
        .with_joliet(JolietLevel::L3)
        .with_rock_ridge(RockRidge::default())
        .with_enhanced_tree()
}

#[test]
fn sync_raw_and_shared_tiers() {
    let tree = sample(true, true);
    let mut iso = hadris_iso::sync::IsoImage::open(image(&tree, &options())).unwrap();
    for ns in [
        Namespace::RockRidge,
        Namespace::Joliet,
        Namespace::Enhanced,
        Namespace::Primary,
    ] {
        let mut view = iso.view(ns).unwrap();
        hadris_fs::sync::contract::check_read_only(&mut view)
            .unwrap_or_else(|err| panic!("{ns:?}: {err}"));
    }
    let view = iso.into_view(Namespace::Preferred).unwrap();
    let vol = hadris_fs::sync::Volume::new(view);
    hadris_fs::sync::contract::check_read_only(&mut &vol).unwrap();
}

#[test]
fn async_modes() {
    let tree = sample(true, true);
    let bytes = image(&tree, &options());
    common::block_on(async {
        let mut iso = hadris_iso::r#async::IsoImage::open(hadris_storage::MemDevice::new(
            bytes.get_ref().as_slice(),
            common::SECTOR,
        ))
        .await
        .map_err(|_| ())
        .unwrap();
        let mut view = iso.view(Namespace::RockRidge).unwrap();
        hadris_fs::r#async::contract::check_read_only(&mut view)
            .await
            .unwrap();
        let iso = hadris_iso::r#async::IsoImage::open(bytes).await.unwrap();
        let view = iso.into_view(Namespace::Joliet).unwrap();
        let vol = hadris_fs::r#async::Volume::new(view);
        hadris_fs::r#async::contract::check_read_only(&mut &vol)
            .await
            .unwrap();
    });
}
