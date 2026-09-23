//! Images the writer makes read back through every tree of the reader.

mod common;

use common::{image, pattern, sample};
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::tree::WarningKind;
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
    });
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
