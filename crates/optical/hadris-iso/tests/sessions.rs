//! Sessions read an image into a tree and write it back, as a new session
//! or in place, keeping boot data.

mod common;

use common::Paths;
use common::{image, pattern, sample};
use hadris_fs::MountOptions;
use hadris_fs::{Content, Node};
use hadris_iso::sync::{IsoFs, Session};
use hadris_iso::{
    BootEntry, BootInfo, ElTorito, HybridBoot, IsoOptions, JolietLevel, Namespace, Platform,
    RockRidge, SessionMode,
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
    let mut iso = MemDevice::new(bytes, common::SECTOR);
    assert_eq!(
        IsoFs::mount(&mut iso, MountOptions::new())
            .unwrap()
            .boot_catalog()
            .unwrap()
            .unwrap()
            .block(),
        catalog
    );
    for ns in [Namespace::RockRidge, Namespace::Joliet] {
        let mut view = IsoFs::mount_namespace(&mut iso, MountOptions::new(), ns).unwrap();
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
    let mut rr =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
    assert!(rr.exists("/docs/link").unwrap());
    assert!(rr.exists("/dev/null").unwrap());
}

#[test]
fn both_modes_write_changed_trees_back() {
    let tree = sample(true, true);
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let dev = grown(image(&tree, &options()));
        let catalog = IsoFs::mount(
            MemDevice::new(dev.get_ref().clone(), common::SECTOR),
            MountOptions::new(),
        )
        .unwrap()
        .boot_catalog_block()
        .unwrap();
        let mut session = Session::open(dev).unwrap();
        assert_eq!(session.tree().entry("docs/big.bin").unwrap().links(), 1);
        session
            .tree_mut()
            .insert("added/new.txt", Node::file(Content::bytes("brand new")))
            .unwrap();
        session.tree_mut().remove("docs/big.bin").unwrap();
        let opts = session.options();
        assert!(opts.rock_ridge().is_some() && opts.joliet().is_some());
        let first = session.write(&opts, mode).unwrap();
        session
            .tree_mut()
            .insert("added/second.txt", Node::file(Content::bytes("second")))
            .unwrap();
        let second = session.write(&opts, mode).unwrap();
        assert!(second.size() / 2048 > first.size() / 2048, "{mode:?}");
        assert_eq!(
            second.extents("readme.txt").map(|e| e[0]),
            first.extents("readme.txt").map(|e| e[0])
        );
        assert_eq!(
            second.extents("added/new.txt").map(|e| e[0]),
            first.extents("added/new.txt").map(|e| e[0])
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
                    covered * 512 >= (second.size() / 2048 - 9) * 2048,
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
    let mut iso = session.into_inner();
    let catalog = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .boot_catalog()
        .unwrap()
        .unwrap();
    assert_eq!(
        u64::from(catalog.default_entry().load_block()) * 2048,
        report
            .extents("BOOT/BOOT.IMG")
            .map(|e| e[0])
            .unwrap()
            .offset()
    );
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Preferred).unwrap();
    assert_eq!(view.read_to_vec("/docs/big.bin").unwrap(), pattern(100_000));
}

/// A kept catalog follows the tree path of its boot images (osdev-5).
#[test]
fn kept_catalogs_follow_replaced_boot_images() {
    let tree = sample(true, true);
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let mut session = Session::open(grown(image(&tree, &options()))).unwrap();
        let replaced = vec![0x5Au8; 4096];
        session
            .tree_mut()
            .replace(
                "boot/boot.img",
                Node::file(Content::bytes(replaced.clone())),
            )
            .unwrap();
        let opts = session.options();
        let report = session.write(&opts, mode).unwrap();
        let moved = report.extents("boot/boot.img").map(|e| e[0]).unwrap();
        let mut iso = session.into_inner();
        let catalog = IsoFs::mount(&mut iso, MountOptions::new())
            .unwrap()
            .boot_catalog()
            .unwrap()
            .unwrap();
        let entries: Vec<_> = catalog.entries().iter().map(|e| e.load_block()).collect();
        assert_eq!(u64::from(entries[0]) * 2048, moved.offset(), "{mode:?}");
        assert_eq!(
            u64::from(entries[1]) * 2048,
            report
                .extents("boot/efi.img")
                .map(|e| e[0])
                .unwrap()
                .offset()
        );
        let mut loaded = vec![0u8; 4096];
        IsoFs::mount(&mut iso, MountOptions::new())
            .unwrap()
            .read_raw(moved.offset(), &mut loaded)
            .unwrap();
        assert_eq!(loaded, replaced);
        let mut view =
            IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
        let listed = view.read_to_vec("/boot/boot.cat").unwrap();
        assert_eq!(
            u32::from_le_bytes(listed[40..44].try_into().unwrap()),
            entries[0]
        );
    }

    let mut session = Session::open(grown(image(&tree, &options()))).unwrap();
    session.tree_mut().remove("boot/efi.img").unwrap();
    let opts = session.options();
    let before = session.volume_blocks();
    let err = session.write(&opts, SessionMode::Rewrite).unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.detail().and_then(hadris_iso::Detail::from_code)
        ),
        (
            hadris_fs::ErrorKind::InvalidInput,
            Some(hadris_iso::Detail::BootImage)
        )
    );
    assert_eq!(session.volume_blocks(), before);
}

/// A kept catalog entry whose image is replaced gets the new image's load
/// size, unless it loaded only part of the old one, and the new image gets
/// the boot information table the old one had.
#[test]
fn replaced_boot_images_get_load_sizes_and_info_tables() {
    let tree = sample(false, false);
    let opts = IsoOptions::default()
        .with_rock_ridge(RockRidge::default())
        .with_el_torito(
            ElTorito::new(
                BootEntry::new("boot/boot.img")
                    .with_load_size(4)
                    .with_boot_info_table(BootInfo::Grub2),
            )
            .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi)),
        );
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let mut session = Session::open(grown(image(&tree, &opts))).unwrap();
        let bios = pattern(10_001);
        let efi = vec![0xEEu8; 3000];
        session
            .tree_mut()
            .replace("boot/boot.img", Node::file(Content::bytes(bios.clone())))
            .unwrap();
        session
            .tree_mut()
            .replace("boot/efi.img", Node::file(Content::bytes(efi)))
            .unwrap();
        let kept = session.options();
        let report = session.write(&kept, mode).unwrap();
        let moved = report.extents("boot/boot.img").map(|e| e[0]).unwrap();
        let mut iso = session.into_inner();
        let catalog = IsoFs::mount(&mut iso, MountOptions::new())
            .unwrap()
            .boot_catalog()
            .unwrap()
            .unwrap();
        let counts: Vec<_> = catalog.entries().iter().map(|e| e.sector_count()).collect();
        assert_eq!(counts, [4, 6], "{mode:?}");
        let mut loaded = vec![0u8; bios.len()];
        IsoFs::mount(&mut iso, MountOptions::new())
            .unwrap()
            .read_raw(moved.offset(), &mut loaded)
            .unwrap();
        let word = |at: usize| u32::from_le_bytes(loaded[at..at + 4].try_into().unwrap());
        let sum = bios[64..64 + (bios.len() - 64) / 4 * 4]
            .chunks_exact(4)
            .fold(0u32, |sum, w| {
                sum.wrapping_add(u32::from_le_bytes(w.try_into().unwrap()))
            });
        assert_eq!(
            [word(8), word(12), word(16), word(20)],
            [16, (moved.offset() / 2048) as u32, bios.len() as u32, sum],
            "{mode:?}"
        );
        assert!(loaded[24..64].iter().all(|&b| b == 0), "{mode:?}");
        assert_eq!(loaded[..8], bios[..8]);
        assert_eq!(loaded[64..], bios[64..]);
    }
}

/// An appended session leaves the system area alone and says so when the
/// options ask for hybrid boot (osdev-7).
#[test]
fn appended_sessions_report_ignored_hybrid_options() {
    let tree = sample(false, false);
    let mut session = Session::open(grown(image(&tree, &IsoOptions::default()))).unwrap();
    let opts = session.options().with_hybrid(HybridBoot::mbr());
    let report = session.write(&opts, SessionMode::Append).unwrap();
    assert!(
        report
            .warnings()
            .iter()
            .any(|w| w.message().contains("hybrid boot options")),
        "{:?}",
        report.warnings()
    );
    let bytes = session.into_inner().into_inner();
    assert!(bytes[..512].iter().all(|&byte| byte == 0));
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
        .insert("x.txt", Node::file(Content::bytes("x")))
        .unwrap();
    sync_session.write(&opts, SessionMode::Append).unwrap();
    let expected = sync_session.into_inner().into_inner();
    common::block_on(async {
        let mut session = hadris_iso::r#async::Session::open(dev).await.unwrap();
        session
            .tree_mut()
            .insert("x.txt", Node::file(Content::bytes("x")))
            .unwrap();
        session.write(&opts, SessionMode::Append).await.unwrap();
        assert_eq!(session.into_inner().into_inner(), expected);
    });
}
