//! The writer refuses options that do not fit the tree before writing, and
//! the reader refuses malformed images without panicking.

mod common;

use common::{image, sample};
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::tree::{Content, Tree, WarningKind};
use hadris_fs::{ErrorKind, NodeId};
use hadris_iso::sync::IsoImage;
use hadris_iso::{
    BootEntry, BootInfo, Detail, ElTorito, Emulation, HybridBoot, IsoOptions, NameCase, Namespace,
    Platform, Relocation, RockRidge, VolumeIdentifiers,
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
fn boot_options_are_checked_against_the_images() {
    let tree = sample(false, false);
    let el_torito = || ElTorito::new(BootEntry::new("boot/boot.img").with_load_size(4));
    let bootstrap = IsoOptions::default()
        .with_el_torito(el_torito())
        .with_hybrid(HybridBoot::mbr().with_bootstrap(vec![0x90u8; 447]));
    assert_eq!(
        refused(&tree, &bootstrap),
        (ErrorKind::LimitExceeded, Some(Detail::HybridBoot))
    );
    let fits = IsoOptions::default()
        .with_el_torito(el_torito())
        .with_hybrid(HybridBoot::mbr().with_bootstrap(vec![0x90u8; 446]));
    let mut iso = IsoImage::open(image(&tree, &fits)).unwrap();
    let mut mbr = [0u8; 512];
    iso.read_bytes(0, &mut mbr).unwrap();
    assert!(mbr[..446].iter().all(|&byte| byte == 0x90));

    let zero = IsoOptions::default().with_el_torito(ElTorito::new(
        BootEntry::new("boot/boot.img").with_load_size(0),
    ));
    assert_eq!(
        refused(&tree, &zero),
        (ErrorKind::InvalidInput, Some(Detail::BootImage))
    );
    let floppy = IsoOptions::default().with_el_torito(ElTorito::new(
        BootEntry::new("boot/boot.img").with_emulation(Emulation::Floppy144),
    ));
    assert_eq!(
        refused(&tree, &floppy),
        (ErrorKind::InvalidInput, Some(Detail::BootImage))
    );
    let mut disk = sample(false, false);
    disk.add_file("floppy.img", Content::bytes(vec![0u8; 1_474_560]))
        .unwrap();
    let floppy = IsoOptions::default().with_el_torito(ElTorito::new(
        BootEntry::new("floppy.img").with_emulation(Emulation::Floppy144),
    ));
    assert!(hadris_iso::sync::plan(&disk, &floppy).is_ok());

    let past = IsoOptions::default().with_el_torito(ElTorito::new(
        BootEntry::new("boot/boot.img").with_load_size(9),
    ));
    let report = hadris_iso::sync::plan(&tree, &past).unwrap();
    assert!(
        report
            .warnings()
            .iter()
            .any(|w| w.path() == "/boot/boot.img" && w.kind() == WarningKind::IgnoredMetadata),
        "{:?}",
        report.warnings()
    );
    let exact = IsoOptions::default().with_el_torito(el_torito());
    assert!(
        hadris_iso::sync::plan(&tree, &exact)
            .unwrap()
            .warnings()
            .iter()
            .all(|w| w.path() != "/boot/boot.img")
    );

    let two_efi = IsoOptions::default()
        .with_el_torito(
            el_torito()
                .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi))
                .with_entry(BootEntry::new("boot/boot.img").with_platform(Platform::Efi)),
        )
        .with_hybrid(HybridBoot::gpt());
    let report = hadris_iso::sync::plan(&tree, &two_efi).unwrap();
    assert!(
        report
            .warnings()
            .iter()
            .any(|w| w.path() == "/boot/efi.img" && w.kind() == WarningKind::Skipped),
        "{:?}",
        report.warnings()
    );
    let named = IsoOptions::default()
        .with_el_torito(
            el_torito()
                .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi))
                .with_entry(BootEntry::new("boot/boot.img").with_platform(Platform::Efi)),
        )
        .with_hybrid(HybridBoot::gpt().with_efi_partition("boot/efi.img"));
    assert!(
        hadris_iso::sync::plan(&tree, &named)
            .unwrap()
            .warnings()
            .iter()
            .all(|w| w.kind() != WarningKind::Skipped)
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

/// A directory record whose extent points at something that is not a
/// directory yields a listed id that reads as corrupt, not as an invalid
/// handle (inspect-2).
#[test]
fn damaged_records_behind_listed_ids_are_corrupt() {
    let tree = sample(false, false);
    let good = image(&tree, &IsoOptions::default()).into_inner();
    let mut iso = IsoImage::open(MemDevice::new(good.clone(), common::SECTOR)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    let docs = view.resolve("/DOCS").unwrap();
    let big = view.resolve("/DOCS/BIG.BIN").unwrap();
    let data = view.raw_record(big).unwrap().header().extent.get();
    let root = view.root().get() as usize;
    let record = (root..root + 2048)
        .step_by(2)
        .find(|&at| {
            let len = good[at + 2..at + 6].try_into().unwrap();
            u64::from(u32::from_le_bytes(len)) * 2048 == docs.get() && good[at + 32] == 4
        })
        .unwrap();
    let point = |bytes: &mut Vec<u8>, block: u32| {
        bytes[record + 2..record + 6].copy_from_slice(&block.to_le_bytes());
        bytes[record + 6..record + 10].copy_from_slice(&block.to_be_bytes());
    };

    let mut bad = good.clone();
    point(&mut bad, data + 1);
    let mut iso = IsoImage::open(MemDevice::new(bad, common::SECTOR)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    let listed = view
        .lookup(view.root(), hadris_fs::Name::new(b"DOCS").unwrap())
        .unwrap();
    assert_eq!(listed.get(), u64::from(data + 1) * 2048);
    assert_eq!(
        view.node_metadata(listed).unwrap_err().kind(),
        ErrorKind::Corrupt
    );

    let mut bad = good;
    point(&mut bad, u32::MAX);
    let mut iso = IsoImage::open(MemDevice::new(bad, common::SECTOR)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    let err = view
        .lookup(view.root(), hadris_fs::Name::new(b"DOCS").unwrap())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
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

/// A directory record that points back at an ancestor's extent makes the
/// tree walks fail as corrupt instead of descending forever.
#[test]
fn directory_cycles_are_corrupt() {
    let mut tree = Tree::new();
    tree.add_file("a/b/f.txt", Content::bytes("f")).unwrap();
    let good = image(&tree, &IsoOptions::default()).into_inner();
    let mut iso = IsoImage::open(MemDevice::new(good.clone(), common::SECTOR)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    let root = view.root().get() as u32 / 2048;
    let a = view.resolve("/A").unwrap().get() as usize;
    let record = (a..a + 2048)
        .find(|&at| good[at] >= 34 && good[at + 32] == 1 && good[at + 33] == b'B')
        .unwrap();
    for target in [a as u32 / 2048, root] {
        let mut bad = good.clone();
        bad[record + 2..record + 6].copy_from_slice(&target.to_le_bytes());
        bad[record + 6..record + 10].copy_from_slice(&target.to_be_bytes());
        let mut iso = IsoImage::open(MemDevice::new(bad, common::SECTOR)).unwrap();
        let mut view = iso.view(Namespace::Primary).unwrap();
        let err = <Tree as hadris_fs::sync::TreeExt>::from_filesystem(&mut view).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt);
        let out = tempfile::tempdir().unwrap();
        let err = hadris_fs::sync::extract_to_host(&mut view, "/", out.path()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    }
}

/// libarchive takes the first root directory named `rr_moved` or
/// `.rr_moved` for the relocation directory, so a tree directory with the
/// other name that sorts ahead of the real one is refused.
#[test]
fn relocation_names_libarchive_would_mistake_are_refused() {
    let tree = |user: &str| {
        let mut tree = sample(true, false);
        tree.add_file(&format!("{user}/user.txt"), Content::bytes("user"))
            .unwrap();
        tree
    };
    let options = |case: NameCase, container: &str| {
        IsoOptions::default().with_name_case(case).with_rock_ridge(
            RockRidge::default().with_relocation(Relocation::Directory(container.into())),
        )
    };
    for (case, container, user) in [
        (NameCase::Preserve, "rr_moved", ".rr_moved"),
        (NameCase::Upper, ".rr_moved", "rr_moved"),
    ] {
        assert_eq!(
            refused(&tree(user), &options(case, container)),
            (ErrorKind::InvalidInput, Some(Detail::Relocation)),
            "{case:?} {container} {user}"
        );
        let shallow = {
            let mut tree = sample(false, false);
            tree.add_file(&format!("{user}/user.txt"), Content::bytes("user"))
                .unwrap();
            tree
        };
        image(&shallow, &options(case, container));
    }
    for (case, container, user) in [
        (NameCase::Upper, "rr_moved", ".rr_moved"),
        (NameCase::Preserve, ".rr_moved", "rr_moved"),
    ] {
        let mut iso = IsoImage::open(image(&tree(user), &options(case, container))).unwrap();
        let mut view = iso.view(Namespace::RockRidge).unwrap();
        assert_eq!(
            view.read_to_vec("/a/b/c/d/e/f/g/h/i/deep.txt").unwrap(),
            b"deep"
        );
    }
}
