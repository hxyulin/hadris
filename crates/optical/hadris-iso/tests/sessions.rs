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
        assert_eq!(
            [word(8), word(12), word(16), word(20)],
            [
                16,
                (moved.offset() / 2048) as u32,
                bios.len() as u32,
                words_from_64(&loaded)
            ],
            "{mode:?}"
        );
        assert!(loaded[24..64].iter().all(|&b| b == 0), "{mode:?}");
        assert_eq!(loaded[..8], bios[..8]);
        let grub2 = u64::from_le_bytes(loaded[2548..2556].try_into().unwrap());
        assert_eq!(grub2, moved.offset() / 512 + 5, "{mode:?}");
        assert_eq!(loaded[64..2548], bios[64..2548]);
        assert_eq!(loaded[2556..], bios[2556..]);
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

#[cfg(feature = "async")]
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
        let mut session = hadris_iso::async_::Session::open(dev).await.unwrap();
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

#[cfg(feature = "async")]
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
            hadris_iso::async_::Session::open(MemDevice::new(original, common::SECTOR))
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
    let grub2 = u64::from_le_bytes(bytes[2548..2556].try_into().unwrap());
    assert_eq!(grub2, offset / 512 + 5, "xorriso --grub2-boot-info");
    assert_eq!(word(20), words_from_64(&bytes));
    assert_eq!(&bytes[64..2548], &[0x90; 2548 - 64]);
    assert_eq!(&bytes[2556..], &[0x90; 4096 - 2556]);
}

/// The boot information checksum of the image `bytes` as written.
fn words_from_64(bytes: &[u8]) -> u32 {
    bytes[64..].chunks(4).fold(0u32, |sum, w| {
        let mut word = [0u8; 4];
        word[..w.len()].copy_from_slice(w);
        sum.wrapping_add(u32::from_le_bytes(word))
    })
}

#[test]
fn growing_descriptor_sets_move_the_files_they_cover() {
    let data = pattern(3000);
    let mut tree = hadris_fs::Tree::new();
    tree.insert("A.TXT", Node::file(Content::bytes(data.clone())))
        .unwrap();
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let dev = grown(image(&tree, &IsoOptions::default()));
        let mut session = Session::open(dev).unwrap();
        let first = session
            .tree()
            .get("A.TXT")
            .and_then(Node::content)
            .and_then(Content::stored_extents)
            .unwrap()[0]
            .offset();
        assert_eq!(first, 19 * 2048);
        session
            .tree_mut()
            .insert("boot.img", Node::file(Content::bytes(vec![0x90u8; 2048])))
            .unwrap();
        let opts = session
            .options()
            .with_joliet()
            .with_el_torito(ElTorito::new().with_entry(BootEntry::bios("boot.img")));
        let report = session.write(&opts, mode).unwrap();
        assert!(report.extents("A.TXT").unwrap()[0].offset() >= 20 * 2048);
        let mut iso = session.into_inner();
        for ns in [Namespace::Primary, Namespace::Joliet] {
            let mut view = IsoFs::mount_namespace(&mut iso, MountOptions::new(), ns).unwrap();
            assert_eq!(view.read_to_vec("/A.TXT").unwrap(), data, "{mode:?} {ns:?}");
        }
    }
}

#[test]
fn growing_descriptor_sets_refuse_to_cover_a_kept_catalog() {
    let mut tree = hadris_fs::Tree::new();
    tree.insert("BOOT.IMG", Node::file(Content::bytes(vec![0x90u8; 2048])))
        .unwrap();
    let opts = IsoOptions::default().with_el_torito(
        ElTorito::new()
            .with_entry(BootEntry::bios("BOOT.IMG"))
            .with_catalog_path("BOOT.CAT"),
    );
    let dev = grown(image(&tree, &opts));
    let original = dev.get_ref().clone();
    let mut session = Session::open(dev).unwrap();
    let opts = session.options().with_joliet().with_iso1999();
    let err = session.write(&opts, SessionMode::Rewrite).unwrap_err();
    assert_eq!(err.kind(), hadris_fs::ErrorKind::NoSpace);
    assert_eq!(session.into_inner().into_inner(), original);
}

/// The byte offset of the record named `name` in the directory at `block`.
fn record_in(bytes: &[u8], block: usize, name: &[u8]) -> usize {
    let start = block * 2048;
    let size = u32::from_le_bytes(bytes[start + 10..start + 14].try_into().unwrap()) as usize;
    let mut at = start;
    while at < start + size {
        let len = bytes[at] as usize;
        if len == 0 {
            at = (at / 2048 + 1) * 2048;
            continue;
        }
        let id_len = bytes[at + 32] as usize;
        if &bytes[at + 33..at + 33 + id_len] == name {
            return at;
        }
        at += len;
    }
    panic!("no record {name:?}");
}

#[test]
fn directory_cycles_fail_to_open() {
    let mut tree = hadris_fs::Tree::new();
    tree.insert("A/B/F.TXT", Node::file(Content::bytes("f")))
        .unwrap();
    let mut bytes = image(&tree, &IsoOptions::default()).into_inner();
    let extent = |bytes: &[u8], at: usize| {
        u32::from_le_bytes(bytes[at + 2..at + 6].try_into().unwrap()) as usize
    };
    let root = extent(&bytes, 16 * 2048 + 156);
    let a = extent(&bytes, record_in(&bytes, root, b"A"));
    let b = record_in(&bytes, a, b"B");
    bytes[b + 2..b + 6].copy_from_slice(&(root as u32).to_le_bytes());
    bytes[b + 6..b + 10].copy_from_slice(&(root as u32).to_be_bytes());
    let err = Session::open(MemDevice::new(bytes, common::SECTOR)).unwrap_err();
    assert_eq!(err.kind(), hadris_fs::ErrorKind::Corrupt);
}

#[test]
fn stored_files_keep_every_extent_record() {
    let data = pattern(100 * 2048);
    let mut tree = hadris_fs::Tree::new();
    tree.insert("MANY.BIN", Node::file(Content::bytes(data.clone())))
        .unwrap();
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        for rock_ridge in [false, true] {
            let opts = match rock_ridge {
                true => IsoOptions::default().with_rock_ridge(),
                false => IsoOptions::default(),
            };
            let mut session = Session::open(grown(image(&tree, &opts))).unwrap();
            let start = session
                .tree()
                .get("MANY.BIN")
                .and_then(Node::content)
                .and_then(Content::stored_extents)
                .unwrap()[0]
                .offset();
            let pieces: Vec<_> = (0..100)
                .map(|i| hadris_fs::Extent::new(start + i * 2048, 2048))
                .collect();
            session
                .tree_mut()
                .replace("MANY.BIN", Node::file(Content::stored(pieces).unwrap()))
                .unwrap();
            let opts = session.options();
            session.write(&opts, mode).unwrap();
            let mut iso = session.into_inner();
            let mut view = IsoFs::mount(&mut iso, MountOptions::new()).unwrap();
            let node = view.resolve_path("/MANY.BIN").unwrap();
            assert_eq!(view.all_extents(node).len(), 100, "{mode:?} {rock_ridge}");
            assert_eq!(view.read_to_vec("/MANY.BIN").unwrap(), data);
        }
    }
}

/// A memory device that counts the reads it serves.
struct Counting {
    inner: MemDevice<Vec<u8>>,
    reads: usize,
}

impl hadris_io::ErrorType for Counting {
    type Error = <MemDevice<Vec<u8>> as hadris_io::ErrorType>::Error;
}

impl hadris_storage::sync::BlockDevice for Counting {
    fn block_size(&self) -> hadris_storage::BlockSize {
        self.inner.block_size()
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }

    fn writable(&self) -> bool {
        false
    }

    fn read_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &mut [u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.reads += 1;
        self.inner.read_blocks(first, buf)
    }

    fn write_blocks(
        &mut self,
        first: hadris_storage::BlockIndex,
        buf: &[u8],
    ) -> Result<(), hadris_io::Error<Self::Error>> {
        self.inner.write_blocks(first, buf)
    }
}

#[test]
fn reading_a_file_of_many_extents_takes_linear_reads() {
    let reads = |count: u64| {
        let mut tree = hadris_fs::Tree::new();
        tree.insert(
            "MANY.BIN",
            Node::file(Content::bytes(pattern(count as usize * 2048))),
        )
        .unwrap();
        let mut session = Session::open(grown(image(&tree, &IsoOptions::default()))).unwrap();
        let start = session
            .tree()
            .get("MANY.BIN")
            .and_then(Node::content)
            .and_then(Content::stored_extents)
            .unwrap()[0]
            .offset();
        let pieces: Vec<_> = (0..count)
            .map(|i| hadris_fs::Extent::new(start + i * 2048, 2048))
            .collect();
        session
            .tree_mut()
            .replace("MANY.BIN", Node::file(Content::stored(pieces).unwrap()))
            .unwrap();
        let opts = session.options();
        session.write(&opts, SessionMode::Rewrite).unwrap();
        let mut dev = Counting {
            inner: session.into_inner(),
            reads: 0,
        };
        let session = Session::open(&mut dev).unwrap();
        let extents = session
            .tree()
            .get("MANY.BIN")
            .and_then(Node::content)
            .and_then(Content::stored_extents)
            .unwrap()
            .len();
        assert_eq!(extents as u64, count);
        drop(session);
        dev.reads
    };
    let (small, large) = (reads(64), reads(256));
    assert!(
        large < 5 * small,
        "{small} reads for 64 extents, {large} for 256"
    );
}

fn rr_read(bytes: &[u8], path: &[u8]) -> Vec<u8> {
    use hadris_fs::sync::FileSystem;
    let mut iso = MemDevice::new(bytes.to_vec(), common::SECTOR);
    let mut rr =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
    let node = rr.resolve(path, hadris_fs::Resolve::Lexical).unwrap();
    common::read_node(&mut rr, node).unwrap()
}

fn latin1_tree() -> hadris_fs::Tree {
    let mut tree = hadris_fs::Tree::new();
    tree.insert(&b"caf\xe9.txt"[..], Node::file(Content::bytes("one")))
        .unwrap();
    tree.insert(&b"r\xe9p/\xff"[..], Node::file(Content::bytes("two")))
        .unwrap();
    tree
}

#[test]
fn sessions_rewrite_latin1_rock_ridge_names() {
    let opts = IsoOptions::default().with_rock_ridge().with_joliet();
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        let mut session = Session::open(grown(image(&latin1_tree(), &opts))).unwrap();
        assert!(session.tree().get(&b"/caf\xe9.txt"[..]).is_some());
        session
            .tree_mut()
            .replace(&b"caf\xe9.txt"[..], Node::file(Content::bytes("uno")))
            .unwrap();
        session
            .tree_mut()
            .insert(&b"r\xe9p/na\xefve"[..], Node::file(Content::bytes("new")))
            .unwrap();
        let report = session.write(&opts, mode).unwrap();
        assert!(report.extents(b"/r\xe9p/na\xefve").is_some(), "{mode:?}");
        let bytes = session.into_inner().into_inner();
        assert_eq!(rr_read(&bytes, b"/caf\xe9.txt"), b"uno", "{mode:?}");
        assert_eq!(rr_read(&bytes, b"/r\xe9p/\xff"), b"two", "{mode:?}");
        assert_eq!(rr_read(&bytes, b"/r\xe9p/na\xefve"), b"new", "{mode:?}");
    }
}

#[test]
fn kept_catalogs_follow_boot_images_with_latin1_names() {
    let tree = sample(false, false);
    let mut bytes = grown(image(&tree, &options())).into_inner();
    let from = b"NM\x0d\x01\x00boot.img";
    let at: Vec<_> = (0..bytes.len() - from.len())
        .filter(|&i| &bytes[i..i + from.len()] == from)
        .collect();
    assert_eq!(at.len(), 1);
    bytes[at[0] + 5..at[0] + 13].copy_from_slice(b"b\xf4\xf4t.img");
    let mut session = Session::open(MemDevice::new(bytes, common::SECTOR)).unwrap();
    assert!(session.tree().get(&b"boot/b\xf4\xf4t.img"[..]).is_some());
    let replaced = vec![0x5Au8; 4096];
    session
        .tree_mut()
        .replace(
            &b"boot/b\xf4\xf4t.img"[..],
            Node::file(Content::bytes(replaced.clone())),
        )
        .unwrap();
    let opts = session.options();
    let report = session.write(&opts, SessionMode::Rewrite).unwrap();
    let moved = report
        .extents(b"boot/b\xf4\xf4t.img")
        .map(|e| e[0])
        .unwrap();
    let mut iso = session.into_inner();
    let catalog = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .catalog()
        .unwrap()
        .unwrap();
    assert_eq!(
        u64::from(catalog.entries()[0].load_block()) * 2048,
        moved.offset()
    );
    assert_eq!(rr_read(iso.get_ref(), b"/boot/b\xf4\xf4t.img"), replaced);
}

#[cfg(feature = "async")]
#[test]
fn async_sessions_rewrite_latin1_names_alike() {
    let opts = IsoOptions::default().with_rock_ridge();
    let dev = grown(image(&latin1_tree(), &opts));
    let mut sync_session =
        Session::open(MemDevice::new(dev.get_ref().clone(), common::SECTOR)).unwrap();
    sync_session
        .tree_mut()
        .insert(&b"r\xe9p/na\xefve"[..], Node::file(Content::bytes("new")))
        .unwrap();
    sync_session.write(&opts, SessionMode::Rewrite).unwrap();
    let expected = sync_session.into_inner().into_inner();
    assert_eq!(rr_read(&expected, b"/r\xe9p/na\xefve"), b"new");
    common::block_on(async {
        let mut session = hadris_iso::async_::Session::open(dev).await.unwrap();
        session
            .tree_mut()
            .insert(&b"r\xe9p/na\xefve"[..], Node::file(Content::bytes("new")))
            .unwrap();
        session.write(&opts, SessionMode::Rewrite).await.unwrap();
        assert_eq!(session.into_inner().into_inner(), expected);
    });
}

fn identity_tree() -> hadris_fs::Tree {
    use hadris_fs::FileType;

    let mut tree = hadris_fs::Tree::new();
    let target = (0..220)
        .map(|i| format!("{i:0200}"))
        .collect::<Vec<_>>()
        .join("/");
    for (name, node) in [
        ("empty", Node::file(Content::empty())),
        ("file", Node::file(Content::bytes("shared"))),
        ("symlink", Node::symlink(target)),
        (
            "device",
            Node::special(
                FileType::CharDevice,
                Some(hadris_fs::DeviceNumber::new(1, 3)),
            ),
        ),
    ] {
        tree.insert(name, node).unwrap();
        tree.link(name, format!("{name}-alias")).unwrap();
    }
    tree.insert("independent", Node::file(Content::bytes("shared")))
        .unwrap();
    tree.insert("independent-empty", Node::file(Content::empty()))
        .unwrap();
    tree
}

fn check_identity_tree(tree: &hadris_fs::Tree) {
    let expected = identity_tree();
    for name in ["empty", "file", "symlink", "device"] {
        let entry = tree.entry(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(
            entry.id(),
            tree.entry(format!("{name}-alias")).unwrap().id(),
            "{name}"
        );
        assert_eq!(entry.links(), 2, "{name}");
        let actual = tree.get(name).unwrap();
        let expected = expected.get(name).unwrap();
        assert_eq!(actual.file_type(), expected.file_type(), "{name}");
        assert_eq!(actual.target(), expected.target(), "{name}");
        assert_eq!(actual.device(), expected.device(), "{name}");
    }
    for (a, b) in [("file", "independent"), ("empty", "independent-empty")] {
        assert_ne!(tree.entry(a).unwrap().id(), tree.entry(b).unwrap().id());
    }
}

#[test]
fn sessions_preserve_long_symlinks_and_node_identity() {
    let opts = IsoOptions::new().with_rock_ridge();
    let mut session = Session::open(image(&identity_tree(), &opts)).unwrap();
    check_identity_tree(session.tree());
    let mut out = MemDevice::new(vec![0; 1 << 20], common::SECTOR);
    session.export(&mut out, &opts).unwrap();
    check_identity_tree(Session::open(out).unwrap().tree());
}

#[cfg(feature = "async")]
#[test]
fn async_sessions_preserve_long_symlinks_and_node_identity() {
    let opts = IsoOptions::new().with_rock_ridge();
    let dev = image(&identity_tree(), &opts);
    common::block_on(async {
        let mut session = hadris_iso::async_::Session::open(dev).await.unwrap();
        check_identity_tree(session.tree());
        let mut out = MemDevice::new(vec![0; 1 << 20], common::SECTOR);
        session.export(&mut out, &opts).await.unwrap();
        check_identity_tree(hadris_iso::async_::Session::open(out).await.unwrap().tree());
    });
}

#[test]
fn independent_files_sharing_extents_keep_their_identity() {
    let opts = IsoOptions::new().with_rock_ridge();
    let mut session = Session::open(grown(image(&identity_tree(), &opts))).unwrap();
    let content = session
        .tree()
        .get("file")
        .unwrap()
        .content()
        .unwrap()
        .clone();
    session
        .tree_mut()
        .replace("independent", Node::file(content))
        .unwrap();
    let report = session.write(&opts, SessionMode::Rewrite).unwrap();
    assert_eq!(report.extents("file"), report.extents("independent"));
    check_identity_tree(Session::open(session.into_inner()).unwrap().tree());
}

fn catalog_tree() -> hadris_fs::Tree {
    let mut tree = hadris_fs::Tree::new();
    tree.insert("BOOT.IMG", Node::file(Content::bytes(vec![0x90; 2048])))
        .unwrap();
    tree
}

fn boot_options(count: usize) -> IsoOptions {
    let mut boot = ElTorito::new();
    for _ in 0..count {
        boot = boot.with_entry(BootEntry::bios("BOOT.IMG"));
    }
    IsoOptions::new().with_el_torito(boot)
}

fn check_boot_entries(dev: MemDevice<Vec<u8>>, count: usize, block: u32, value: u8, len: usize) {
    let mut iso = IsoFs::mount(dev, MountOptions::new()).unwrap();
    let mut bytes = vec![0; 64 * 1024];
    let catalog = iso.boot_catalog(&mut bytes).unwrap().unwrap();
    let entries: Vec<_> = catalog.entries().collect();
    assert_eq!(entries.len(), count);
    for entry in entries {
        assert_eq!(entry.load_block(), block);
        assert_eq!(usize::from(entry.sector_count()), len.div_ceil(512));
    }
    assert_eq!(iso.read_to_vec("/BOOT.IMG").unwrap(), vec![value; len]);
}

#[test]
fn new_and_large_catalogs_follow_successive_boot_image_replacements() {
    for mode in [SessionMode::Append, SessionMode::Rewrite] {
        for count in [1, 260] {
            let initial = if count == 1 {
                IsoOptions::new()
            } else {
                boot_options(count)
            };
            let mut session =
                Session::open(grown(grown(grown(image(&catalog_tree(), &initial))))).unwrap();
            if count == 1 {
                session.write(&boot_options(count), mode).unwrap();
            }
            let mut block = 0;
            for (value, len) in [(0xA5, 4096), (0x5A, 1536)] {
                session
                    .tree_mut()
                    .replace("BOOT.IMG", Node::file(Content::bytes(vec![value; len])))
                    .unwrap();
                let report = session.write(&session.options(), mode).unwrap();
                block = (report.extents("BOOT.IMG").unwrap()[0].offset() / 2048) as u32;
            }
            check_boot_entries(session.into_inner(), count, block, 0x5A, 1536);
        }
    }
}

#[cfg(feature = "async")]
#[test]
fn async_new_and_large_catalogs_follow_successive_boot_image_replacements() {
    common::block_on(async {
        for mode in [SessionMode::Append, SessionMode::Rewrite] {
            for count in [1, 260] {
                let initial = if count == 1 {
                    IsoOptions::new()
                } else {
                    boot_options(count)
                };
                let mut session = hadris_iso::async_::Session::open(grown(grown(grown(image(
                    &catalog_tree(),
                    &initial,
                )))))
                .await
                .unwrap();
                if count == 1 {
                    session.write(&boot_options(count), mode).await.unwrap();
                }
                let mut block = 0;
                for (value, len) in [(0xA5, 4096), (0x5A, 1536)] {
                    session
                        .tree_mut()
                        .replace("BOOT.IMG", Node::file(Content::bytes(vec![value; len])))
                        .unwrap();
                    let report = session.write(&session.options(), mode).await.unwrap();
                    block = (report.extents("BOOT.IMG").unwrap()[0].offset() / 2048) as u32;
                }
                check_boot_entries(session.into_inner(), count, block, 0x5A, 1536);
            }
        }
    });
}

#[test]
fn sessions_reject_catalogs_beyond_the_parser_limit() {
    let dev = image(&catalog_tree(), &boot_options(600));
    let err = Session::open(dev).unwrap_err();
    assert_eq!(err.kind(), hadris_fs::ErrorKind::Corrupt);
}

#[cfg(feature = "async")]
#[test]
fn async_sessions_reject_catalogs_beyond_the_parser_limit() {
    let dev = image(&catalog_tree(), &boot_options(600));
    common::block_on(async {
        let err = hadris_iso::async_::Session::open(dev).await.unwrap_err();
        assert_eq!(err.kind(), hadris_fs::ErrorKind::Corrupt);
    });
}
