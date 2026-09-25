//! `detect` lists every format a device holds with the damage a mount
//! would report, and `open` mounts the first filesystem as an `AnyFs`
//! (5.8, D4, D5, Q12).

use hadris::cpio::Format;
use hadris::fat::FatKind;
use hadris::fs::sync::FileSystem;
use hadris::fs::{Content, MountOptions, Node, OpenMode, Resolve, Tree};
use hadris::storage::{BlockSize, MemDevice};
use hadris::sync::{AnyFs, detect, open};
use hadris::{ErrorKind, ImageFormat};

const PAYLOAD: &[u8] = b"detected";

fn device(bytes: Vec<u8>, block: u32) -> MemDevice<Vec<u8>> {
    MemDevice::new(bytes, BlockSize::new(block).unwrap())
}

fn formats(bytes: Vec<u8>, block: u32) -> Vec<(ImageFormat, Option<ErrorKind>)> {
    detect(&mut device(bytes, block))
        .unwrap()
        .iter()
        .map(|c| (c.format(), c.damage().map(|err| err.kind())))
        .collect()
}

fn refusal(bytes: Vec<u8>) -> (ErrorKind, &'static str) {
    let Err(err) = open(device(bytes, 512), MountOptions::new()) else {
        panic!("the device opened");
    };
    (err.kind(), err.error().message())
}

fn tree() -> Tree {
    let mut tree = Tree::new();
    tree.insert("DOCS/README.TXT", Node::file(Content::bytes(PAYLOAD)))
        .unwrap();
    tree
}

fn optical(iso: bool, udf: bool) -> Vec<u8> {
    let mut dev = device(vec![0u8; 4 * 1024 * 1024], 2048);
    let (iso_opts, udf_opts) = (
        hadris::iso::IsoOptions::new(),
        hadris::udf::UdfOptions::new(),
    );
    match (iso, udf) {
        (true, false) => drop(hadris::iso::sync::write(&mut dev, &tree(), &iso_opts).unwrap()),
        (false, true) => drop(hadris::udf::sync::write(&mut dev, &tree(), &udf_opts).unwrap()),
        _ => {
            drop(hadris::udf::sync::write_bridge(&mut dev, &tree(), &iso_opts, &udf_opts).unwrap())
        }
    }
    dev.into_inner()
}

fn fat(kind: FatKind, len: usize) -> Vec<u8> {
    let mut dev = device(vec![0u8; len], 512);
    let options = hadris::fat::FatOptions::new().with_kind(kind);
    hadris::fat::sync::format(&mut dev, &options).unwrap();
    dev.into_inner()
}

fn exfat() -> Vec<u8> {
    let mut dev = device(vec![0u8; 8 * 1024 * 1024], 512);
    hadris::fat::exfat::sync::format(&mut dev, &hadris::fat::exfat::ExFatOptions::new()).unwrap();
    dev.into_inner()
}

fn get<F: FileSystem>(fs: &mut F, path: &str) -> Vec<u8> {
    let node = fs.resolve(path.as_bytes(), Resolve::Lexical).unwrap();
    fs.open(node, OpenMode::Read).unwrap();
    let mut buf = vec![0u8; 64];
    let n = fs.read(node, 0, &mut buf).unwrap();
    fs.close(node).unwrap();
    fs.forget(node, 1);
    buf.truncate(n);
    buf
}

/// Sector 0 with MBR partition entries of the given types, and the GPT
/// header signature when `efi` is set.
fn table(types: &[u8], efi: bool) -> Vec<u8> {
    let mut image = vec![0u8; 64 * 512];
    for (i, kind) in types.iter().enumerate() {
        let entry = 446 + i * 16;
        image[entry + 4] = *kind;
        image[entry + 8..entry + 12].copy_from_slice(&1u32.to_le_bytes());
        image[entry + 12..entry + 16].copy_from_slice(&63u32.to_le_bytes());
    }
    image[510..512].copy_from_slice(&[0x55, 0xAA]);
    if efi {
        image[512..520].copy_from_slice(b"EFI PART");
    }
    image
}

#[test]
fn fat_volumes_are_detected_and_opened() {
    for (kind, len) in [
        (FatKind::Fat12, 2 << 20),
        (FatKind::Fat16, 32 << 20),
        (FatKind::Fat32, 64 << 20),
    ] {
        let image = fat(kind, len);
        assert_eq!(
            formats(image.clone(), 512),
            [(ImageFormat::Fat(kind), None)]
        );
        let mut fs = open(device(image, 512), MountOptions::new()).unwrap();
        assert!(matches!(&fs, AnyFs::Fat(fat) if fat.info().kind() == kind));
        let root = fs.root();
        fs.stat(root).unwrap();
        fs.unmount().unwrap();
    }
    assert_eq!(
        formats(fat(FatKind::Fat12, 2 << 20), 64),
        [(ImageFormat::Fat(FatKind::Fat12), None)]
    );
}

#[test]
fn a_damaged_fat_boot_sector_is_fat_with_damage() {
    let mut image = fat(FatKind::Fat12, 2 << 20);
    image[11..13].fill(0);
    assert_eq!(
        formats(image.clone(), 512),
        [(ImageFormat::Fat(FatKind::Fat12), Some(ErrorKind::Corrupt))]
    );
    assert_eq!(refusal(image).0, ErrorKind::Corrupt);
}

#[test]
fn exfat_is_detected_opened_and_reported_damaged() {
    let image = exfat();
    assert_eq!(formats(image.clone(), 512), [(ImageFormat::ExFat, None)]);
    let fs = open(device(image.clone(), 512), MountOptions::new()).unwrap();
    assert!(matches!(fs, AnyFs::ExFat(_)));

    let mut image = image;
    image[100] ^= 0xFF;
    image[12 * 512 + 100] ^= 0xFF;
    assert_eq!(
        formats(image.clone(), 512),
        [(ImageFormat::ExFat, Some(ErrorKind::Corrupt))]
    );
    assert_eq!(refusal(image).0, ErrorKind::Corrupt);
}

#[test]
fn optical_images_list_the_bridge_first() {
    for block in [512, 2048, 4096] {
        assert_eq!(
            formats(optical(true, false), block),
            [(ImageFormat::Iso, None)]
        );
        assert_eq!(
            formats(optical(false, true), block),
            [(ImageFormat::Udf, None)]
        );
        assert_eq!(
            formats(optical(true, true), block),
            [
                (ImageFormat::IsoUdfBridge, None),
                (ImageFormat::Iso, None),
                (ImageFormat::Udf, None),
            ]
        );
    }
    let mut fs = open(device(optical(true, false), 2048), MountOptions::new()).unwrap();
    assert!(matches!(fs, AnyFs::Iso(_)));
    assert_eq!(get(&mut fs, "/DOCS/README.TXT"), PAYLOAD);
    let mut fs = open(device(optical(true, true), 2048), MountOptions::new()).unwrap();
    assert!(matches!(fs, AnyFs::Udf(_)));
    assert_eq!(get(&mut fs, "/DOCS/README.TXT"), PAYLOAD);
}

#[test]
fn a_bridge_with_a_damaged_udf_side_opens_as_iso() {
    let mut image = optical(true, true);
    image[256 * 2048..257 * 2048].fill(0);
    let last = image.len() / 2048;
    for anchor in [last - 257, last - 1] {
        image[anchor * 2048..(anchor + 1) * 2048].fill(0);
    }
    let found = formats(image.clone(), 2048);
    assert_eq!(found[0].0, ImageFormat::IsoUdfBridge);
    assert!(found[0].1.is_some());
    assert_eq!(found[1], (ImageFormat::Iso, None));
    let mut fs = open(device(image, 2048), MountOptions::new()).unwrap();
    assert!(matches!(fs, AnyFs::Iso(_)));
    assert_eq!(get(&mut fs, "/DOCS/README.TXT"), PAYLOAD);
}

#[test]
fn descriptors_without_volumes_are_damaged_not_foreign() {
    let mut image = vec![0u8; 40 * 2048];
    image[16 * 2048 + 1..16 * 2048 + 7].copy_from_slice(b"CD001\x01");
    let found = formats(image.clone(), 2048);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, ImageFormat::Iso);
    assert!(found[0].1.is_some());
    assert_ne!(refusal(image).0, ErrorKind::NotRecognized);
}

#[test]
fn partition_tables_are_listed_but_not_opened() {
    assert_eq!(
        formats(table(&[0x83], false), 512),
        [(ImageFormat::Mbr, None)]
    );
    assert_eq!(
        formats(table(&[0xEE], true), 512),
        [(ImageFormat::Gpt, None)]
    );
    assert_eq!(
        formats(table(&[0xEE, 0x0C], true), 512),
        [(ImageFormat::Gpt, None), (ImageFormat::Mbr, None)]
    );
    assert_eq!(formats(table(&[0xEE], false), 512), []);
    assert_eq!(
        refusal(table(&[0x83], false)),
        (ErrorKind::NotRecognized, "partition table")
    );
}

#[test]
fn a_partition_opens_through_its_table() {
    use hadris::part;
    let (start, blocks) = (2048u64, (2u64 << 20) / 512);
    let layout = part::DiskLayout::gpt(part::Guid::from_bytes([0x61; 16])).partition(
        part::PartitionSpec::new(part::gpt::types::EFI_SYSTEM, part::Size::Blocks(blocks))
            .with_start(start),
    );
    let mut disk = device(vec![0u8; ((start + blocks + 34) * 512) as usize], 512);
    let table = part::sync::create(&mut disk, &layout).unwrap();
    let entry = table.partition(0).unwrap();
    let options = hadris::fat::FatOptions::new().with_kind(FatKind::Fat12);
    hadris::fat::sync::format(&mut part::sync::open(&mut disk, &entry).unwrap(), &options).unwrap();

    let found: Vec<_> = detect(&mut disk)
        .unwrap()
        .iter()
        .map(|c| c.format())
        .collect();
    assert_eq!(found, [ImageFormat::Gpt]);
    let entry = part::sync::read(&mut disk).unwrap().partition(0).unwrap();
    let fs = open(
        part::sync::open(&mut disk, &entry).unwrap(),
        MountOptions::new(),
    )
    .unwrap();
    assert!(matches!(&fs, AnyFs::Fat(fat) if fat.info().kind() == FatKind::Fat12));
    assert_eq!(fs.into_inner().offset(), start * 512);
}

#[test]
fn a_4kn_gpt_header_is_in_block_one() {
    let mut image = table(&[0xEE], true);
    image.resize(4 * 4096, 0);
    assert_eq!(formats(image.clone(), 4096), []);
    image[512..520].fill(0);
    image[4096..4104].copy_from_slice(b"EFI PART");
    assert_eq!(formats(image, 4096), [(ImageFormat::Gpt, None)]);
}

#[test]
fn a_hybrid_iso_lists_the_table_after_the_image() {
    let mut image = optical(true, false);
    image[..512].copy_from_slice(&table(&[0x17], false)[..512]);
    assert_eq!(
        formats(image.clone(), 2048),
        [(ImageFormat::Iso, None), (ImageFormat::Mbr, None)]
    );
    assert!(matches!(
        open(device(image, 2048), MountOptions::new()).unwrap(),
        AnyFs::Iso(_)
    ));
}

#[test]
fn ntfs_is_detected_but_not_opened() {
    let mut image = vec![0u8; 64 * 512];
    image[3..11].copy_from_slice(b"NTFS    ");
    image[510..512].copy_from_slice(&[0x55, 0xAA]);
    assert_eq!(formats(image.clone(), 512), [(ImageFormat::Ntfs, None)]);
    assert_eq!(refusal(image), (ErrorKind::NotRecognized, "ntfs"));
}

#[test]
fn cpio_archives_are_detected_but_not_opened() {
    for (magic, format) in [
        (&b"070701"[..], Format::Newc),
        (b"070702", Format::Crc),
        (b"070707", Format::Odc),
        (&[0xC7, 0x71], Format::Binary),
    ] {
        let mut image = vec![0u8; 1024];
        image[..magic.len()].copy_from_slice(magic);
        assert_eq!(
            formats(image.clone(), 512),
            [(ImageFormat::Cpio(format), None)]
        );
        assert_eq!(refusal(image), (ErrorKind::NotRecognized, "archive"));
    }
}

#[test]
fn blank_small_and_huge_block_devices_hold_nothing() {
    assert_eq!(formats(vec![0u8; 64 * 512], 512), []);
    assert_eq!(formats(vec![0u8; 256], 256), []);
    assert_eq!(formats(fat(FatKind::Fat12, 2 << 20), 8192), []);
    assert_eq!(refusal(vec![0u8; 64 * 512]).0, ErrorKind::NotRecognized);
}

#[test]
fn a_failed_open_gives_the_device_back() {
    let image = table(&[0x83], false);
    let Err(err) = open(device(image.clone(), 512), MountOptions::new()) else {
        panic!("the device opened");
    };
    assert_eq!(err.into_device().into_inner(), image);
}

#[test]
fn host_open_mounts_an_image_file_read_only() {
    let path = std::env::temp_dir().join(format!("hadris-detect-{}.iso", std::process::id()));
    std::fs::write(&path, optical(true, false)).unwrap();
    let mut fs = hadris::host::open(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(!fs.capabilities().writable());
    assert_eq!(get(&mut fs, "/DOCS/README.TXT"), PAYLOAD);

    let err = hadris::host::open(&path).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert_eq!(err.host_path(), Some(path.as_path()));
}

/// Read from the trait definition, so a method added to `FileSystem`
/// fails here until `AnyFs` forwards it.
#[test]
fn any_fs_forwards_every_trait_method() {
    fn methods(source: &str) -> std::collections::BTreeSet<&str> {
        source
            .split("fn ")
            .skip(1)
            .filter_map(|rest| rest.split(['(', '<']).next())
            .filter(|name| name.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect()
    }
    fn block<'s>(source: &'s str, start: &str) -> &'s str {
        let body = &source[source.find(start).unwrap()..];
        &body[..body.find("\n}\n").unwrap()]
    }
    let trait_source = include_str!("../../hadris-fs/src/api/filesystem.rs").replace("\r\n", "\n");
    let any_source = include_str!("../src/open.rs").replace("\r\n", "\n");
    let declared = methods(block(&trait_source, "pub trait FileSystem {"));
    let forwarded = methods(block(
        &any_source,
        "impl<D: BlockDevice> FileSystem for AnyFs<D> {",
    ));
    assert!(declared.len() > 20, "{declared:?}");
    assert_eq!(declared, forwarded);
}

#[cfg(feature = "async")]
mod asynch {
    use super::*;

    fn block_on<F: core::future::Future>(future: F) -> F::Output {
        use core::task::{Context, Poll, Waker};
        let mut future = core::pin::pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
                return out;
            }
        }
    }

    #[test]
    fn async_mode_detects_and_opens() {
        use hadris::fs::r#async::FileSystem;
        block_on(async {
            let found = hadris::r#async::detect(&mut device(optical(true, true), 2048))
                .await
                .unwrap();
            assert_eq!(found.first().unwrap().format(), ImageFormat::IsoUdfBridge);
            assert_eq!(found.iter().count(), 3);

            let mut fs = hadris::r#async::open(
                device(fat(FatKind::Fat12, 2 << 20), 512),
                MountOptions::new(),
            )
            .await
            .unwrap();
            assert!(matches!(fs, hadris::r#async::AnyFs::Fat(_)));
            let root = fs.root();
            fs.stat(root).await.unwrap();
            fs.unmount().await.unwrap();
        });
    }

    /// The futures are `Send` with no bounds beyond the mode's own.
    #[test]
    fn async_futures_are_send() {
        fn send<T: Send>(value: T) -> T {
            value
        }
        let mut dev = device(optical(true, false), 2048);
        assert_eq!(
            block_on(send(hadris::r#async::detect(&mut dev)))
                .unwrap()
                .iter()
                .count(),
            1
        );
        let opened = block_on(send(hadris::r#async::open(dev, MountOptions::new())));
        assert!(matches!(opened, Ok(hadris::r#async::AnyFs::Iso(_))));
    }
}
