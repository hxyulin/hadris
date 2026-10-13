//! `open_partition` mounts a filesystem through an MBR or GPT table,
//! `detect_partitions` lists what each partition holds, and `AnyFs`
//! reaches format extras by `match`.

use hadris::fat::FatKind;
use hadris::fs::sync::FileSystem;
use hadris::fs::{Content, Extent, MountOptions, Name, Node, OpenMode, Resolve, SetAttr, Tree};
use hadris::part::{self, DiskLayout, Guid, MbrType, PartitionSpec, Size, gpt::types};
use hadris::storage::{BlockSize, MemDevice};
use hadris::sync::{AnyFs, detect_partitions, open, open_partition};
use hadris::{ErrorKind, ImageFormat, Select};

const ESP: Guid = Guid::from_bytes([0x11; 16]);
const DATA: Guid = Guid::from_bytes([0x22; 16]);

type Dev = MemDevice<Vec<u8>>;

fn device(len: usize, block: u32) -> Dev {
    MemDevice::new(vec![0u8; len], BlockSize::new(block).unwrap())
}

fn format_fat(dev: &mut Dev, entry: &part::Partition, kind: Option<FatKind>) {
    let mut options = hadris::fat::FatOptions::new().with_serial(0x1234_5678);
    if let Some(kind) = kind {
        options = options.with_kind(kind);
    }
    hadris::fat::sync::format(&mut part::sync::open(dev, entry).unwrap(), &options).unwrap();
}

fn format_exfat(dev: &mut Dev, entry: &part::Partition) {
    let options = hadris::fat::exfat::ExFatOptions::new().with_serial(0xCAFE_F00D);
    hadris::fat::exfat::sync::format(&mut part::sync::open(dev, entry).unwrap(), &options).unwrap();
}

/// An MBR disk: slot 0 an empty Linux partition, slot 1 a FAT volume.
fn mbr_disk() -> Dev {
    let mut dev = device(8 << 20, 512);
    let layout = DiskLayout::mbr()
        .partition(PartitionSpec::new(MbrType::LINUX, Size::MiB(1)))
        .partition(PartitionSpec::new(MbrType::FAT16, Size::Remaining));
    let disk = part::sync::create(&mut dev, &layout).unwrap();
    format_fat(&mut dev, &disk.partition(1).unwrap(), None);
    dev
}

/// A GPT disk: slot 0 a FAT12 ESP, slot 1 an exFAT data partition.
fn gpt_disk() -> Dev {
    let mut dev = device(16 << 20, 512);
    let layout = DiskLayout::gpt(Guid::from_bytes([0x33; 16]))
        .partition(
            PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(4))
                .with_name("EFI")
                .with_unique_guid(ESP),
        )
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Remaining).with_unique_guid(DATA));
    let disk = part::sync::create(&mut dev, &layout).unwrap();
    format_fat(&mut dev, &disk.partition(0).unwrap(), Some(FatKind::Fat12));
    format_exfat(&mut dev, &disk.partition(1).unwrap());
    dev
}

fn put<F: FileSystem>(fs: &mut F, name: &str, data: &[u8]) {
    let root = fs.root();
    let node = fs.create(root, Name::new(name), &SetAttr::new()).unwrap();
    fs.open(node, OpenMode::Write).unwrap();
    assert_eq!(fs.write(node, 0, data).unwrap(), data.len());
    fs.close(node).unwrap();
    fs.forget(node, 1);
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

/// Writes `data` to `name` in the partition `select` names, unmounts, and
/// reads it back through a fresh read-only mount of the whole disk.
fn round_trip(dev: Dev, select: Select, name: &str, data: &[u8]) -> Dev {
    let mut fs = open_partition(dev, select, MountOptions::new()).unwrap();
    put(&mut fs, name, data);
    let dev = fs.unmount().unwrap().into_inner();
    let mut fs = open_partition(dev, select, MountOptions::new().read_only()).unwrap();
    assert_eq!(get(&mut fs, &format!("/{name}")), data);
    fs.into_inner().into_inner()
}

#[test]
fn an_mbr_partition_round_trips() {
    let dev = round_trip(mbr_disk(), Select::Index(1), "MBR.TXT", b"through the mbr");
    let fs = open_partition(dev, Select::First, MountOptions::new().read_only()).unwrap();
    assert!(matches!(fs, AnyFs::Fat(_)));
    let window = fs.into_inner();
    assert_eq!(window.offset(), 2 << 20);
}

#[test]
fn a_gpt_partition_round_trips_by_index_and_guid() {
    let dev = round_trip(gpt_disk(), Select::Index(0), "ESP.TXT", b"esp");
    let dev = round_trip(dev, Select::Guid(DATA), "DATA.TXT", b"data");

    let fs = open_partition(dev, Select::First, MountOptions::new().read_only()).unwrap();
    assert!(matches!(&fs, AnyFs::Fat(fat) if fat.info().kind() == FatKind::Fat12));
    let dev = fs.into_inner().into_inner();
    let mut fs = open_partition(dev, Select::Guid(ESP), MountOptions::new().read_only()).unwrap();
    assert_eq!(get(&mut fs, "/ESP.TXT"), b"esp");
    let dev = fs.into_inner().into_inner();
    let fs = open_partition(dev, Select::Index(1), MountOptions::new().read_only()).unwrap();
    assert_eq!(fs.image_format(), ImageFormat::ExFat);
}

#[test]
fn detect_partitions_lists_each_partition() {
    let found: Vec<_> = detect_partitions(&mut gpt_disk())
        .unwrap()
        .into_iter()
        .map(|(entry, found)| {
            let formats: Vec<_> = found.iter().map(|c| c.format()).collect();
            (entry.slot(), entry.unique_guid(), formats)
        })
        .collect();
    assert_eq!(
        found,
        [
            (0, Some(ESP), vec![ImageFormat::Fat(FatKind::Fat12)]),
            (1, Some(DATA), vec![ImageFormat::ExFat]),
        ]
    );

    let found = detect_partitions(&mut mbr_disk()).unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].1.iter().count(), 0);
    assert!(matches!(
        found[1].1.first().unwrap().format(),
        ImageFormat::Fat(_)
    ));
}

#[test]
fn a_missing_partition_or_table_gives_the_device_back() {
    for select in [Select::Index(7), Select::Guid(Guid::from_bytes([0x44; 16]))] {
        let err = open_partition(gpt_disk(), select, MountOptions::new()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(err.into_parts().1.into_inner().len(), 16 << 20);
    }

    let err = open_partition(mbr_disk(), Select::Index(0), MountOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotRecognized);

    let mut empty = device(8 << 20, 512);
    let layout = DiskLayout::mbr().partition(PartitionSpec::new(MbrType::LINUX, Size::Remaining));
    part::sync::create(&mut empty, &layout).unwrap();
    let err = open_partition(empty, Select::First, MountOptions::new()).unwrap_err();
    assert_eq!(
        (err.kind(), err.error().message()),
        (ErrorKind::NotRecognized, "no partition holds a filesystem")
    );

    let err = open_partition(device(1 << 20, 512), Select::First, MountOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotRecognized);
    assert!(detect_partitions(&mut device(1 << 20, 512)).is_err());
}

/// An MBR disk truncated to 6 MiB: slot 0 starts past its end, and slot 1
/// at 1 MiB holds a FAT volume, unless `fat` is `None`, damaged when
/// `Some(true)`.
fn truncated_disk(fat: Option<bool>) -> Dev {
    let mut dev = device(16 << 20, 512);
    let layout = DiskLayout::mbr()
        .partition(PartitionSpec::new(MbrType::LINUX, Size::MiB(4)).with_start(8 << 11))
        .partition(PartitionSpec::new(MbrType::FAT16, Size::MiB(4)).with_start(1 << 11));
    let disk = part::sync::create(&mut dev, &layout).unwrap();
    let data = disk.partition(1).unwrap();
    assert_eq!(disk.partition(0).unwrap().start(), 8 << 11);
    if fat.is_some() {
        format_fat(&mut dev, &data, Some(FatKind::Fat12));
    }
    let mut bytes = dev.into_inner();
    if fat == Some(true) {
        let boot = (data.start() * 512) as usize;
        bytes[boot + 11..boot + 13].fill(0);
    }
    bytes.truncate(6 << 20);
    MemDevice::new(bytes, BlockSize::new(512).unwrap())
}

#[test]
fn first_prefers_filesystem_errors_to_partitions_past_the_end() {
    let fs = open_partition(
        truncated_disk(Some(false)),
        Select::First,
        MountOptions::new(),
    )
    .unwrap();
    assert_eq!(fs.into_inner().offset(), 1 << 20);

    let err = open_partition(
        truncated_disk(Some(true)),
        Select::First,
        MountOptions::new(),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);

    let err = open_partition(truncated_disk(None), Select::First, MountOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert_eq!(err.into_parts().1.into_inner().len(), 6 << 20);

    let err =
        open_partition(truncated_disk(None), Select::Index(0), MountOptions::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[test]
fn any_fs_reaches_format_extras() {
    let mut fs = open_partition(gpt_disk(), Select::Index(0), MountOptions::new()).unwrap();
    assert_eq!(fs.volume_serial().unwrap(), Some(0x1234_5678));
    put(&mut fs, "RUN.BIN", &[7u8; 3000]);
    let node = fs.resolve(b"/RUN.BIN", Resolve::Lexical).unwrap();
    let mut out = [Extent::new(0, 0); 4];
    let n = fs.extents(node, 0, &mut out).unwrap();
    fs.forget(node, 1);
    assert!(n >= 1);
    assert_eq!(out[..n].iter().map(|e| e.len()).sum::<u64>(), 3000);

    fs.set_label(Some("BOOT")).unwrap();
    let mut buf = [0u8; 32];
    assert_eq!(fs.label(&mut buf).unwrap(), Some("BOOT"));
    assert_eq!(
        fs.set_label(Some("NAME TOO LONG FOR FAT"))
            .unwrap_err()
            .kind(),
        hadris::fat::VolumeLabel::new("NAME TOO LONG FOR FAT").unwrap_err()
    );
    let dev = fs.unmount().unwrap().into_inner();

    let mut fs = open_partition(dev, Select::Index(1), MountOptions::new()).unwrap();
    assert_eq!(fs.volume_serial().unwrap(), Some(0xCAFE_F00D));
    fs.set_label(Some("Data")).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(fs.label(&mut buf).unwrap(), Some("Data"));
    let dev = fs.unmount().unwrap().into_inner();
    let mut fs = open_partition(dev, Select::Index(0), MountOptions::new().read_only()).unwrap();
    assert_eq!(fs.set_label(None).unwrap_err().kind(), ErrorKind::ReadOnly);

    let mut tree = Tree::new();
    tree.insert("A.TXT", Node::file(Content::bytes(b"iso")))
        .unwrap();
    let mut iso = device(4 << 20, 2048);
    hadris::iso::sync::write(&mut iso, &tree, &hadris::iso::IsoOptions::new()).unwrap();
    let mut fs = open(iso, MountOptions::new()).unwrap();
    assert_eq!(
        fs.volume_serial().unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        fs.set_label(Some("X")).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    let node = fs.resolve(b"/A.TXT", Resolve::Lexical).unwrap();
    let mut out = [Extent::new(0, 0); 1];
    assert_eq!(fs.extents(node, 0, &mut out).unwrap(), 1);
    assert_eq!(out[0].len(), 3);

    let mut udf = device(4 << 20, 2048);
    hadris::udf::sync::write(&mut udf, &tree, &hadris::udf::UdfOptions::new()).unwrap();
    let mut fs = open(udf, MountOptions::new()).unwrap();
    assert!(matches!(fs, AnyFs::Udf(_)));
    fs.volume_serial().unwrap();
    assert_eq!(
        fs.set_label(Some("X")).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

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

    fn send<T: Send>(value: T) -> T {
        value
    }

    #[test]
    fn async_mode_opens_and_detects_partitions() {
        let mut dev = gpt_disk();
        let found = block_on(send(hadris::async_::detect_partitions(&mut dev))).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].1.first().unwrap().format(), ImageFormat::ExFat);

        let opened = block_on(send(hadris::async_::open_partition(
            dev,
            Select::Guid(DATA),
            MountOptions::new().read_only(),
        )));
        let mut fs = opened.unwrap();
        assert!(matches!(fs, hadris::async_::AnyFs::ExFat(_)));
        assert_eq!(fs.volume_serial().unwrap(), Some(0xCAFE_F00D));
        assert_eq!(
            block_on(fs.set_label(Some("X"))).unwrap_err().kind(),
            ErrorKind::ReadOnly
        );
        let dev = block_on(fs.unmount()).unwrap().into_inner();

        let err = block_on(hadris::async_::open_partition(
            dev,
            Select::Index(9),
            MountOptions::new(),
        ))
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }
}
