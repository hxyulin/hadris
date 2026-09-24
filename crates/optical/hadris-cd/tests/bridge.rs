//! Hybrid images read back through both the ISO 9660 and the UDF reader,
//! and both trees point at the same file data.

use hadris_cd::iso::{Namespace, RockRidge, VolumeIdentifiers};
use hadris_cd::udf::UdfRevision;
use hadris_cd::{CdOptions, IsoOptions, UdfOptions};
use hadris_fs::sync::{DriverExt, FsDriver};
use hadris_fs::tree::{Content, Tree};
use hadris_fs::{ErrorKind, Extent, Mode, SetMetadata};
use hadris_storage::{BlockSize, MemDevice};

const SECTOR: usize = 2048;
const VOLUME: &str = "BRIDGE_TEST";

fn large() -> Vec<u8> {
    (0..5000).map(|i| (i % 251) as u8).collect()
}

fn fixture() -> Tree {
    let mut tree = Tree::new();
    tree.add_file("EMPTY.TXT", Content::empty()).unwrap();
    tree.add_file("DOCS/LARGE.BIN", Content::bytes(large()))
        .unwrap();
    tree.add_file(
        "DOCS/NESTED/NOTE.TXT",
        Content::bytes("qualified through both namespaces"),
    )
    .unwrap();
    tree.add_hard_link("DOCS/COPY.BIN", "DOCS/LARGE.BIN")
        .unwrap();
    tree.add_symlink("LINK", "DOCS/NESTED/NOTE.TXT").unwrap();
    tree.set_metadata(
        "DOCS/NESTED/NOTE.TXT",
        SetMetadata::new().with_mode(Mode::new(0o640)),
    )
    .unwrap();
    tree
}

fn options(revision: UdfRevision) -> CdOptions {
    CdOptions::default()
        .with_iso(
            CdOptions::default()
                .iso()
                .clone()
                .with_volume(VolumeIdentifiers::new(VOLUME))
                .with_rock_ridge(RockRidge::default()),
        )
        .with_udf(
            UdfOptions::default()
                .with_volume_id(VOLUME)
                .with_revision(revision),
        )
}

fn create(tree: &Tree, options: &CdOptions) -> Vec<u8> {
    let report = hadris_cd::sync::plan(tree, options).unwrap();
    let mut dev = MemDevice::new(
        vec![0u8; report.size_bytes() as usize],
        BlockSize::new(2048).unwrap(),
    );
    let written = hadris_cd::sync::write(&mut dev, tree, options).unwrap();
    assert_eq!(written, report);
    dev.into_inner()
}

fn tag_at(bytes: &[u8], sector: usize) -> u16 {
    u16::from_le_bytes([bytes[sector * SECTOR], bytes[sector * SECTOR + 1]])
}

fn verify(bytes: &[u8]) {
    let dev = MemDevice::new(bytes, BlockSize::new(2048).unwrap());
    let mut iso = hadris_cd::iso::sync::IsoImage::open(dev).unwrap();
    let mut view = iso.view(Namespace::RockRidge).unwrap();
    assert_eq!(view.read_to_vec("/EMPTY.TXT").unwrap(), b"");
    assert_eq!(view.read_to_vec("/DOCS/LARGE.BIN").unwrap(), large());
    let mut iso_extents = Vec::new();
    for path in ["/DOCS/LARGE.BIN", "/DOCS/NESTED/NOTE.TXT"] {
        let node = view.resolve(path).unwrap();
        let mut extents = Vec::new();
        view.extents(node, |extent| extents.push(extent)).unwrap();
        iso_extents.push(extents);
    }

    let mut udf =
        hadris_cd::udf::sync::UdfFs::open(MemDevice::new(bytes, BlockSize::new(2048).unwrap()))
            .unwrap();
    assert_eq!(udf.volume_id(), VOLUME);
    assert_eq!(udf.read_to_vec("/EMPTY.TXT").unwrap(), b"");
    assert_eq!(udf.read_to_vec("/DOCS/LARGE.BIN").unwrap(), large());
    assert_eq!(udf.read_to_vec("/DOCS/COPY.BIN").unwrap(), large());
    assert_eq!(
        udf.read_to_vec("/DOCS/NESTED/NOTE.TXT").unwrap(),
        b"qualified through both namespaces"
    );
    assert_eq!(
        udf.metadata("/DOCS/NESTED/NOTE.TXT").unwrap().permissions(),
        Some(Mode::new(0o640))
    );
    let link = udf.resolve("/LINK").unwrap();
    let mut target = [0u8; 64];
    let n = udf.read_link(link, &mut target).unwrap();
    assert_eq!(&target[..n], b"DOCS/NESTED/NOTE.TXT");
    assert_eq!(
        udf.resolve("/DOCS/COPY.BIN").unwrap(),
        udf.resolve("/DOCS/LARGE.BIN").unwrap()
    );
    for (path, iso) in ["/DOCS/LARGE.BIN", "/DOCS/NESTED/NOTE.TXT"]
        .into_iter()
        .zip(iso_extents)
    {
        let node = udf.resolve(path).unwrap();
        let mut extents: Vec<Extent> = Vec::new();
        udf.extents(node, |extent| extents.push(extent)).unwrap();
        assert_eq!(extents, iso, "{path} shares its data");
    }
}

#[test]
fn bridge_reads_back_through_iso_and_udf() {
    for revision in [UdfRevision::V1_02, UdfRevision::V2_01] {
        let tree = fixture();
        let bytes = create(&tree, &options(revision));
        verify(&bytes);
    }
}

#[test]
fn bridge_layout_follows_udf_and_ecma_119() {
    let tree = fixture();
    let bytes = create(&tree, &options(UdfRevision::V1_02));
    let last = bytes.len() / SECTOR - 1;
    assert_eq!(tag_at(&bytes, 256), 2, "anchor at 256");
    assert_eq!(tag_at(&bytes, last - 256), 2, "anchor at N-256");
    assert_ne!(tag_at(&bytes, last), 2);
    for (index, id) in [1u16, 4, 5, 6, 7, 8].into_iter().enumerate() {
        assert_eq!(tag_at(&bytes, 257 + index), id);
        assert_eq!(tag_at(&bytes, 273 + index), id);
    }
    assert_eq!(tag_at(&bytes, 289), 9, "integrity descriptor");
    assert_eq!(tag_at(&bytes, 290), 256, "file set descriptor");
    let field = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(
        (field(256 * SECTOR + 16), field(256 * SECTOR + 20)),
        (16 * 2048, 257)
    );
    assert_eq!(
        (field(256 * SECTOR + 24), field(256 * SECTOR + 28)),
        (16 * 2048, 273)
    );
    assert_eq!(
        field(289 * SECTOR + 28),
        1,
        "the integrity descriptor is closed"
    );
    assert_eq!(field(260 * SECTOR + 268), 1, "one partition map");
    assert_eq!(
        field(259 * SECTOR + 188),
        290,
        "the partition starts after the integrity descriptor"
    );
    let mut udf = hadris_cd::udf::sync::UdfFs::open(MemDevice::new(
        bytes.as_slice(),
        BlockSize::new(2048).unwrap(),
    ))
    .unwrap();
    let large = udf.resolve("/DOCS/LARGE.BIN").unwrap();
    let entry = (290 + large.get() - 1) as usize * SECTOR;
    assert_eq!(
        u16::from_le_bytes([bytes[entry + 34], bytes[entry + 35]]) & 7,
        0,
        "short allocation descriptors"
    );

    let terminator = (16..32)
        .find(|&s| bytes[s * SECTOR] == 255 && &bytes[s * SECTOR + 1..s * SECTOR + 6] == b"CD001")
        .unwrap();
    for (index, id) in [b"BEA01", b"NSR02", b"TEA01"].into_iter().enumerate() {
        let at = (terminator + 1 + index) * SECTOR;
        assert_eq!(&bytes[at + 1..at + 6], id);
    }
    let bytes = create(&tree, &options(UdfRevision::V2_01));
    let at = (terminator + 2) * SECTOR;
    assert_eq!(&bytes[at + 1..at + 6], b"NSR03");
}

#[test]
fn reports_devices_and_modes_agree() {
    let tree = fixture();
    let options = options(UdfRevision::V1_02);
    let bytes = create(&tree, &options);
    let report = hadris_cd::sync::plan(&tree, &options).unwrap();
    assert_eq!(
        report.extent_of("DOCS/LARGE.BIN"),
        report.udf().extent_of("DOCS/LARGE.BIN")
    );
    assert_eq!(
        report.extent_of("DOCS/COPY.BIN"),
        report.extent_of("DOCS/LARGE.BIN")
    );
    assert!(
        report.udf().allocated_end()
            <= report
                .iso()
                .extent_of("EMPTY.TXT")
                .map_or(u64::MAX, |e| e.offset() / 2048)
    );

    let file = tempfile_like();
    let dev = hadris_storage::host::FileDevice::new(file.0.try_clone().unwrap()).unwrap();
    let written = hadris_cd::sync::write(dev, &tree, &options).unwrap();
    assert_eq!(written.size_bytes(), file.0.metadata().unwrap().len());
    let mut host = Vec::new();
    std::io::Read::read_to_end(&mut std::fs::File::open(&file.1).unwrap(), &mut host).unwrap();
    assert_eq!(host, bytes);

    let expected = bytes;
    let block_on = |future: std::pin::Pin<&mut dyn core::future::Future<Output = ()>>| {
        let mut future = future;
        let mut cx = core::task::Context::from_waker(core::task::Waker::noop());
        while future.as_mut().poll(&mut cx).is_pending() {}
    };
    let run = async {
        let mut dev = MemDevice::new(vec![0u8; expected.len()], BlockSize::new(2048).unwrap());
        hadris_cd::r#async::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &expected);
        let mut dev = MemDevice::new(vec![0u8; expected.len()], BlockSize::new(2048).unwrap());
        let report = hadris_cd::async_send::plan(&tree, &options).await.unwrap();
        assert_eq!(report.size_bytes(), expected.len() as u64);
        hadris_cd::async_send::write(&mut dev, &tree, &options)
            .await
            .unwrap();
        assert_eq!(dev.get_ref(), &expected);
    };
    block_on(std::pin::pin!(run));
    let _ = std::fs::remove_file(&file.1);
}

fn tempfile_like() -> (std::fs::File, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("hadris-cd-{}.iso", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    (file, path)
}

#[test]
fn writer_errors_keep_their_detail() {
    let tree = fixture();
    let mut dev = MemDevice::new(vec![0u8; 8 << 20], BlockSize::new(4096).unwrap());
    let err = hadris_cd::sync::write(&mut dev, &tree, &CdOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(
        err.detail().and_then(hadris_cd::iso::Detail::from_code),
        Some(hadris_cd::iso::Detail::OutputBlockSize)
    );
    assert_eq!(err.detail().and_then(hadris_cd::Detail::from_code), None);

    let mut long = Tree::new();
    long.add_file(&"n".repeat(255), Content::empty()).unwrap();
    let options = CdOptions::default().with_iso(IsoOptions::default());
    let err = hadris_cd::sync::plan(&long, &options).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NameTooLong);
}

#[test]
fn iso_volume_space_covers_the_udf_tail() {
    let tree = fixture();
    let iso = options(UdfRevision::V2_01)
        .iso()
        .clone()
        .with_joliet(hadris_cd::iso::JolietLevel::L3);
    let options = options(UdfRevision::V2_01).with_iso(iso);
    let bytes = create(&tree, &options);
    let blocks = (bytes.len() / SECTOR) as u32;
    let mut descriptors = 0;
    for sector in 16.. {
        let at = sector * SECTOR;
        match bytes[at] {
            255 => break,
            1 | 2 => {
                let field = &bytes[at + 80..at + 88];
                assert_eq!(field[..4], blocks.to_le_bytes(), "sector {sector}");
                assert_eq!(field[4..], blocks.to_be_bytes(), "sector {sector}");
                descriptors += 1;
            }
            _ => {}
        }
    }
    assert!(descriptors >= 2);
    let dev = MemDevice::new(bytes.as_slice(), BlockSize::new(2048).unwrap());
    let iso = hadris_cd::iso::sync::IsoImage::open(dev).unwrap();
    assert_eq!(iso.volume_blocks(), blocks);
    verify(&bytes);
}
