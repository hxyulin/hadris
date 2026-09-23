//! The writer refuses options that do not fit the tree before writing, and
//! the reader refuses malformed images without panicking.

mod common;

use common::{image, sample};
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::tree::{Content, Tree};
use hadris_fs::{ErrorKind, NodeId};
use hadris_iso::sync::IsoImage;
use hadris_iso::{
    BootEntry, BootInfo, Detail, ElTorito, HybridBoot, IsoOptions, Namespace, Relocation,
    RockRidge, VolumeIdentifiers,
};
use hadris_storage::MemDevice;

fn refused(tree: &Tree, options: &IsoOptions) -> (ErrorKind, Option<Detail>) {
    let err = hadris_iso::sync::plan(tree, options).unwrap_err();
    let mut dev = MemDevice::new(vec![0u8; 1 << 20], common::SECTOR);
    let write = hadris_iso::sync::write(&mut dev, tree, options).unwrap_err();
    assert_eq!((write.kind(), write.detail()), (err.kind(), err.detail()));
    assert!(
        dev.get_ref().iter().all(|&b| b == 0),
        "a refused write writes nothing"
    );
    (err.kind(), err.detail())
}

#[test]
fn bad_options_are_refused_before_writing() {
    let tree = sample(false, false);
    let missing = IsoOptions::default().with_el_torito(ElTorito::new(BootEntry::new("nope.img")));
    assert_eq!(
        refused(&tree, &missing),
        (ErrorKind::InvalidInput, Some(Detail::BootImage))
    );
    let small = {
        let mut tree = sample(false, false);
        tree.add_file("tiny.img", Content::bytes([1u8; 16]))
            .unwrap();
        tree
    };
    let info = IsoOptions::default().with_el_torito(ElTorito::new(
        BootEntry::new("tiny.img").with_boot_info_table(BootInfo::Grub2),
    ));
    assert_eq!(
        refused(&small, &info),
        (ErrorKind::InvalidInput, Some(Detail::BootInfoTable))
    );
    let clash = IsoOptions::default().with_el_torito(
        ElTorito::new(BootEntry::new("boot/boot.img")).with_catalog_path("readme.txt"),
    );
    assert_eq!(
        refused(&tree, &clash),
        (ErrorKind::InvalidInput, Some(Detail::CatalogPath))
    );
    let long = IsoOptions::default().with_volume(VolumeIdentifiers::new("X".repeat(33)));
    assert_eq!(
        refused(&tree, &long),
        (ErrorKind::InvalidInput, Some(Detail::Identifier))
    );
    let efi =
        IsoOptions::default().with_hybrid(HybridBoot::gpt().with_efi_partition("missing.img"));
    assert_eq!(
        refused(&tree, &efi),
        (ErrorKind::InvalidInput, Some(Detail::HybridBoot))
    );

    let deep = sample(true, true);
    let reject = IsoOptions::default()
        .with_rock_ridge(RockRidge::default().with_relocation(Relocation::Reject));
    assert_eq!(
        refused(&deep, &reject),
        (ErrorKind::InvalidInput, Some(Detail::Relocation))
    );
    let mut taken = sample(true, true);
    taken.add_file("rr_moved", Content::empty()).unwrap();
    let relocate = IsoOptions::default().with_rock_ridge(RockRidge::default());
    assert_eq!(
        refused(&taken, &relocate),
        (ErrorKind::InvalidInput, Some(Detail::Relocation))
    );
    let named = IsoOptions::default().with_rock_ridge(
        RockRidge::default().with_relocation(Relocation::Directory("deep".into())),
    );
    let mut iso = IsoImage::open(image(&taken, &named)).unwrap();
    let mut view = iso.view(Namespace::RockRidge).unwrap();
    assert_eq!(
        view.read_to_vec("/a/b/c/d/e/f/g/h/i/deep.txt").unwrap(),
        b"deep"
    );
}

#[test]
fn output_blocks_must_divide_2048() {
    let tree = sample(false, false);
    let dev = MemDevice::new(
        vec![0u8; 1 << 20],
        hadris_storage::BlockSize::new(4096).unwrap(),
    );
    let err = hadris_iso::sync::write(dev, &tree, &IsoOptions::default()).unwrap_err();
    assert_eq!(
        (err.kind(), err.detail()),
        (ErrorKind::Unsupported, Some(Detail::OutputBlockSize))
    );
}

#[test]
fn malformed_images_are_refused() {
    let tree = sample(false, false);
    let good = image(&tree, &IsoOptions::default()).into_inner();

    let mut bad = good.clone();
    bad[16 * 2048 + 1] = b'X';
    let err = IsoImage::open(MemDevice::new(bad, common::SECTOR)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);

    let mut bad = good.clone();
    bad[16 * 2048 + 128] = 0x05;
    assert_eq!(
        IsoImage::open(MemDevice::new(bad, common::SECTOR))
            .unwrap_err()
            .kind(),
        ErrorKind::Corrupt
    );

    let mut iso = IsoImage::open(MemDevice::new(good.clone(), common::SECTOR)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    let err = view.node_metadata(NodeId::new(17 * 2048 + 3)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidHandle);
    let err = view.node_metadata(NodeId::new(1 << 40)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidHandle);
    let readme = view.resolve("/README.TXT").unwrap();
    let record = view.raw_record(readme).unwrap();
    assert_eq!(record.name(), b"README.TXT;1");

    let offset = readme.get() as usize;
    let mut bad = good.clone();
    bad[offset + 2] ^= 0x40;
    let mut iso = IsoImage::open(MemDevice::new(bad, common::SECTOR)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    assert_eq!(
        view.read_to_vec("/README.TXT").unwrap_err().kind(),
        ErrorKind::Corrupt
    );

    let terminator = (16..32)
        .map(|sector| sector * 2048)
        .find(|&offset| good[offset] == 255)
        .unwrap();
    let mut bad = good.clone();
    bad[terminator + 100] = 1;
    assert_eq!(
        IsoImage::open(MemDevice::new(bad, common::SECTOR))
            .unwrap_err()
            .kind(),
        ErrorKind::Corrupt
    );

    let mut truncated = good;
    truncated.truncate(18 * 2048);
    assert!(IsoImage::open(MemDevice::new(truncated, common::SECTOR)).is_err());
}

#[test]
fn missing_namespaces_are_reported() {
    let tree = sample(false, false);
    let mut iso = IsoImage::open(image(&tree, &IsoOptions::default())).unwrap();
    let err = iso.view(Namespace::Joliet).unwrap_err();
    assert_eq!(
        (err.kind(), err.detail()),
        (ErrorKind::NotFound, Some(Detail::NoNamespace))
    );
    assert!(iso.boot_catalog().unwrap().is_none());
    let err = iso.into_view(Namespace::RockRidge).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn directories_past_the_end_of_a_truncated_image_are_corrupt() {
    let tree = sample(false, false);
    let good = image(&tree, &IsoOptions::default()).into_inner();
    let mut iso = IsoImage::open(MemDevice::new(good.clone(), common::SECTOR)).unwrap();
    let docs = iso
        .view(Namespace::Primary)
        .unwrap()
        .resolve("/DOCS")
        .unwrap();

    let mut truncated = good;
    truncated.truncate(docs.get() as usize);
    let mut iso = IsoImage::open(MemDevice::new(truncated, common::SECTOR)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    let root = view.root();
    let listed = view
        .lookup(root, hadris_fs::Name::new(b"DOCS").unwrap())
        .unwrap();
    assert_eq!(listed, docs);
    assert_eq!(
        view.node_metadata(listed).unwrap_err().kind(),
        ErrorKind::Corrupt
    );
    assert_eq!(
        view.node_metadata(NodeId::new(1 << 40)).unwrap_err().kind(),
        ErrorKind::InvalidHandle
    );
}
