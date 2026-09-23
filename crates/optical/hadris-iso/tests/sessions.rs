//! Sessions read an image into a tree and write it back, as a new session
//! or in place, keeping boot data.

mod common;

use common::{image, pattern, sample};
use hadris_fs::sync::DriverExt;
use hadris_fs::tree::Content;
use hadris_iso::sync::{IsoImage, Session};
use hadris_iso::{
    BootEntry, ElTorito, HybridBoot, IsoOptions, JolietLevel, Namespace, Platform, RockRidge,
    SessionMode,
};
use hadris_storage::MemDevice;

fn options() -> IsoOptions {
    IsoOptions::default()
        .with_joliet(JolietLevel::L3)
        .with_rock_ridge(RockRidge::default())
        .with_el_torito(
            ElTorito::new(BootEntry::new("boot/boot.img"))
                .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi))
                .with_catalog_path("boot/boot.cat"),
        )
        .with_hybrid(HybridBoot::hybrid())
}

fn grown(dev: MemDevice<Vec<u8>>) -> MemDevice<Vec<u8>> {
    let mut bytes = dev.into_inner();
    bytes.resize(bytes.len() + (1 << 20), 0);
    MemDevice::new(bytes, common::SECTOR)
}

fn check(bytes: Vec<u8>, catalog: u32) {
    let mut iso = IsoImage::open(MemDevice::new(bytes, common::SECTOR)).unwrap();
    assert_eq!(iso.boot_catalog().unwrap().unwrap().block(), catalog);
    for ns in [Namespace::RockRidge, Namespace::Joliet] {
        let mut view = iso.view(ns).unwrap();
        assert_eq!(
            view.read_to_vec("/added/new.txt").unwrap(),
            b"brand new",
            "{ns:?}"
        );
        assert_eq!(
            view.read_to_vec("/added/second.txt").unwrap(),
            b"second",
            "{ns:?}"
        );
        assert_eq!(
            view.read_to_vec("/readme.txt").unwrap(),
            b"hello world\n",
            "{ns:?}"
        );
        assert!(!view.exists("/docs/big.bin").unwrap(), "{ns:?}");
        assert_eq!(
            view.read_to_vec("/a/b/c/d/e/f/g/h/i/deep.txt").unwrap(),
            b"deep",
            "{ns:?}"
        );
        hadris_fs::sync::contract::check_read_only(&mut view).unwrap();
    }
    let mut rr = iso.view(Namespace::RockRidge).unwrap();
    assert!(rr.exists("/docs/link").unwrap());
    assert!(rr.exists("/dev/null").unwrap());
}

#[test]
fn both_modes_write_changed_trees_back() {
    let tree = sample(true, true);
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let dev = grown(image(&tree, &options()));
        let catalog = IsoImage::open(MemDevice::new(dev.get_ref().clone(), common::SECTOR))
            .unwrap()
            .boot_catalog_block()
            .unwrap();
        let mut session = Session::open(dev).unwrap();
        assert_eq!(session.tree().get("docs/big.bin").unwrap().links(), 1);
        session
            .tree_mut()
            .add_file("added/new.txt", Content::bytes("brand new"))
            .unwrap();
        session.tree_mut().remove("docs/big.bin").unwrap();
        let opts = session.options();
        assert!(opts.rock_ridge().is_some() && opts.joliet().is_some());
        let first = session.write(&opts, mode).unwrap();
        session
            .tree_mut()
            .add_file("added/second.txt", Content::bytes("second"))
            .unwrap();
        let second = session.write(&opts, mode).unwrap();
        assert!(second.total_blocks() > first.total_blocks(), "{mode:?}");
        assert_eq!(
            second.extent_of("readme.txt"),
            first.extent_of("readme.txt")
        );
        assert_eq!(
            second.extent_of("added/new.txt"),
            first.extent_of("added/new.txt")
        );
        let bytes = session.into_inner().into_inner();
        let disk = hadris_part::sync::read(&mut MemDevice::new(
            bytes.clone(),
            hadris_storage::BlockSize::new(512).unwrap(),
        ))
        .unwrap();
        let covered = disk.partitions().map(|p| p.end()).max().unwrap();
        match mode {
            SessionMode::Rewrite => {
                assert!(
                    covered * 512 >= (second.total_blocks() - 9) * 2048,
                    "{mode:?} partitions grow"
                );
                assert!(disk.partitions().count() == 3);
            }
            _ => assert!(
                second
                    .warnings()
                    .iter()
                    .any(|w| w.message().contains("partition"))
            ),
        }
        check(bytes, catalog);
    }
}

#[test]
fn new_boot_options_replace_the_catalog() {
    let tree = sample(false, false);
    let dev = grown(image(&tree, &IsoOptions::default()));
    let mut session = Session::open(dev).unwrap();
    assert!(session.tree().get("BOOT/BOOT.IMG").is_some());
    let opts = session
        .options()
        .with_el_torito(ElTorito::new(BootEntry::new("BOOT/BOOT.IMG")));
    let report = session.write(&opts, SessionMode::Rewrite).unwrap();
    let mut iso = IsoImage::open(session.into_inner()).unwrap();
    let catalog = iso.boot_catalog().unwrap().unwrap();
    assert_eq!(
        u64::from(catalog.default_entry().load_block()) * 2048,
        report.extent_of("BOOT/BOOT.IMG").unwrap().offset()
    );
    let mut view = iso.view(Namespace::Preferred).unwrap();
    assert_eq!(view.read_to_vec("/docs/big.bin").unwrap(), pattern(100_000));
}

#[test]
fn async_sessions_match_sync_ones() {
    let tree = sample(false, true);
    let opts = IsoOptions::default().with_rock_ridge(RockRidge::default());
    let dev = grown(image(&tree, &opts));
    let mut sync_session =
        Session::open(MemDevice::new(dev.get_ref().clone(), common::SECTOR)).unwrap();
    sync_session
        .tree_mut()
        .add_file("x.txt", Content::bytes("x"))
        .unwrap();
    sync_session.write(&opts, SessionMode::Append).unwrap();
    let expected = sync_session.into_inner().into_inner();
    common::block_on(async {
        let mut session = hadris_iso::async_send::Session::open(dev).await.unwrap();
        session
            .tree_mut()
            .add_file("x.txt", Content::bytes("x"))
            .unwrap();
        session.write(&opts, SessionMode::Append).await.unwrap();
        assert_eq!(session.into_inner().into_inner(), expected);
    });
}
