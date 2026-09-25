//! Images the writer makes read back through every tree of the reader.

mod common;

use common::Paths;
use common::{image, pattern, sample};
use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, Field, Node, Tree, WarningKind};
use hadris_fs::{DeviceNumber, ErrorKind, FileType, Permissions, Resolve};
use hadris_iso::sync::IsoFs;
use hadris_iso::{
    BootEntry, BootInfo, ElTorito, Emulation, IsoLevel, IsoOptions, JolietLevel, NameCase,
    Namespace, Platform, RockRidge, VolumeIdentifiers,
};

fn full() -> IsoOptions {
    IsoOptions::default()
        .with_volume(VolumeIdentifiers::new("ROUNDTRIP").with_publisher("hadris"))
        .with_level(IsoLevel::L2)
        .with_joliet(JolietLevel::L3)
        .with_rock_ridge(RockRidge::default())
        .with_enhanced_tree()
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
        let mut extents = Vec::new();
        view.extents(node, |extent| extents.push(extent)).unwrap();
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
                .volume_blocks()
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
    assert_eq!(view.raw_record(node).unwrap().name(), b"readme.txt;1");
}

#[test]
fn boot_catalogs_read_back() {
    let tree = sample(false, false);
    let options = IsoOptions::default().with_el_torito(
        ElTorito::new(
            BootEntry::new("boot/boot.img")
                .with_load_size(4)
                .with_boot_info_table(BootInfo::Standard),
        )
        .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi))
        .with_catalog_path("boot/boot.cat"),
    );
    let report = hadris_iso::plan(&tree, &options).unwrap();
    let mut iso = image(&tree, &options);
    let catalog = IsoFs::mount(&mut iso, MountOptions::new())
        .unwrap()
        .boot_catalog()
        .unwrap()
        .unwrap();
    assert_eq!(
        Some(catalog.block()),
        IsoFs::mount(&mut iso, MountOptions::new())
            .unwrap()
            .boot_catalog_block()
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

#[test]
fn async_modes_read_and_write_alike() {
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
        assert_eq!(view.raw_record(readme).unwrap().name(), b"README.;1");
        let tarball = view.resolve_path("/X_TAR.GZ").unwrap();
        assert_eq!(view.raw_record(tarball).unwrap().name(), b"X_TAR.GZ;1");
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
    let options = IsoOptions::default().with_joliet(JolietLevel::L3);
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
            ElTorito::new(BootEntry::new("boot/boot.img"))
                .with_entry(BootEntry::new("boot/efi.img").with_platform(Platform::Efi)),
        )
        .with_hybrid(hadris_iso::HybridBoot::gpt());
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
