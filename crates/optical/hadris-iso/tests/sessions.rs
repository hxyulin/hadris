//! Sessions read an image into a tree and write it back, as a new session
//! or in place, keeping boot data.

mod common;

use common::{IsoExtras, Paths};
use common::{image, pattern, sample};
use hadris_fs::MountOptions;
use hadris_fs::{Content, Node};
use hadris_iso::sync::{IsoFs, Session};
use hadris_iso::{BootEntry, BootInfo, ElTorito, Hybrid, IsoOptions, Namespace, SessionMode};
use hadris_storage::MemDevice;

fn options() -> IsoOptions {
    IsoOptions::default()
        .with_joliet()
        .with_rock_ridge()
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("boot/boot.img"))
                .with_entry(BootEntry::uefi("boot/efi.img"))
                .with_catalog_path("boot/boot.cat"),
        )
        .with_hybrid(Hybrid::gpt_hybrid_mbr())
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
            .catalog()
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
        .catalog()
        .unwrap()
        .map(|c| c.block())
        .unwrap();
        let mut session = Session::open(dev).unwrap();
        assert_eq!(session.tree().entry("docs/big.bin").unwrap().links(), 1);
        session
            .tree_mut()
            .insert("added/new.txt", Node::file(Content::bytes("brand new")))
            .unwrap();
        session.tree_mut().remove("docs/big.bin").unwrap();
        let opts = session.options();
        assert!(opts.rock_ridge() && opts.joliet());
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
        .with_el_torito(ElTorito::new().with_entry(BootEntry::bios("BOOT/BOOT.IMG")));
    let report = session.write(&opts, SessionMode::Rewrite).unwrap();
    let mut iso = session.into_inner();
    let catalog = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .catalog()
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
            .catalog()
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
    let opts = IsoOptions::default().with_rock_ridge().with_el_torito(
        ElTorito::new()
            .with_entry(
                BootEntry::bios("boot/boot.img")
                    .with_load_size(4)
                    .with_boot_info(BootInfo::Grub2),
            )
            .with_entry(BootEntry::uefi("boot/efi.img")),
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
            .catalog()
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
    let opts = session.options().with_hybrid(Hybrid::mbr());
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
    let opts = IsoOptions::default().with_rock_ridge();
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

struct ReadBounded(MemDevice<Vec<u8>>);

impl hadris_io::ErrorType for ReadBounded {
    type Error = core::convert::Infallible;
}

impl hadris_storage::sync::BlockDevice for ReadBounded {
    fn block_size(&self) -> hadris_storage::BlockSize {
        common::SECTOR
    }

    fn block_count(&self) -> u64 {
        self.0.get_ref().len() as u64 / 2048
    }

    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        assert!(buf.len() <= 64 * 1024, "export must stream file content");
        hadris_storage::sync::BlockDevice::read_blocks(&mut self.0, first, buf)
    }
}

#[test]
fn export_streams_edited_tree_to_independent_geometry() {
    use hadris_fs::{Extent, Tree};
    use hadris_storage::BlockSize;

    let opts = IsoOptions::new().with_rock_ridge();
    let payload = pattern(200_003);
    let mut tree = Tree::new();
    tree.insert("large.bin", Node::file(Content::bytes(payload.clone())))
        .unwrap();
    tree.insert("empty", Node::file(Content::empty())).unwrap();
    tree.insert("removed", Node::file(Content::bytes("gone")))
        .unwrap();
    tree.insert("changed", Node::file(Content::bytes("before")))
        .unwrap();
    let original = image(&tree, &opts).into_inner();
    let mut session = Session::open(ReadBounded(MemDevice::new(
        original.clone(),
        common::SECTOR,
    )))
    .unwrap();
    let start = session
        .tree()
        .get("large.bin")
        .unwrap()
        .content()
        .unwrap()
        .stored_extents()
        .unwrap()[0]
        .offset();
    session
        .tree_mut()
        .insert(
            "pieces",
            Node::file(
                Content::stored(vec![
                    Extent::new(start + 2047, 70_001),
                    Extent::new(start + 100_003, 87_001),
                ])
                .unwrap(),
            ),
        )
        .unwrap();
    session.tree_mut().link("large.bin", "hardlink").unwrap();
    session.tree_mut().remove("removed").unwrap();
    session
        .tree_mut()
        .replace("changed", Node::file(Content::bytes("after")))
        .unwrap();
    session
        .tree_mut()
        .insert("added", Node::file(Content::bytes("new")))
        .unwrap();
    let mut out = MemDevice::new(vec![0; 1 << 20], BlockSize::new(512).unwrap());
    let report = session.export(&mut out, &opts).unwrap();
    assert_eq!(report.extents("large.bin"), report.extents("hardlink"));
    let mut iso = IsoFs::mount(out, MountOptions::new()).unwrap();
    assert_eq!(iso.read_to_vec("/large.bin").unwrap(), payload);
    assert_eq!(iso.read_to_vec("/hardlink").unwrap(), payload);
    assert_eq!(
        iso.read_to_vec("/pieces").unwrap(),
        [&payload[2047..72_048], &payload[100_003..187_004]].concat()
    );
    assert_eq!(iso.read_to_vec("/empty").unwrap(), b"");
    assert_eq!(iso.read_to_vec("/changed").unwrap(), b"after");
    assert_eq!(iso.read_to_vec("/added").unwrap(), b"new");
    assert!(!iso.exists("/removed").unwrap());
    assert_eq!(
        session
            .tree()
            .get("large.bin")
            .unwrap()
            .content()
            .unwrap()
            .stored_extents()
            .unwrap()[0]
            .offset(),
        start
    );
    let mut second = MemDevice::new(vec![0; 1 << 20], common::SECTOR);
    session.export(&mut second, &opts).unwrap();
    assert_eq!(session.into_inner().0.into_inner(), original);
}

#[test]
fn export_rejects_outside_source_before_writing() {
    use hadris_fs::{ErrorKind, Extent, Tree};

    let opts = IsoOptions::new();
    let dev = image(&Tree::new(), &opts);
    let end = dev.get_ref().len() as u64;
    let mut session = Session::open(dev).unwrap();
    session
        .tree_mut()
        .insert(
            "outside",
            Node::file(Content::stored(vec![Extent::new(end - 1, 2)]).unwrap()),
        )
        .unwrap();
    let initial = vec![0xA5; 1 << 20];
    let mut out = MemDevice::new(initial.clone(), common::SECTOR);
    let err = session.export(&mut out, &opts).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
    assert!(err.path().is_some());
    assert_eq!(out.into_inner(), initial);
}

#[test]
fn async_export_matches_sync() {
    let opts = IsoOptions::new().with_rock_ridge();
    let tree = sample(false, true);
    let original = image(&tree, &opts).into_inner();
    let mut session = Session::open(MemDevice::new(original.clone(), common::SECTOR)).unwrap();
    session
        .tree_mut()
        .insert("added", Node::file(Content::bytes("new")))
        .unwrap();
    let mut expected = MemDevice::new(vec![0; 1 << 20], common::SECTOR);
    session.export(&mut expected, &opts).unwrap();
    common::block_on(async {
        let mut session =
            hadris_iso::r#async::Session::open(MemDevice::new(original, common::SECTOR))
                .await
                .unwrap();
        session
            .tree_mut()
            .insert("added", Node::file(Content::bytes("new")))
            .unwrap();
        let mut out = MemDevice::new(vec![0; 1 << 20], common::SECTOR);
        session.export(&mut out, &opts).await.unwrap();
        assert_eq!(out.into_inner(), expected.into_inner());
    });
}

#[test]
fn export_rebuilds_explicit_boot_info_from_stored_content() {
    let tree = sample(false, false);
    let initial_opts = IsoOptions::new().with_rock_ridge();
    let mut session = Session::open(image(&tree, &initial_opts)).unwrap();
    let opts = initial_opts.with_el_torito(
        ElTorito::new()
            .with_entry(BootEntry::bios("boot/boot.img").with_boot_info(BootInfo::Grub2)),
    );
    let mut out = MemDevice::new(vec![0; 1 << 20], common::SECTOR);
    let report = session.export(&mut out, &opts).unwrap();
    let offset = report.extents("boot/boot.img").unwrap()[0].offset();
    let mut iso = IsoFs::mount(out, MountOptions::new()).unwrap();
    assert!(iso.catalog().unwrap().is_some());
    let bytes = iso.read_to_vec("/boot/boot.img").unwrap();
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(word(12), (offset / 2048) as u32);
    assert_eq!(word(16), 4096);
    assert_eq!(
        word(20),
        (0..(4096 - 64) / 4).fold(0u32, |sum, _| sum.wrapping_add(0x9090_9090))
    );
    assert_eq!(&bytes[64..], &[0x90; 4096 - 64]);
}
