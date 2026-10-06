//! Images the writer makes read back through every tree of the reader.

mod common;

use common::{IsoExtras, Paths};
use common::{image, pattern, sample};
use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, Field, Node, Tree, WarningKind};
use hadris_fs::{DeviceNumber, ErrorKind, FileType, Permissions};
use hadris_iso::sync::IsoFs;
use hadris_iso::{
    AppendedPartition, BootEntry, BootInfo, ElTorito, Emulation, Hybrid, IsoDate, IsoId, IsoLevel,
    IsoOptions, JolietLevel, NameCase, Namespace, Platform,
};

fn full() -> IsoOptions {
    IsoOptions::default()
        .with_id(IsoId::Volume, "ROUNDTRIP")
        .with_id(IsoId::Publisher, "hadris")
        .with_level(IsoLevel::L2)
        .with_joliet()
        .with_rock_ridge()
        .with_iso1999()
}

#[test]
fn every_tree_reads_back() {
    let tree = sample(true, true);
    let mut iso = image(&tree, &full());
    let namespaces = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .namespaces();
    assert!(namespaces.contains(Namespace::RockRidge));
    assert_eq!(namespaces.joliet_level(), Some(JolietLevel::L3));
    assert!(namespaces.contains(Namespace::Enhanced));
    assert_eq!(namespaces.preferred(), Namespace::RockRidge);

    for ns in [
        Namespace::RockRidge,
        Namespace::Joliet,
        Namespace::Enhanced,
        Namespace::Primary,
    ] {
        let mut view = IsoFs::mount_namespace(&mut iso, MountOptions::new(), ns).unwrap();
        let big = if ns == Namespace::Primary {
            "/DOCS/BIG.BIN"
        } else {
            "/docs/big.bin"
        };
        assert_eq!(view.read_to_vec(big).unwrap(), pattern(100_000), "{ns:?}");
        assert_eq!(
            view.read_to_vec("/readme.txt").unwrap(),
            b"hello world\n",
            "{ns:?}"
        );
    }

    let mut rr =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
    assert_eq!(
        rr.read_to_vec("/a/b/c/d/e/f/g/h/i/deep.txt").unwrap(),
        b"deep"
    );
    assert_eq!(
        rr.read_to_vec("/Long Name With Spaces é.txt").unwrap(),
        b"long"
    );
    let readme = rr.resolve_path("/readme.txt").unwrap();
    let meta = rr.stat(readme).unwrap();
    assert_eq!(meta.permissions(), Permissions::new(0o600));
    assert_eq!(meta.owner(), Some(hadris_fs::Owner::new(1000, 100)));
    assert_eq!(meta.nlink(), 2);
    assert_eq!(meta.modified().unwrap().unix_seconds(), 1_700_000_000);
    let hard = rr.resolve_path("/docs/hard.txt").unwrap();
    let info = rr.rock_ridge(hard).unwrap().unwrap();
    assert_eq!(
        info.serial(),
        rr.rock_ridge(readme).unwrap().unwrap().serial()
    );
    let link = rr.resolve_path("/docs/link").unwrap();
    assert_eq!(rr.stat(link).unwrap().file_type(), FileType::Symlink);
    let mut target = [0u8; 64];
    assert_eq!(rr.readlink(link, &mut target).unwrap(), b"../readme.txt");
    let dev = rr.resolve_path("/dev/null").unwrap();
    assert_eq!(rr.stat(dev).unwrap().file_type(), FileType::CharDevice);
    assert_eq!(
        rr.rock_ridge(dev).unwrap().unwrap().device(),
        Some(DeviceNumber::new(1, 3))
    );
    assert!(!rr.exists("/rr_moved/RRD000001").unwrap());

    let deep = rr.resolve_path("/a/b/c/d/e/f/g/h").unwrap();
    let parent = rr.parent(deep).unwrap();
    assert_eq!(parent, rr.resolve_path("/a/b/c/d/e/f/g").unwrap());

    let mut joliet =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Joliet).unwrap();
    assert_eq!(
        joliet.read_to_vec("/Long Name With Spaces é.txt").unwrap(),
        b"long"
    );
    assert!(!joliet.exists("/docs/link").unwrap());
    assert!(!joliet.exists("/rr_moved").unwrap());
    assert_eq!(
        joliet.read_to_vec("/a/b/c/d/e/f/g/h/i/deep.txt").unwrap(),
        b"deep"
    );

    let mut primary =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
    assert_eq!(
        primary.read_to_vec("/readme.txt").unwrap(),
        b"hello world\n"
    );
    assert_eq!(
        primary.capabilities().case(),
        hadris_fs::CaseRule::Insensitive
    );
}

fn names<D: FileSystem>(view: &mut D, path: &str) -> Vec<String> {
    let mut names = view.names(path).unwrap();
    names.sort();
    names
}

#[test]
fn relocation_reuses_a_root_directory_of_that_name() {
    let mut tree = sample(true, true);
    tree.insert("rr_moved/user.txt", Node::file(Content::bytes("user")))
        .unwrap();
    tree.insert(
        "rr_moved/RRD000001/inner.txt",
        Node::file(Content::bytes("inner")),
    )
    .unwrap();
    let mut iso = image(&tree, &full());
    for ns in [Namespace::RockRidge, Namespace::Joliet] {
        let mut view = IsoFs::mount_namespace(&mut iso, MountOptions::new(), ns).unwrap();
        assert_eq!(
            view.read_to_vec("/a/b/c/d/e/f/g/h/i/deep.txt").unwrap(),
            b"deep",
            "{ns:?}"
        );
        assert_eq!(
            view.read_to_vec("/rr_moved/user.txt").unwrap(),
            b"user",
            "{ns:?}"
        );
        assert_eq!(
            view.read_to_vec("/rr_moved/RRD000001/inner.txt").unwrap(),
            b"inner",
            "{ns:?}"
        );
        assert_eq!(
            names(&mut view, "/rr_moved"),
            ["RRD000001", "user.txt"],
            "{ns:?}"
        );
    }
    let mut primary =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
    assert_eq!(names(&mut primary, "/RR_MOVED").len(), 3);
}

#[test]
fn listings_resume_and_skip_dots() {
    let tree = sample(false, false);
    let mut iso = image(&tree, &IsoOptions::default());
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Preferred).unwrap();
    assert_eq!(view.namespace(), Namespace::Primary);
    let root = view.root();
    let mut cursor = hadris_fs::DirCursor::START;
    let mut names = Vec::new();
    while let Some(entry) = view.readdir(root, cursor).unwrap() {
        cursor = entry.next_cursor();
        names.push(String::from_utf8(entry.name().as_bytes().to_vec()).unwrap());
    }
    assert_eq!(
        names,
        ["BOOT", "DOCS", "EMPTY", "LONG_NAM.TXT", "README.TXT"]
    );
    assert_eq!(view.read_to_vec("/docs/empty.txt").unwrap(), b"");
    assert_eq!(
        view.lookup(root, hadris_fs::Name::new("missing"))
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn reports_match_what_the_reader_finds() {
    let tree = sample(true, true);
    let options = full();
    let report = hadris_iso::plan(&tree, &options).unwrap();
    let mut iso = image(&tree, &options);
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
    for path in [
        "/readme.txt",
        "/docs/big.bin",
        "/docs/hard.txt",
        "/boot/efi.img",
    ] {
        let node = view.resolve_path(path).unwrap();
        let extents = view.all_extents(node);
        assert_eq!(
            report.extents(path).map(|e| e[0]),
            Some(extents[0]),
            "{path}"
        );
    }
    assert_eq!(report.extents("docs/empty.txt").map(|e| e[0]), None);
    let relocated: Vec<_> = report
        .warnings()
        .iter()
        .map(|w| (w.kind(), w.path()))
        .collect();
    assert_eq!(
        relocated,
        [(WarningKind::Relocated, Some(&b"a/b/c/d/e/f/g/h"[..]))]
    );
    assert_eq!(
        report.size() / 2048,
        u64::from(
            IsoFs::mount(&mut iso, MountOptions::new())
                .unwrap()
                .info()
                .volume_space_size()
        )
    );
}

#[test]
fn trees_without_rock_ridge_report_what_they_drop() {
    let tree = sample(false, true);
    let report = hadris_iso::plan(&tree, &IsoOptions::default()).unwrap();
    let kinds: Vec<_> = report.warnings().iter().map(|w| w.kind()).collect();
    assert!(kinds.contains(&WarningKind::Skipped));
    assert!(kinds.contains(&WarningKind::Dropped(Field::Permissions)));
    assert!(kinds.contains(&WarningKind::Dropped(Field::Owner)));
    assert!(kinds.contains(&WarningKind::Renamed));
    let deep = sample(true, false);
    let err = hadris_iso::plan(&deep, &IsoOptions::default()).unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.detail().and_then(hadris_iso::Detail::from_code)
        ),
        (
            ErrorKind::InvalidInput,
            Some(hadris_iso::Detail::Relocation)
        )
    );
}

#[test]
fn lowercase_names_are_kept_on_request() {
    let tree = sample(false, false);
    let options = IsoOptions::default().with_name_case(NameCase::Preserve);
    let mut iso = image(&tree, &options);
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
    assert!(view.exists("/readme.txt").unwrap());
    let node = view.resolve_path("/readme.txt").unwrap();
    assert_eq!(view.record(node).name(), b"readme.txt;1");
}

#[test]
fn boot_catalogs_read_back() {
    let tree = sample(false, false);
    let options = IsoOptions::default().with_el_torito(
        ElTorito::new()
            .with_entry(
                BootEntry::bios("boot/boot.img")
                    .with_load_size(4)
                    .with_boot_info(BootInfo::Table),
            )
            .with_entry(BootEntry::uefi("boot/efi.img"))
            .with_catalog_path("boot/boot.cat"),
    );
    let report = hadris_iso::plan(&tree, &options).unwrap();
    let mut iso = image(&tree, &options);
    let catalog = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .catalog()
        .unwrap()
        .unwrap();
    assert_eq!(
        Some(catalog.block()),
        IsoFs::mount(&mut iso, MountOptions::new())
            .unwrap()
            .catalog()
            .unwrap()
            .map(|c| c.block())
    );
    assert_eq!(
        report
            .extents("boot/boot.cat")
            .map(|e| e[0])
            .unwrap()
            .offset(),
        u64::from(catalog.block()) * 2048
    );
    let entries = catalog.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        (
            entries[0].platform(),
            entries[0].emulation(),
            entries[0].sector_count()
        ),
        (Platform::X86, Some(Emulation::NoEmulation), 4)
    );
    assert_eq!(
        (
            entries[1].platform(),
            entries[1].section(),
            entries[1].sector_count()
        ),
        (Platform::Efi, Some(0), 16)
    );
    assert!(entries.iter().all(|entry| entry.is_bootable()));
    assert_eq!(
        u64::from(entries[1].load_block()) * 2048,
        report
            .extents("boot/efi.img")
            .map(|e| e[0])
            .unwrap()
            .offset()
    );

    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
    let boot = view.read_to_vec("/boot/boot.img").unwrap();
    let table: [u8; 16] = boot[8..24].try_into().unwrap();
    assert_eq!(u32::from_le_bytes(table[..4].try_into().unwrap()), 16);
    assert_eq!(
        u64::from(u32::from_le_bytes(table[4..8].try_into().unwrap())) * 2048,
        report
            .extents("boot/boot.img")
            .map(|e| e[0])
            .unwrap()
            .offset()
    );
    assert_eq!(u32::from_le_bytes(table[8..12].try_into().unwrap()), 4096);
    let sum = boot[64..].chunks_exact(4).fold(0u32, |sum, w| {
        sum.wrapping_add(u32::from_le_bytes(w.try_into().unwrap()))
    });
    assert_eq!(u32::from_le_bytes(table[12..16].try_into().unwrap()), sum);
}

#[cfg(feature = "async")]
#[test]
fn async_modes_read_and_write_alike() {
    use hadris_fs::Resolve;
    let tree = sample(true, true);
    let options = full();
    let sync_image = image(&tree, &options).into_inner();
    common::block_on(async {
        let size = hadris_iso::plan(&tree, &options).unwrap().size();
        let mut dev = hadris_storage::MemDevice::new(vec![0u8; size as usize], common::SECTOR);
        hadris_iso::r#async::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &sync_image);
        let mut dev = hadris_storage::MemDevice::new(vec![0u8; size as usize], common::SECTOR);
        hadris_iso::r#async::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &sync_image);

        let mut view = hadris_iso::r#async::IsoFs::mount(dev, MountOptions::new())
            .await
            .unwrap();
        use hadris_fs::r#async::FileSystem as _;
        let big = view
            .resolve(b"/docs/big.bin", Resolve::Lexical)
            .await
            .unwrap();
        view.open(big, hadris_fs::OpenMode::Read).await.unwrap();
        let mut data = vec![0u8; 100_001];
        let mut len = 0;
        loop {
            let n = view.read(big, len as u64, &mut data[len..]).await.unwrap();
            if n == 0 {
                break;
            }
            len += n;
        }
        view.close(big).await.unwrap();
        assert_eq!(&data[..len], pattern(100_000));
        let linked = view
            .resolve(b"/docs/hard.txt", Resolve::Lexical)
            .await
            .unwrap();
        let target = view
            .resolve(b"/readme.txt", Resolve::Lexical)
            .await
            .unwrap();
        assert_eq!(linked, target);
    });
}

#[test]
fn hard_links_share_one_node_id() {
    let tree = sample(true, true);
    let mut iso = image(&tree, &full());
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
    let target = view.resolve_path("/readme.txt").unwrap();
    assert_eq!(view.resolve_path("/docs/hard.txt").unwrap(), target);
    assert_ne!(view.resolve_path("/docs/big.bin").unwrap(), target);
    assert_eq!(view.metadata("/docs/hard.txt").unwrap().nlink(), 2);
    let mut primary =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
    assert_ne!(
        primary.resolve_path("/README.TXT").unwrap(),
        primary.resolve_path("/DOCS/HARD.TXT").unwrap()
    );
}

#[test]
fn smaller_device_blocks_hold_images() {
    let tree = sample(false, false);
    let options = IsoOptions::default();
    let bytes = image(&tree, &options).into_inner();
    let mut dev = hadris_storage::MemDevice::new(
        vec![0u8; bytes.len()],
        hadris_storage::BlockSize::new(512).unwrap(),
    );
    hadris_iso::sync::write(&mut dev, &tree, &options).unwrap();
    assert_eq!(dev.get_ref(), &bytes);
    let mut iso = dev;
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
    assert_eq!(view.read_to_vec("/docs/big.bin").unwrap(), pattern(100_000));
}

#[test]
fn supplementary_escape_sequences_are_zero_padded() {
    let tree = sample(true, true);
    let mut iso = image(&tree, &full());
    let mut index = 0;
    let mut seen = 0;
    while let Some(descriptor) = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .descriptor(index)
        .unwrap()
    {
        index += 1;
        if let hadris_iso::raw::VolumeDescriptor::Supplementary(svd) = descriptor {
            let escapes = svd.escape_sequences;
            let used = if svd.is_enhanced() { 0 } else { 3 };
            assert!(
                escapes[used..].iter().all(|&byte| byte == 0),
                "ECMA-119 8.5.6 sets unused escape bytes to (00): {escapes:?}"
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 2, "a Joliet and an enhanced descriptor");
}

#[test]
fn primary_names_keep_the_separator_and_split_at_the_last_dot() {
    let mut tree = Tree::new();
    tree.insert("README", Node::file(Content::bytes("r")))
        .unwrap();
    tree.insert("x.tar.gz", Node::file(Content::bytes("x")))
        .unwrap();
    for level in [IsoLevel::L1, IsoLevel::L2] {
        let options = IsoOptions::default().with_level(level);
        let mut iso = image(&tree, &options);
        let mut view =
            IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Primary).unwrap();
        let readme = view.resolve_path("/README").unwrap();
        assert_eq!(view.record(readme).name(), b"README.;1");
        let tarball = view.resolve_path("/X_TAR.GZ").unwrap();
        assert_eq!(view.record(tarball).name(), b"X_TAR.GZ;1");
        assert_eq!(view.read_to_vec("/README").unwrap(), b"r");
    }
}

#[test]
fn joliet_reports_the_names_it_changes() {
    let mut tree = Tree::new();
    let long = "n".repeat(70);
    for name in [
        "a*b?c:d",
        "emoji\u{1F600}.txt",
        "Makefile",
        "makefile",
        long.as_str(),
        "plain.txt",
    ] {
        tree.insert(name, Node::file(Content::bytes("x"))).unwrap();
    }
    let options = IsoOptions::default().with_joliet();
    let report = hadris_iso::plan(&tree, &options).unwrap();
    let mut warned: Vec<_> = report
        .warnings()
        .iter()
        .inspect(|w| assert_eq!(w.kind(), WarningKind::Renamed))
        .map(|w| String::from_utf8(w.path().unwrap().to_vec()).unwrap())
        .collect();
    warned.sort();
    assert_eq!(
        warned,
        [
            "a*b?c:d".to_string(),
            "emoji\u{1F600}.txt".to_string(),
            "makefile".to_string(),
            long.clone(),
        ]
    );
    let mut iso = image(&tree, &options);
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::Joliet).unwrap();
    assert_eq!(view.read_to_vec("/a_b_c_d").unwrap(), b"x");
    assert_eq!(view.read_to_vec("emoji_.txt").unwrap(), b"x");
}

#[test]
fn the_tree_and_the_seed_or_the_time_decide_the_gpt_guids() {
    let tree = sample(false, false);
    let options = IsoOptions::default()
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("boot/boot.img"))
                .with_entry(BootEntry::uefi("boot/efi.img")),
        )
        .with_hybrid(hadris_iso::Hybrid::gpt());
    let guid_of = |tree: &Tree, options: &IsoOptions| {
        image(tree, options).into_inner()[512 + 56..512 + 72].to_vec()
    };
    let disk_guid = |options: &IsoOptions| guid_of(&tree, options);
    let default = disk_guid(&options);
    let mut other = sample(false, false);
    other
        .insert("extra.txt", Node::file(Content::bytes(*b"x")))
        .unwrap();
    assert_ne!(guid_of(&other, &options), default);
    assert_ne!(
        guid_of(&other, &options.clone().with_seed(7)),
        disk_guid(&options.clone().with_seed(7))
    );
    assert_eq!(disk_guid(&options), default);
    let later = hadris_fs::DateTime::from_unix_seconds(1_700_000_000).unwrap();
    assert_ne!(disk_guid(&options.clone().with_time(later)), default);
    let seeded = disk_guid(&options.clone().with_seed(7));
    assert_ne!(seeded, default);
    assert_eq!(
        disk_guid(&options.clone().with_seed(7).with_time(later)),
        seeded
    );
}

#[test]
fn identifiers_and_dates_reach_the_descriptors() {
    let tree = sample(false, false);
    let created = hadris_fs::DateTime::from_unix_seconds(1_600_000_000).unwrap();
    let expires = hadris_fs::DateTime::from_unix_seconds(1_900_000_000).unwrap();
    let options = IsoOptions::new()
        .with_id(IsoId::System, "LINUX")
        .with_id(IsoId::Volume, "IDS")
        .with_id(IsoId::VolumeSet, "SET")
        .with_id(IsoId::Publisher, "PUB")
        .with_id(IsoId::Preparer, "PREP")
        .with_id(IsoId::Application, "")
        .with_id(IsoId::CopyrightFile, "COPYING.TXT;1")
        .with_id(IsoId::AbstractFile, "ABSTRACT.TXT;1")
        .with_id(IsoId::BibliographicFile, "BIB.TXT;1")
        .with_date(IsoDate::Created, created)
        .with_date(IsoDate::Expires, expires)
        .with_joliet();
    assert_eq!(options.id(IsoId::Application), None);
    assert_eq!(options.date(IsoDate::Modified), Some(options.time()));
    assert_eq!(options.date(IsoDate::Effective), None);
    let bytes = image(&tree, &options).into_inner();
    let pvd = &bytes[16 * 2048..17 * 2048];
    let field = |range: core::ops::Range<usize>| {
        String::from_utf8(pvd[range].to_vec())
            .unwrap()
            .trim_end()
            .to_string()
    };
    assert_eq!(field(8..40), "LINUX");
    assert_eq!(field(40..72), "IDS");
    assert_eq!(field(190..318), "SET");
    assert_eq!(field(318..446), "PUB");
    assert_eq!(field(446..574), "PREP");
    assert_eq!(field(574..702), "");
    assert_eq!(field(702..739), "COPYING.TXT;1");
    assert_eq!(field(739..776), "ABSTRACT.TXT;1");
    assert_eq!(field(776..813), "BIB.TXT;1");
    assert_eq!(&pvd[813..829], b"2020091312264000");
    assert_eq!(&pvd[830..846], b"1980010100000000");
    assert_eq!(&pvd[847..863], b"2030031717464000");
    assert_eq!(&pvd[864..880], b"0000000000000000");
    let joliet = &bytes[17 * 2048..18 * 2048];
    assert_eq!(&joliet[40..48], &[0, b'I', 0, b'D', 0, b'S', 0, b' ']);
    assert_eq!(&joliet[813..829], b"2020091312264000");

    let long = IsoOptions::new().with_id(IsoId::CopyrightFile, &"C".repeat(38));
    let err = hadris_iso::plan(&tree, &long).unwrap_err();
    assert_eq!(
        err.detail().and_then(hadris_iso::Detail::from_code),
        Some(hadris_iso::Detail::Identifier)
    );
}

#[test]
fn an_appended_esp_is_stored_once_for_el_torito_and_the_gpt() {
    let tree = sample(false, false);
    let esp = pattern(6000).into_iter().rev().collect::<Vec<u8>>();
    let options = IsoOptions::new()
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("boot/boot.img"))
                .with_entry(BootEntry::uefi_appended(0)),
        )
        .with_hybrid(
            Hybrid::gpt_hybrid_mbr()
                .with_appended(AppendedPartition::esp(Content::bytes(esp.clone()))),
        );
    let report = hadris_iso::plan(&tree, &options).unwrap();
    let mut iso = image(&tree, &options);
    let catalog = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .catalog()
        .unwrap()
        .unwrap();
    let entries = catalog.entries();
    assert_eq!(entries[1].platform(), Platform::Efi);
    assert_eq!(entries[1].sector_count(), 12);
    let start = u64::from(entries[1].load_block()) * 2048;
    let bytes = iso.into_inner();
    assert_eq!(&bytes[start as usize..start as usize + esp.len()], &esp[..]);
    assert!(start + esp.len() as u64 <= report.size());
    assert!(
        report
            .files()
            .flat_map(|(_, extents)| extents)
            .all(|extent| extent.end() <= start || extent.offset() >= start + 6144)
    );

    let disk = hadris_part::sync::read(&mut hadris_storage::MemDevice::new(
        bytes,
        hadris_storage::BlockSize::new(512).unwrap(),
    ))
    .unwrap();
    let esp_part = disk
        .partitions()
        .find(|p| p.kind() == hadris_part::PartitionKind::Gpt(hadris_part::gpt::types::EFI_SYSTEM))
        .unwrap();
    assert_eq!(esp_part.start() * 512, start);
    assert_eq!(esp_part.len() * 512, 6144);
}

#[test]
fn min_image_blocks_pads_the_volume_and_moves_the_backup_gpt() {
    let tree = sample(false, false);
    for (hybrid, gpt) in [
        (None, false),
        (Some(Hybrid::mbr()), false),
        (Some(Hybrid::gpt()), true),
    ] {
        let mut options = IsoOptions::default();
        if let Some(hybrid) = hybrid.clone() {
            options = options.with_hybrid(hybrid);
        }
        let natural = hadris_iso::plan(&tree, &options).unwrap().size() / 2048;
        let blocks = natural + 300;
        let options = options.with_min_image_blocks(blocks);
        let mut dev = image(&tree, &options);
        let info = *IsoFs::mount(&mut dev, MountOptions::new()).unwrap().info();
        assert_eq!(u64::from(info.volume_space_size()), blocks);
        let bytes = dev.into_inner();
        assert_eq!(bytes.len() as u64, blocks * 2048);
        assert!(
            bytes[natural as usize * 2048..bytes.len() - 9 * 2048]
                .iter()
                .all(|&b| b == 0)
        );
        if hybrid.is_none() {
            continue;
        }
        let disk = hadris_part::sync::read(&mut hadris_storage::MemDevice::new(
            bytes.as_slice(),
            hadris_storage::BlockSize::new(512).unwrap(),
        ))
        .unwrap();
        let end = disk
            .partitions()
            .map(|p| p.start() + p.len())
            .max()
            .unwrap();
        if gpt {
            let last = blocks * 4 - 1;
            assert_eq!(&bytes[last as usize * 512..][..8], b"EFI PART");
            assert_eq!(bytes[512 + 32..512 + 40], last.to_le_bytes());
            assert!(end > (blocks - 10) * 4);
        } else {
            assert_eq!(end, blocks * 4);
        }
    }
}

#[test]
fn extras_read_the_descriptor_catalog_and_records() {
    let tree = sample(false, false);
    let time = hadris_fs::DateTime::from_unix_seconds(1_700_000_000).unwrap();
    let options = IsoOptions::default()
        .with_id(IsoId::Volume, "EXTRAS")
        .with_id(IsoId::Publisher, "PUBLISHER")
        .with_time(time)
        .with_el_torito(
            ElTorito::new()
                .with_entry(BootEntry::bios("boot/boot.img").with_load_size(4))
                .with_entry(BootEntry::uefi("boot/efi.img")),
        );
    let report = hadris_iso::plan(&tree, &options).unwrap();
    let mut dev = image(&tree, &options);
    let mut iso = IsoFs::mount(&mut dev, MountOptions::new()).unwrap();
    let info = *iso.info();
    assert_eq!(info.block_size(), 2048);
    assert_eq!(u64::from(info.volume_space_size()) * 2048, report.size());
    assert_eq!(info.id(IsoId::Volume), b"EXTRAS");
    assert_eq!(info.id(IsoId::Publisher), b"PUBLISHER");
    assert_eq!(info.id(IsoId::CopyrightFile), b"");
    assert_eq!(
        info.date(IsoDate::Created).map(|t| t.unix_seconds()),
        Some(time.unix_seconds())
    );
    assert_eq!(info.date(IsoDate::Expires), None);

    let mut small = [0u8; 64];
    let err = iso.boot_catalog(&mut small).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
    let mut buf = [0u8; 2048];
    let catalog = iso.boot_catalog(&mut buf).unwrap().unwrap();
    let entries: Vec<_> = catalog.entries().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(catalog.default_entry(), entries[0]);
    assert_eq!(catalog.platform(), Platform::X86);
    let efi = iso.boot_image(&entries[1]);
    let stored = report.extents("boot/efi.img").unwrap()[0];
    assert_eq!(efi.offset(), stored.offset());
    let bios = iso.boot_image(&entries[0]);
    assert_eq!(bios.len(), 4 * 512);
    let mut loaded = vec![0u8; bios.len() as usize];
    iso.read_raw(bios.offset(), &mut loaded).unwrap();

    let node = iso.resolve_path("/readme.txt").unwrap();
    assert_eq!(iso.record(node).name(), b"README.TXT;1");
    let mut out = [hadris_fs::Extent::new(0, 0); 1];
    assert_eq!(iso.extents(node, 0, &mut out).unwrap(), 1);
    assert_eq!(out[0], report.extents("readme.txt").unwrap()[0]);
    assert_eq!(iso.extents(node, out[0].len(), &mut out).unwrap(), 0);
    assert_eq!(
        iso.records(node, &mut []).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}

/// Checks that every continuation area the system use area `su` chains to
/// stays inside one block, and returns how many there are.
fn continuation_areas(bytes: &[u8], su: &[u8]) -> usize {
    let word = |at: usize| u32::from_le_bytes(su[at..at + 4].try_into().unwrap()) as usize;
    let mut count = 0;
    let mut at = 0;
    while at + 4 <= su.len() && su[at + 2] >= 4 {
        if &su[at..at + 2] == b"CE" {
            let (block, offset, size) = (word(at + 4), word(at + 12), word(at + 20));
            assert!(offset + size <= 2048, "area at {offset} of {size} bytes");
            let area = block * 2048 + offset;
            count += 1 + continuation_areas(bytes, &bytes[area..area + size]);
        }
        at += su[at + 2] as usize;
    }
    count
}

#[test]
fn continuation_areas_stay_inside_their_block() {
    let names: Vec<String> = (0..12)
        .map(|i| format!("{}{i:02}", "n".repeat(198)))
        .collect();
    let target = vec!["t".repeat(200); 15].join("/");
    let mut tree = Tree::new();
    for name in &names {
        tree.insert(name, Node::file(Content::bytes(name.clone())))
            .unwrap();
    }
    tree.insert("link", Node::symlink(target.as_bytes()))
        .unwrap();
    let dev = image(&tree, &IsoOptions::default().with_rock_ridge());
    let bytes = dev.get_ref().clone();
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let root = word(16 * 2048 + 156 + 2) * 2048;
    let size = word(16 * 2048 + 156 + 10);
    let (mut at, mut areas) = (root, 0);
    while at < root + size {
        let len = bytes[at] as usize;
        if len == 0 {
            at = (at / 2048 + 1) * 2048;
            continue;
        }
        let id_len = bytes[at + 32] as usize;
        let su = at + 33 + id_len + (id_len + 1) % 2;
        areas += continuation_areas(&bytes, &bytes[su..at + len]);
        at += len;
    }
    assert!(areas > names.len(), "{areas} areas");

    let mut iso = dev;
    let mut rr =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
    let mut listed = rr.names("/").unwrap();
    listed.sort();
    let mut expected = names.clone();
    expected.push("link".into());
    expected.sort();
    assert_eq!(listed, expected);
    for name in &names {
        assert_eq!(
            rr.read_to_vec(&format!("/{name}")).unwrap(),
            name.as_bytes()
        );
    }
    let link = rr.resolve_path("/link").unwrap();
    let mut buf = vec![0u8; 4096];
    assert_eq!(rr.readlink(link, &mut buf).unwrap(), target.as_bytes());
}

#[test]
fn rock_ridge_names_too_long_to_list_fall_back_to_the_identifier() {
    let mut tree = Tree::new();
    for name in ["a", &"m".repeat(800), &"n".repeat(1100), "z"] {
        tree.insert(name, Node::file(Content::bytes(name))).unwrap();
    }
    let mut iso = image(&tree, &IsoOptions::default().with_rock_ridge());
    let mut view =
        IsoFs::mount_namespace(&mut iso, MountOptions::new(), Namespace::RockRidge).unwrap();
    let listed = view.names("/").unwrap();
    assert_eq!(listed.len(), 4, "{listed:?}");
    assert_eq!(listed[0], "a");
    assert_eq!(listed[3], "z");
    let root = view.root();
    let mut cursor = hadris_fs::DirCursor::START;
    while let Some(entry) = view.readdir(root, cursor).unwrap() {
        cursor = entry.next_cursor();
        assert_eq!(view.lookup(root, entry.name()).unwrap(), entry.node());
    }
    assert_eq!(view.read_to_vec("/z").unwrap(), b"z");
}

#[test]
fn rock_ridge_names_too_long_to_list_are_reported() {
    let mut tree = Tree::new();
    for name in ["a", &"m".repeat(768), &"n".repeat(769)] {
        tree.insert(name, Node::file(Content::bytes(name))).unwrap();
    }
    tree.insert("d", Node::dir()).unwrap();
    tree.insert(format!("d/{}", "p".repeat(1000)), Node::dir())
        .unwrap();
    let report = hadris_iso::plan(&tree, &IsoOptions::default().with_rock_ridge()).unwrap();
    let long: Vec<_> = report
        .warnings()
        .iter()
        .filter(|w| w.kind() == WarningKind::Renamed)
        .map(|w| w.path().unwrap().len())
        .collect();
    assert_eq!(long, [769, 2 + 1000]);
}

#[test]
fn rock_ridge_names_of_every_length_fit_their_records() {
    for len in 1..=255 {
        let name = "a".repeat(len);
        let mut tree = Tree::new();
        tree.insert(&name, Node::file(Content::bytes("x"))).unwrap();
        let options = IsoOptions::default().with_rock_ridge();
        let size = hadris_iso::plan(&tree, &options)
            .unwrap_or_else(|err| panic!("{len}: {err:?}"))
            .size();
        let mut dev = hadris_storage::MemDevice::new(vec![0u8; size as usize], common::SECTOR);
        hadris_iso::sync::write(&mut dev, &tree, &options)
            .unwrap_or_else(|err| panic!("{len}: {err:?}"));
        let mut view =
            IsoFs::mount_namespace(&mut dev, MountOptions::new(), Namespace::RockRidge).unwrap();
        assert_eq!(view.names("/").unwrap(), [name], "{len}");
    }
}

#[test]
fn directories_have_no_length() {
    let tree = sample(true, true);
    let mut iso = image(&tree, &full());
    for ns in [
        Namespace::Primary,
        Namespace::RockRidge,
        Namespace::Joliet,
        Namespace::Enhanced,
    ] {
        let mut view = IsoFs::mount_namespace(&mut iso, MountOptions::new(), ns).unwrap();
        let root = view.root();
        assert_eq!(view.stat(root).unwrap().len(), 0, "{ns:?}");
        let docs = view.metadata("/docs").unwrap();
        assert!(docs.file_type().is_dir());
        assert_eq!(docs.len(), 0, "{ns:?}");
    }
}

#[test]
fn path_tables_use_final_directory_identifiers() {
    use hadris_iso::raw::{DirectoryRecord, FileFlags};
    fn table(bytes: &[u8], start: usize, size: usize, big: bool) -> Vec<(Vec<u8>, u32, u16)> {
        let mut entries = Vec::new();
        let mut pos = start;
        while pos < start + size {
            let len = bytes[pos] as usize;
            let block = bytes[pos + 2..pos + 6].try_into().unwrap();
            let parent = bytes[pos + 6..pos + 8].try_into().unwrap();
            entries.push((
                bytes[pos + 8..pos + 8 + len].to_vec(),
                if big {
                    u32::from_be_bytes(block)
                } else {
                    u32::from_le_bytes(block)
                },
                if big {
                    u16::from_be_bytes(parent)
                } else {
                    u16::from_le_bytes(parent)
                },
            ));
            pos += 8 + len + len % 2;
        }
        assert_eq!(pos, start + size);
        entries
    }
    let mut tree = Tree::new();
    for i in 0..32 {
        tree.insert(
            format!("long directory number {i:04}/file"),
            Node::file(Content::bytes("data")),
        )
        .unwrap();
    }
    tree.insert("a/b/c/d/e/f/g/h/i/file", Node::file(Content::bytes("deep")))
        .unwrap();
    let bytes = image(&tree, &full().with_level(IsoLevel::L1)).into_inner();
    for descriptor in bytes[16 * 2048..]
        .chunks_exact(2048)
        .take_while(|sector| sector[0] != 255)
    {
        if !matches!(descriptor[0], 1 | 2) {
            continue;
        }
        let le = |at| u32::from_le_bytes(descriptor[at..at + 4].try_into().unwrap()) as usize;
        let size = le(132);
        let little = table(&bytes, le(140) * 2048, size, false);
        let big = table(
            &bytes,
            u32::from_be_bytes(descriptor[148..152].try_into().unwrap()) as usize * 2048,
            size,
            true,
        );
        assert_eq!(little, big);
        for (name, extent, parent) in little.iter().skip(1) {
            let start = little[usize::from(*parent) - 1].1 as usize * 2048;
            let dot = DirectoryRecord::parse(&bytes[start..start + 2048])
                .unwrap()
                .unwrap();
            let size = dot.header().data_len.get() as usize;
            let mut pos = 0;
            let mut matched = false;
            while pos < size {
                let end = (pos / 2048 + 1) * 2048;
                match DirectoryRecord::parse(&bytes[start + pos..start + end.min(size)]).unwrap() {
                    Some(record) => {
                        if record.header().file_flags().contains(FileFlags::DIRECTORY)
                            && record.header().extent.get() == *extent
                            && record.name() == name
                        {
                            matched = true;
                            break;
                        }
                        pos += record.len();
                    }
                    None => pos = end,
                }
            }
            assert!(
                matched,
                "path table identifier must match its parent's directory record"
            );
        }
    }
}

fn tree_shape(tree: &Tree) -> std::collections::BTreeMap<Vec<u8>, (FileType, u64)> {
    let mut shape = std::collections::BTreeMap::new();
    let mut pending = vec![(tree.root(), Vec::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for (name, entry) in dir.children() {
            let mut path = prefix.clone();
            path.push(b'/');
            path.extend_from_slice(name.as_bytes());
            let node = entry.node();
            shape.insert(
                path.clone(),
                (node.file_type(), node.content().map_or(0, Content::len)),
            );
            if node.file_type() == FileType::Dir {
                pending.push((entry, path));
            }
        }
    }
    shape
}

macro_rules! relocation_cases {
    ($mode:ident, $test:ident, $run:ident) => {
        #[test]
        fn $test() {
            $run!(async {
                use hadris_fs::$mode::FileSystem;
                use hadris_fs::{DirCursor, Name, OpenMode};
                use hadris_iso::Relocation;
                for (name, relocation) in [
                    ("rr_moved", Relocation::RrMoved),
                    (".rr_moved", Relocation::DotRrMoved),
                ] {
                    for user in 0..3 {
                        let mut tree = Tree::new();
                        tree.insert("a/b/c/d/e/f/g/h/i/leaf", Node::file(Content::bytes("deep")))
                            .unwrap();
                        if user > 0 {
                            tree.insert(name, Node::dir()).unwrap();
                        }
                        if user > 1 {
                            tree.insert(
                                format!("{name}/RRD000001/own"),
                                Node::file(Content::bytes("user")),
                            )
                            .unwrap();
                        }
                        let options = IsoOptions::new()
                            .with_rock_ridge()
                            .with_relocation(relocation);
                        let image = common::image(&tree, &options);
                        let mut fs = hadris_iso::$mode::IsoFs::mount(image, MountOptions::new())
                            .await
                            .unwrap();
                        let root = fs.root();
                        let mut names = Vec::new();
                        let mut cursor = DirCursor::START;
                        while let Some(entry) = fs.readdir(root, cursor).await.unwrap() {
                            cursor = entry.next_cursor();
                            names.push(entry.name().as_bytes().to_vec());
                        }
                        names.sort();
                        let expected: Vec<_> = tree
                            .root()
                            .children()
                            .map(|(name, _)| name.as_bytes().to_vec())
                            .collect();
                        assert_eq!(names, expected, "{name} user={user}");
                        if user == 0 {
                            assert_eq!(
                                fs.lookup(root, Name::new(name)).await.unwrap_err().kind(),
                                ErrorKind::NotFound
                            );
                        }
                        let leaf = fs
                            .resolve(b"/a/b/c/d/e/f/g/h/i/leaf", hadris_fs::Resolve::Lexical)
                            .await
                            .unwrap();
                        fs.open(leaf, OpenMode::Read).await.unwrap();
                        let mut data = [0; 4];
                        assert_eq!(fs.read(leaf, 0, &mut data).await.unwrap(), 4);
                        assert_eq!(&data, b"deep");
                        fs.close(leaf).await.unwrap();
                        let volume = hadris_fs::$mode::Volume::new(fs);
                        let extracted = hadris_fs::$mode::read_tree(&volume, "/").await.unwrap();
                        assert_eq!(tree_shape(&extracted), tree_shape(&tree));
                    }
                }
            });
        }
    };
}
macro_rules! relocation_sync { ($($body:tt)*) => { common::block_on(hadris_macros::strip_async! { $($body)* }) }; }
#[cfg(feature = "async")]
macro_rules! relocation_async {
    ($body:expr) => {
        common::block_on($body)
    };
}
relocation_cases!(
    sync,
    relocation_listing_preserves_only_the_logical_tree_sync,
    relocation_sync
);
#[cfg(feature = "async")]
relocation_cases!(
    r#async,
    relocation_listing_preserves_only_the_logical_tree_async,
    relocation_async
);
