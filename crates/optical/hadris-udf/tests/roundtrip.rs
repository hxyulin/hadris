//! Volumes the writer makes read back through the reader, in every mode.

mod common;

use common::Paths;
use common::{SECTOR, image, open, pattern, sample, with_metadata};
use hadris_fs::MountOptions;
use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, Node, WarningKind};
use hadris_fs::{DirCursor, ErrorKind, FileType, Permissions, Resolve};
use hadris_storage::{BlockSize, MemDevice};
use hadris_udf::{UdfId, UdfOptions, UdfRevision};

fn names(
    fs: &mut impl FileSystem<DeviceError = core::convert::Infallible>,
    path: &str,
) -> Vec<(String, FileType)> {
    let dir = fs.resolve_path(path).unwrap();
    let mut cursor = DirCursor::START;
    let mut out = Vec::new();
    while let Some(entry) = fs.readdir(dir, cursor).unwrap() {
        cursor = entry.next_cursor();
        out.push((
            String::from_utf8(entry.name().as_bytes().to_vec()).unwrap(),
            entry.file_type(),
        ));
    }
    out
}

#[test]
fn every_tree_reads_back() {
    for revision in [
        UdfRevision::V1_02,
        UdfRevision::V1_50,
        UdfRevision::V2_00,
        UdfRevision::V2_01,
    ] {
        let tree = sample();
        let options = UdfOptions::default()
            .with_id(UdfId::Volume, "ROUNDTRIP")
            .with_revision(revision);
        let mut udf = open(image(&tree, &options));
        assert_eq!(udf.info().id(hadris_udf::UdfId::Volume), "ROUNDTRIP");
        assert_eq!(udf.info().id(hadris_udf::UdfId::LogicalVolume), "ROUNDTRIP");
        assert_eq!(udf.info().revision(), revision);
        assert_eq!(udf.info().block_size(), 2048);

        assert_eq!(udf.read_to_vec("/readme.txt").unwrap(), b"hello");
        assert_eq!(udf.read_to_vec("/empty.txt").unwrap(), b"");
        assert_eq!(udf.read_to_vec("/caf\u{e9}.txt").unwrap(), b"latin1");
        assert_eq!(udf.read_to_vec("/emoji-\u{1F600}.bin").unwrap(), b"wide");
        assert_eq!(
            udf.read_to_vec("/docs/big.bin").unwrap(),
            pattern(70_000, 1)
        );
        assert_eq!(udf.read_to_vec("/docs/sub/deep.txt").unwrap(), b"deep");
        for i in [0, 41, 79] {
            assert_eq!(
                udf.read_to_vec(&format!("/many/file-{i:03}.txt")).unwrap(),
                format!("file {i}").as_bytes()
            );
        }

        let root = names(&mut udf, "/");
        let expected: Vec<(String, FileType)> = [
            ("abs", FileType::Symlink),
            ("caf\u{e9}.txt", FileType::File),
            ("emoji-\u{1F600}.bin", FileType::File),
            ("empty.txt", FileType::File),
            ("readme.txt", FileType::File),
            ("docs", FileType::Dir),
            ("emptydir", FileType::Dir),
            ("many", FileType::Dir),
        ]
        .into_iter()
        .map(|(name, kind)| (name.to_string(), kind))
        .collect();
        assert_eq!(root, expected);
        assert_eq!(names(&mut udf, "/many").len(), 80);
        assert!(names(&mut udf, "/emptydir").is_empty());

        let mut target = [0u8; 64];
        let abs = udf.resolve_path("/abs").unwrap();
        let n = udf.readlink(abs, &mut target).unwrap().len();
        assert_eq!(&target[..n], b"/docs/sub/deep.txt");
        assert_eq!(udf.stat(abs).unwrap().len(), n as u64);
        let rel = udf.resolve_path("/docs/rel").unwrap();
        let n = udf.readlink(rel, &mut target).unwrap().len();
        assert_eq!(&target[..n], b"../readme.txt");
        assert_eq!(
            udf.read(rel, 0, &mut target).unwrap_err().kind(),
            ErrorKind::Symlink
        );

        let readme = udf.resolve_path("/readme.txt").unwrap();
        let link = udf.resolve_path("/docs/link.txt").unwrap();
        assert_eq!(readme, link);
        assert_eq!(udf.stat(readme).unwrap().nlink(), 2);
        let docs = udf.resolve_path("/docs").unwrap();
        assert_eq!(udf.stat(docs).unwrap().nlink(), 3);
        let sub = udf.resolve_path("/docs/sub").unwrap();
        assert_eq!(udf.parent(sub).unwrap(), docs);
        assert_eq!(udf.parent(docs).unwrap(), udf.root());
        assert_eq!(udf.parent(udf.root()).unwrap(), udf.root());

        let big = udf.resolve_path("/docs/big.bin").unwrap();
        let mut extents = [hadris_fs::Extent::new(0, 0); 4];
        let n = udf.extents(big, 0, &mut extents).unwrap();
        let extents = &extents[..n];
        assert_eq!(extents.len(), 1);
        assert_eq!(extents[0].len(), 70_000);
    }
}

#[test]
fn metadata_reads_back() {
    let mut tree = sample();
    with_metadata(&mut tree);
    tree.replace(
        "empty.txt",
        Node::file(Content::empty())
            .with_attrs(hadris_fs::SetAttr::new().with_created(common::time(5))),
    )
    .unwrap();
    let options = UdfOptions::default();
    let report = hadris_udf::plan(&tree, &options).unwrap();
    let kinds: Vec<_> = report
        .warnings()
        .iter()
        .map(|w| (w.path(), w.kind()))
        .collect();
    assert_eq!(
        kinds,
        [
            (Some(&b"dev/null"[..]), WarningKind::Skipped),
            (None, WarningKind::Dropped(hadris_fs::Field::Created)),
        ]
    );
    let mut udf = open(image(&tree, &options));
    let meta = udf.metadata("/readme.txt").unwrap();
    assert_eq!(meta.permissions(), Permissions::new(0o4751));
    assert_eq!(meta.owner(), Some(hadris_fs::Owner::new(1000, 100)));
    let times = meta;
    assert_eq!(times.modified().unwrap().unix_seconds(), 1_700_000_000);
    assert_eq!(times.accessed().unwrap().unix_seconds(), 1_700_000_100);
    assert_eq!(
        times.changed().unwrap().unix_seconds(),
        hadris_fs::NoClock::TIME.unix_seconds()
    );
    assert_eq!(times.created(), None);
    assert_eq!(
        udf.metadata("/docs").unwrap().permissions(),
        Permissions::new(0o750)
    );

    let plain = udf.metadata("/emptydir").unwrap();
    assert_eq!(plain.permissions(), Permissions::new(0o777));
    assert_eq!(plain.owner(), None);
    assert_eq!(
        plain.modified(),
        Some(
            hadris_fs::NoClock::TIME
                .with_utc_offset_minutes(Some(0))
                .unwrap()
        )
    );
    assert!(!udf.exists("/dev/null").unwrap());
}

#[test]
fn reports_match_what_is_written() {
    let tree = sample();
    let options = UdfOptions::default();
    let bytes = image(&tree, &options);
    let report = hadris_udf::plan(&tree, &options).unwrap();
    assert_eq!(report.size(), bytes.len() as u64);
    let extent = report.extents("docs/big.bin").map(|e| e[0]).unwrap();
    assert_eq!(report.extents("/docs//big.bin").map(|e| e[0]), Some(extent));
    let start = extent.offset() as usize;
    assert_eq!(&bytes[start..start + 70_000], pattern(70_000, 1).as_slice());
    assert_eq!(
        report.extents("readme.txt").map(|e| e[0]),
        report.extents("docs/link.txt").map(|e| e[0])
    );
    assert_eq!(report.extents("empty.txt").map(|e| e[0]), None);
    assert_eq!(report.extents("docs").map(|e| e[0]), None);

    let again = image(&tree, &options);
    assert_eq!(bytes, again, "the same tree gives the same bytes");
    let dated = image(
        &tree,
        &options.clone().with_time(common::time(1_700_000_000)),
    );
    assert_ne!(bytes, dated);

    let padded = image(&tree, &options.clone().with_min_blocks(2000));
    assert_eq!(padded.len(), 2000 * 2048);
    let udf = open(padded);
    assert_eq!(udf.info().partitions()[0].len(), 2000 - 257 - 290);
}

#[test]
fn small_device_blocks_and_growing_devices_work() {
    let tree = sample();
    let options = UdfOptions::default();
    let size = hadris_udf::plan(&tree, &options).unwrap().size();
    let mut dev = MemDevice::new(vec![0u8; size as usize], BlockSize::new(512).unwrap());
    hadris_udf::sync::write(&mut dev, &tree, &options).unwrap();
    assert_eq!(dev.get_ref().as_slice(), image(&tree, &options).as_slice());
    let mut udf = hadris_udf::sync::UdfFs::mount(dev, MountOptions::new()).unwrap();
    assert_eq!(
        udf.read_to_vec("/docs/big.bin").unwrap(),
        pattern(70_000, 1)
    );

    let file = tempfile::tempfile().unwrap();
    let mut dev = hadris_storage::host::FileDevice::new(file).unwrap();
    let report = hadris_udf::sync::write(&mut dev, &tree, &options).unwrap();
    assert_eq!(
        hadris_storage::sync::BlockDevice::block_count(&dev) * 512,
        report.size()
    );
    let mut udf = hadris_udf::sync::UdfFs::mount(&mut dev, MountOptions::new()).unwrap();
    assert_eq!(udf.read_to_vec("/docs/sub/deep.txt").unwrap(), b"deep");
}

#[test]
fn async_modes_write_and_read_the_same_volume() {
    use hadris_fs::OpenMode;
    use hadris_fs::r#async::FileSystem as _;

    let tree = sample();
    let options = UdfOptions::default().with_revision(UdfRevision::V2_01);
    let expected = image(&tree, &options);
    common::block_on(async {
        let size = hadris_udf::plan(&tree, &options).unwrap().size();
        let mut dev = MemDevice::new(vec![0u8; size as usize], SECTOR);
        hadris_udf::r#async::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &expected);
        let mut udf = hadris_udf::r#async::UdfFs::mount(dev, MountOptions::new())
            .await
            .map_err(|_| ())
            .unwrap();
        let node = udf
            .resolve(b"/docs/big.bin", Resolve::Lexical)
            .await
            .unwrap();
        let mut buf = vec![0u8; 70_000];
        let mut done = 0;
        while done < buf.len() {
            done += udf.read(node, done as u64, &mut buf[done..]).await.unwrap();
        }
        assert_eq!(buf, pattern(70_000, 1));

        let mut dev = MemDevice::new(vec![0u8; size as usize], SECTOR);
        hadris_udf::r#async::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &expected);
        let mut udf = hadris_udf::r#async::UdfFs::mount(dev, MountOptions::new())
            .await
            .map_err(|_| ())
            .unwrap();
        let node = udf
            .resolve(b"/docs/sub/deep.txt", Resolve::Lexical)
            .await
            .unwrap();
        udf.open(node, OpenMode::Read).await.unwrap();
        let mut buf = [0u8; 16];
        let n = udf.read(node, 0, &mut buf).await.unwrap();
        udf.close(node).await.unwrap();
        assert_eq!(&buf[..n], b"deep");
    });
}

#[test]
fn host_trees_extract_back() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("a/b")).unwrap();
    std::fs::write(src.join("a/b/file.txt"), b"host").unwrap();
    std::fs::write(src.join("top.bin"), pattern(5000, 9)).unwrap();
    let (tree, skipped) =
        hadris_fs::host::read_tree(&src, &hadris_fs::host::TreeOptions::new()).unwrap();
    assert!(skipped.is_empty());
    let bytes = image(&tree, &UdfOptions::default());
    let vol = hadris_fs::sync::Volume::new(open(bytes));
    let back = hadris_fs::sync::read_tree(&vol, "/").unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    hadris_fs::host::write_tree(&out, &back).unwrap();
    assert_eq!(std::fs::read(out.join("a/b/file.txt")).unwrap(), b"host");
    assert_eq!(
        std::fs::read(out.join("top.bin")).unwrap(),
        pattern(5000, 9)
    );
    let _ = Content::empty();
}

#[test]
fn anchors_sequences_and_directories_follow_udf() {
    let bytes = image(&sample(), &UdfOptions::default());
    let tag = |sector: usize| u16::from_le_bytes([bytes[sector * 2048], bytes[sector * 2048 + 1]]);
    let last = bytes.len() / 2048 - 1;
    assert_eq!(tag(256), 2);
    assert_eq!(tag(last - 256), 2);
    assert_ne!(tag(last), 2, "exactly two of the three anchor locations");
    let field = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(
        (field(256 * 2048 + 16), field(256 * 2048 + 20)),
        (16 * 2048, 257)
    );
    assert_eq!(
        (field(256 * 2048 + 24), field(256 * 2048 + 28)),
        (16 * 2048, 273)
    );
    for (index, id) in [1u16, 4, 5, 6, 7, 8].into_iter().enumerate() {
        assert_eq!(tag(257 + index), id);
        assert_eq!(tag(273 + index), id);
    }
    assert_eq!(field(260 * 2048 + 268), 1, "one partition map");
    assert_eq!(bytes[260 * 2048 + 440], 1, "of type 1");
    assert_eq!(tag(289), 9);
    assert_eq!(
        field(289 * 2048 + 28),
        1,
        "the integrity descriptor is closed"
    );
    assert_eq!(tag(290), 256, "one file set descriptor");
    assert_ne!(tag(290 + 16), 256);

    let root_fids = 292 * 2048;
    assert_eq!(tag(292), 257);
    let characteristics = bytes[root_fids + 18];
    assert_eq!(
        characteristics & 0x0A,
        0x0A,
        "the parent identifier comes first"
    );
    let icb = u32::from_le_bytes(bytes[root_fids + 24..root_fids + 28].try_into().unwrap());
    assert_eq!(icb, 1, "the root is its own parent");
}

/// The volume set identifier of the primary volume descriptor.
fn volume_set(bytes: &[u8]) -> String {
    let field = &bytes[257 * 2048 + 72..257 * 2048 + 200];
    assert_eq!(field[0], 8);
    String::from_utf8(field[1..usize::from(field[127])].to_vec()).unwrap()
}

#[test]
fn identifiers_and_the_serial_follow_the_options() {
    let tree = sample();
    let options = UdfOptions::new()
        .with_id(UdfId::Volume, "DISC")
        .with_id(UdfId::LogicalVolume, "My Movies")
        .with_id(UdfId::FileSet, "SET");
    let bytes = image(&tree, &options);
    let udf = open(bytes.clone());
    assert_eq!(udf.info().id(hadris_udf::UdfId::Volume), "DISC");
    assert_eq!(udf.info().id(hadris_udf::UdfId::LogicalVolume), "My Movies");
    let serial = volume_set(&bytes);
    assert_eq!(serial.len(), 20);
    assert!(serial[..16].bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(&serial[16..], "DISC");

    assert_eq!(volume_set(&image(&tree, &options)), serial);
    let seeded = volume_set(&image(&tree, &options.clone().with_seed(7)));
    assert_ne!(seeded, serial);
    let later = common::time(1_700_000_000);
    assert_eq!(
        volume_set(&image(
            &tree,
            &options.clone().with_seed(7).with_time(later)
        )),
        seeded
    );
    assert_ne!(
        volume_set(&image(&tree, &options.clone().with_time(later))),
        serial
    );
    let mut other = sample();
    other
        .insert("extra.txt", Node::file(Content::bytes(*b"x")))
        .unwrap();
    assert_ne!(volume_set(&image(&other, &options)), serial);
    let fixed = options
        .clone()
        .with_id(UdfId::VolumeSet, "0123456789ABCDEFfixed");
    assert_eq!(volume_set(&image(&tree, &fixed)), "0123456789ABCDEFfixed");

    for (id, len) in [
        (UdfId::Volume, 31),
        (UdfId::FileSet, 31),
        (UdfId::LogicalVolume, 127),
    ] {
        let err =
            hadris_udf::plan(&tree, &UdfOptions::new().with_id(id, &"x".repeat(len))).unwrap_err();
        assert_eq!(
            err.detail().and_then(hadris_udf::Detail::from_code),
            Some(hadris_udf::Detail::Identifier),
            "{id:?}"
        );
    }
}

#[test]
fn extras_read_the_descriptors_records_and_extents() {
    let tree = sample();
    let options = UdfOptions::new()
        .with_id(UdfId::Volume, "EXTRAS")
        .with_id(UdfId::LogicalVolume, "Logical")
        .with_id(UdfId::FileSet, "Files")
        .with_seed(7)
        .with_time(common::time(1_700_000_000));
    let bytes = image(&tree, &options);
    let mut udf = open(bytes.clone());
    let info = *udf.info();
    assert_eq!(info.id(UdfId::Volume), "EXTRAS");
    assert_eq!(info.id(UdfId::LogicalVolume), "Logical");
    assert_eq!(info.id(UdfId::FileSet), "Files");
    assert!(info.id(UdfId::VolumeSet).ends_with("EXTRAS"));
    let serial = info.volume_serial().unwrap();
    assert_eq!(
        format!("{serial:016x}"),
        info.id(UdfId::VolumeSet)[..16].to_ascii_lowercase()
    );
    assert_eq!(info.domain().name(), b"*OSTA UDF Compliant");
    assert!(!info.implementation().name().is_empty());
    assert_eq!(
        info.partitions()[0].kind(),
        hadris_udf::PartitionKind::Physical
    );
    assert_eq!(
        info.recorded().map(|t| t.unix_seconds()),
        Some(1_700_000_000)
    );
    assert!(info.integrity_recorded().is_some());
    assert!(!udf.was_dirty());

    let big = udf.resolve_path("/docs/big.bin").unwrap();
    let mut out = [hadris_fs::Extent::new(0, 0); 1];
    assert_eq!(udf.extents(big, 0, &mut out).unwrap(), 1);
    let mut data = vec![0u8; out[0].len() as usize];
    udf.read_raw(out[0].offset(), &mut data).unwrap();
    assert_eq!(data, udf.read_to_vec("/docs/big.bin").unwrap());
    assert_eq!(udf.extents(big, out[0].len(), &mut out).unwrap(), 0);
    assert_eq!(udf.records(big, &mut out).unwrap(), 1);
    let mut entry = [0u8; 2];
    udf.read_raw(out[0].offset(), &mut entry).unwrap();
    assert!(matches!(u16::from_le_bytes(entry), 261 | 266));
    assert_eq!(
        udf.records(big, &mut []).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );

    let mut dirty = bytes;
    let sector = (0..dirty.len() as u64 / 2048)
        .find(|&s| dirty[s as usize * 2048..][..2] == 9u16.to_le_bytes())
        .unwrap();
    let at = sector as usize * 2048;
    let crc_length = u16::from_le_bytes([dirty[at + 10], dirty[at + 11]]) as usize;
    dirty[at + 28..at + 32].copy_from_slice(&0u32.to_le_bytes());
    common::reseal(&mut dirty, sector, crc_length);
    assert!(open(dirty).was_dirty());
}
