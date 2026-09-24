//! Images the writer makes read back through every tree of the reader.

mod common;

use common::{image, pattern, sample};
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::tree::{Content, Tree, WarningKind};
use hadris_fs::{DeviceNumber, ErrorKind, FileType, Mode, NameBuf};
use hadris_iso::sync::IsoImage;
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
    let mut iso = IsoImage::open(image(&tree, &full())).unwrap();
    let namespaces = iso.namespaces();
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
        let mut view = iso.view(ns).unwrap();
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

    let mut rr = iso.view(Namespace::RockRidge).unwrap();
    assert_eq!(
        rr.read_to_vec("/a/b/c/d/e/f/g/h/i/deep.txt").unwrap(),
        b"deep"
    );
    assert_eq!(
        rr.read_to_vec("/Long Name With Spaces é.txt").unwrap(),
        b"long"
    );
    let readme = rr.resolve("/readme.txt").unwrap();
    let meta = rr.node_metadata(readme).unwrap();
    assert_eq!(meta.permissions(), Some(Mode::new(0o600)));
    assert_eq!(meta.owner(), Some((1000, 100)));
    assert_eq!(meta.nlink(), 2);
    assert_eq!(
        meta.times().modified().unwrap().unix_seconds(),
        1_700_000_000
    );
    let hard = rr.resolve("/docs/hard.txt").unwrap();
    let info = rr.rock_ridge(hard).unwrap().unwrap();
    assert_eq!(
        info.serial(),
        rr.rock_ridge(readme).unwrap().unwrap().serial()
    );
    let link = rr.resolve("/docs/link").unwrap();
    assert_eq!(
        rr.node_metadata(link).unwrap().file_type(),
        FileType::Symlink
    );
    let mut target = [0u8; 64];
    let len = rr.read_link(link, &mut target).unwrap();
    assert_eq!(&target[..len], b"../readme.txt");
    let dev = rr.resolve("/dev/null").unwrap();
    assert_eq!(
        rr.node_metadata(dev).unwrap().file_type(),
        FileType::CharDevice
    );
    assert_eq!(
        rr.rock_ridge(dev).unwrap().unwrap().device(),
        Some(DeviceNumber::new(1, 3))
    );
    assert!(!rr.exists("/rr_moved/RRD000001").unwrap());

    let deep = rr.resolve("/a/b/c/d/e/f/g/h").unwrap();
    let parent = rr.parent(deep).unwrap();
    assert_eq!(parent, rr.resolve("/a/b/c/d/e/f/g").unwrap());

    let mut joliet = iso.view(Namespace::Joliet).unwrap();
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

    let mut primary = iso.view(Namespace::Primary).unwrap();
    assert_eq!(
        primary.read_to_vec("/readme.txt").unwrap(),
        b"hello world\n"
    );
    assert_eq!(
        primary.capabilities().case_sensitivity(),
        hadris_fs::CaseSensitivity::Insensitive
    );
}

fn names<D: FsDriver>(view: &mut D, path: &str) -> Vec<String> {
    let dir = view.resolve(path).unwrap();
    let mut cursor = hadris_fs::DirCursor::start();
    let mut name = NameBuf::new();
    let mut names = Vec::new();
    while view
        .read_dir_entry(dir, &mut cursor, &mut name)
        .unwrap()
        .is_some()
    {
        names.push(String::from_utf8(name.as_bytes().to_vec()).unwrap());
    }
    names.sort();
    names
}

#[test]
fn relocation_reuses_a_root_directory_of_that_name() {
    let mut tree = sample(true, true);
    tree.add_file("rr_moved/user.txt", Content::bytes("user"))
        .unwrap();
    tree.add_file("rr_moved/RRD000001/inner.txt", Content::bytes("inner"))
        .unwrap();
    let mut iso = IsoImage::open(image(&tree, &full())).unwrap();
    for ns in [Namespace::RockRidge, Namespace::Joliet] {
        let mut view = iso.view(ns).unwrap();
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
    let mut primary = iso.view(Namespace::Primary).unwrap();
    assert_eq!(names(&mut primary, "/RR_MOVED").len(), 3);
}

#[test]
fn listings_resume_and_skip_dots() {
    let tree = sample(false, false);
    let mut iso = IsoImage::open(image(&tree, &IsoOptions::default())).unwrap();
    let mut view = iso.view(Namespace::Preferred).unwrap();
    assert_eq!(view.namespace(), Namespace::Primary);
    let root = view.root();
    let mut cursor = hadris_fs::DirCursor::start();
    let mut name = NameBuf::new();
    let mut names = Vec::new();
    while view
        .read_dir_entry(root, &mut cursor, &mut name)
        .unwrap()
        .is_some()
    {
        names.push(String::from_utf8(name.as_bytes().to_vec()).unwrap());
    }
    assert_eq!(
        names,
        ["BOOT", "DOCS", "EMPTY", "LONG_NAM.TXT", "README.TXT"]
    );
    assert_eq!(view.read_to_vec("/docs/empty.txt").unwrap(), b"");
    assert_eq!(
        view.lookup(root, hadris_fs::Name::new("missing").unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn reports_match_what_the_reader_finds() {
    let tree = sample(true, true);
    let options = full();
    let report = hadris_iso::sync::plan(&tree, &options).unwrap();
    let mut iso = IsoImage::open(image(&tree, &options)).unwrap();
    let mut view = iso.view(Namespace::RockRidge).unwrap();
    for path in [
        "/readme.txt",
        "/docs/big.bin",
        "/docs/hard.txt",
        "/boot/efi.img",
    ] {
        let node = view.resolve(path).unwrap();
        let mut extents = Vec::new();
        view.extents(node, |extent| extents.push(extent)).unwrap();
        assert_eq!(report.extent_of(path), Some(extents[0]), "{path}");
    }
    assert_eq!(report.extent_of("docs/empty.txt"), None);
    assert!(report.warnings().is_empty());
    assert_eq!(report.total_blocks(), u64::from(iso.volume_blocks()));
}

#[test]
fn trees_without_rock_ridge_report_what_they_drop() {
    let tree = sample(false, true);
    let report = hadris_iso::sync::plan(&tree, &IsoOptions::default()).unwrap();
    let kinds: Vec<_> = report.warnings().iter().map(|w| w.kind()).collect();
    assert!(kinds.contains(&WarningKind::Skipped));
    assert!(kinds.contains(&WarningKind::IgnoredMetadata));
    assert!(kinds.contains(&WarningKind::Renamed));
    let deep = sample(true, false);
    let err = hadris_iso::sync::plan(&deep, &IsoOptions::default()).unwrap_err();
    assert_eq!(
        (err.kind(), err.detail()),
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
    let mut iso = IsoImage::open(image(&tree, &options)).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    assert!(view.exists("/readme.txt").unwrap());
    let node = view.resolve("/readme.txt").unwrap();
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
    let report = hadris_iso::sync::plan(&tree, &options).unwrap();
    let mut iso = IsoImage::open(image(&tree, &options)).unwrap();
    let catalog = iso.boot_catalog().unwrap().unwrap();
    assert_eq!(Some(catalog.block()), iso.boot_catalog_block());
    assert_eq!(
        report.extent_of("boot/boot.cat").unwrap().offset(),
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
        report.extent_of("boot/efi.img").unwrap().offset()
    );

    let mut view = iso.view(Namespace::Primary).unwrap();
    let boot = view.read_to_vec("/boot/boot.img").unwrap();
    let table: [u8; 16] = boot[8..24].try_into().unwrap();
    assert_eq!(u32::from_le_bytes(table[..4].try_into().unwrap()), 16);
    assert_eq!(
        u64::from(u32::from_le_bytes(table[4..8].try_into().unwrap())) * 2048,
        report.extent_of("boot/boot.img").unwrap().offset()
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
        let size = hadris_iso::r#async::plan(&tree, &options)
            .await
            .unwrap()
            .size_bytes();
        let mut dev = hadris_storage::MemDevice::new(vec![0u8; size as usize], common::SECTOR);
        hadris_iso::r#async::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &sync_image);
        let mut dev = hadris_storage::MemDevice::new(vec![0u8; size as usize], common::SECTOR);
        hadris_iso::async_send::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &sync_image);

        let mut iso = hadris_iso::async_send::IsoImage::open(dev).await.unwrap();
        let mut view = iso.view(Namespace::Preferred).await_view();
        let data = hadris_fs::async_send::DriverExt::read_to_vec(&mut view, "/docs/big.bin")
            .await
            .unwrap();
        assert_eq!(data, pattern(100_000));
        let linked = hadris_fs::async_send::FsDriver::resolve(&mut view, "/docs/hard.txt")
            .await
            .unwrap();
        let target = hadris_fs::async_send::FsDriver::resolve(&mut view, "/readme.txt")
            .await
            .unwrap();
        assert_eq!(linked, target);
    });
}

#[test]
fn hard_links_share_one_node_id() {
    let tree = sample(true, true);
    let mut iso = IsoImage::open(image(&tree, &full())).unwrap();
    let mut view = iso.view(Namespace::RockRidge).unwrap();
    let target = view.resolve("/readme.txt").unwrap();
    assert_eq!(view.resolve("/docs/hard.txt").unwrap(), target);
    assert_ne!(view.resolve("/docs/big.bin").unwrap(), target);
    assert_eq!(view.metadata("/docs/hard.txt").unwrap().nlink(), 2);
    let mut primary = iso.view(Namespace::Primary).unwrap();
    assert_ne!(
        primary.resolve("/README.TXT").unwrap(),
        primary.resolve("/DOCS/HARD.TXT").unwrap()
    );
}

trait AwaitView<T> {
    fn await_view(self) -> T;
}

impl<T, E: std::fmt::Debug> AwaitView<T> for Result<T, E> {
    fn await_view(self) -> T {
        self.unwrap()
    }
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
    let mut iso = IsoImage::open(dev).unwrap();
    let mut view = iso.view(Namespace::Primary).unwrap();
    assert_eq!(view.read_to_vec("/docs/big.bin").unwrap(), pattern(100_000));
}

#[test]
fn supplementary_escape_sequences_are_zero_padded() {
    let tree = sample(true, true);
    let mut iso = IsoImage::open(image(&tree, &full())).unwrap();
    let mut index = 0;
    let mut seen = 0;
    while let Some(descriptor) = iso.descriptor(index).unwrap() {
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
    tree.add_file("README", Content::bytes("r")).unwrap();
    tree.add_file("x.tar.gz", Content::bytes("x")).unwrap();
    for level in [IsoLevel::L1, IsoLevel::L2] {
        let options = IsoOptions::default().with_level(level);
        let mut iso = IsoImage::open(image(&tree, &options)).unwrap();
        let mut view = iso.view(Namespace::Primary).unwrap();
        let readme = view.resolve("/README").unwrap();
        assert_eq!(view.raw_record(readme).unwrap().name(), b"README.;1");
        let tarball = view.resolve("/X_TAR.GZ").unwrap();
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
        tree.add_file(name, Content::bytes("x")).unwrap();
    }
    let options = IsoOptions::default().with_joliet(JolietLevel::L3);
    let report = hadris_iso::sync::plan(&tree, &options).unwrap();
    let mut warned: Vec<_> = report
        .warnings()
        .iter()
        .inspect(|w| assert_eq!(w.kind(), WarningKind::Renamed))
        .map(|w| w.path().to_string())
        .collect();
    warned.sort();
    assert_eq!(
        warned,
        [
            "/a*b?c:d".to_string(),
            "/emoji\u{1F600}.txt".to_string(),
            "/makefile".to_string(),
            format!("/{long}"),
        ]
    );
    let mut iso = IsoImage::open(image(&tree, &options)).unwrap();
    let mut view = iso.view(Namespace::Joliet).unwrap();
    assert_eq!(view.read_to_vec("/a_b_c_d").unwrap(), b"x");
    assert_eq!(view.read_to_vec("/emoji_.txt").unwrap(), b"x");
}
