//! Table edits: overlap and bounds checks, resize and remove, and the
//! layout builder's rules. A failed edit leaves the table unchanged.

use hadris_fs::ErrorKind;
use hadris_part::gpt::types;
use hadris_part::sync::{read, write};
use hadris_part::{
    Alignment, Detail, Disk, DiskLayout, Gpt, GptEntry, Guid, Hybrid, HybridMbr, Mbr, MbrEntry,
    MbrType, PartitionFlags, PartitionKind, PartitionName, PartitionSpec, PartitionTable, Size,
};
use hadris_storage::{BlockSize, MemDevice};

const B512: BlockSize = BlockSize::new(512).unwrap();

fn guid(n: u8) -> Guid {
    Guid::from_bytes([n; 16])
}

fn gpt() -> Gpt {
    Gpt::new(guid(0xD1), 10_000, B512).unwrap()
}

#[test]
fn gpt_edits_reject_overlap_and_bounds_and_change_nothing() {
    let mut table = gpt();
    let a = table
        .add(GptEntry::new(types::BASIC_DATA, guid(1), 100, 100))
        .unwrap();
    let b = table
        .add(GptEntry::new(types::LINUX_FILESYSTEM, guid(2), 300, 100))
        .unwrap();
    assert_eq!((a, b), (0, 1));
    let before = table.clone();

    for (start, len) in [(150, 10), (50, 100), (199, 2), (399, 1), (0, 10)] {
        let err = table
            .add(GptEntry::new(types::LINUX_SWAP, guid(3), start, len))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{start}+{len}");
    }
    let err = table
        .add(GptEntry::new(types::LINUX_SWAP, guid(3), 150, 10))
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Overlap { index: 2, other: 0 });
    assert_eq!(
        table
            .add(GptEntry::new(types::LINUX_SWAP, guid(3), 9_960, 10))
            .unwrap_err()
            .detail(),
        Detail::OutOfBounds { index: 2 }
    );
    assert_eq!(
        table
            .add(GptEntry::new(types::UNUSED, guid(3), 500, 10))
            .unwrap_err()
            .detail(),
        Detail::Kind
    );
    assert_eq!(
        table
            .add(GptEntry::new(types::BASIC_DATA, guid(3), 500, 0))
            .unwrap_err()
            .detail(),
        Detail::Size
    );
    assert_eq!(
        table.resize(a, 201).unwrap_err().detail(),
        Detail::Overlap { index: 0, other: 1 }
    );
    assert_eq!(
        table.resize(b, 10_000).unwrap_err().detail(),
        Detail::OutOfBounds { index: 1 }
    );
    assert_eq!(table.resize(7, 1).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(
        table.set_type(a, types::UNUSED).unwrap_err().detail(),
        Detail::Kind
    );
    assert_eq!(table, before);

    table.resize(a, 200).unwrap();
    assert_eq!(table.entry(a).unwrap().end(), 300);
    table
        .set_name(b, PartitionName::new("données").unwrap())
        .unwrap();
    table
        .set_flags(b, PartitionFlags::BOOTABLE | PartitionFlags::NO_BLOCK_IO)
        .unwrap();
    table.set_attributes(a, 1 << 60).unwrap();
    table.set_unique_guid(a, guid(9)).unwrap();
    table.set_type(a, types::EFI_SYSTEM).unwrap();
    let p = table.entry(b).unwrap();
    assert_eq!(p.name().unwrap().to_string(), "données");
    assert_eq!(p.attributes(), 0b110);
    let p = table.entry(a).unwrap();
    assert_eq!((p.attributes(), p.unique_guid()), (1 << 60, Some(guid(9))));
    assert_eq!(p.kind(), PartitionKind::Gpt(types::EFI_SYSTEM));

    table.remove(a).unwrap();
    assert_eq!(table.remove(a).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(
        table
            .add(GptEntry::new(types::BASIC_DATA, guid(4), 100, 200))
            .unwrap(),
        0
    );
}

#[test]
fn gpt_table_full_and_tiny_disks() {
    let mut table = gpt();
    for i in 0..128 {
        table
            .add(GptEntry::new(types::BASIC_DATA, guid(1), 100 + i * 10, 10))
            .unwrap();
    }
    let err = table
        .add(GptEntry::new(types::BASIC_DATA, guid(1), 5000, 10))
        .unwrap_err();
    assert_eq!(
        (err.kind(), err.detail()),
        (ErrorKind::LimitExceeded, Detail::TableFull)
    );

    assert_eq!(
        Gpt::new(guid(0), 67, B512).unwrap_err().kind(),
        ErrorKind::NoSpace
    );
    assert!(Gpt::new(guid(0), 68, B512).is_ok());
    assert_eq!(
        Gpt::new(guid(0), 1000, BlockSize::new(520).unwrap())
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn mbr_edits_cover_primaries_and_logicals() {
    let mut mbr = Mbr::new(100_000, B512).unwrap();
    mbr.add(MbrEntry::new(MbrType::FAT32_LBA, 2048, 2048))
        .unwrap();
    let ext = mbr
        .add(MbrEntry::new(MbrType::EXTENDED_LBA, 8192, 20_000))
        .unwrap();
    let first = mbr
        .add_logical(MbrEntry::new(MbrType::LINUX, 10_000, 1000))
        .unwrap();
    let second = mbr
        .add_logical(MbrEntry::new(MbrType::LINUX_SWAP, 12_000, 1000))
        .unwrap();
    assert_eq!((ext, first, second), (1, 4, 5));
    let before = mbr.clone();

    let err = mbr
        .add_logical(MbrEntry::new(MbrType::LINUX, 10_500, 100))
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Overlap { index: 5, other: 4 });
    let err = mbr
        .add_logical(MbrEntry::new(MbrType::LINUX, 11_000, 1000))
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Overlap { index: 5, other: 4 });
    assert_eq!(
        mbr.add_logical(MbrEntry::new(MbrType::LINUX, 27_000, 2000))
            .unwrap_err()
            .detail(),
        Detail::OutOfBounds { index: 6 }
    );
    assert_eq!(
        mbr.add(MbrEntry::new(MbrType::LINUX, 20_000, 100))
            .unwrap_err()
            .detail(),
        Detail::Overlap { index: 2, other: 1 }
    );
    assert_eq!(
        mbr.resize(ext, 3000).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        mbr.resize(first, 2000).unwrap_err().detail(),
        Detail::Overlap { index: 5, other: 4 }
    );
    assert_eq!(mbr.remove(ext).unwrap_err().detail(), Detail::ExtendedInUse);
    assert_eq!(
        mbr.set_kind(first, MbrType::EXTENDED).unwrap_err().detail(),
        Detail::Extended
    );
    assert_eq!(
        mbr.set_flags(first, PartitionFlags::REQUIRED)
            .unwrap_err()
            .detail(),
        Detail::Flags
    );
    assert_eq!(mbr, before);

    mbr.resize(first, 1998).unwrap();
    mbr.set_flags(second, PartitionFlags::BOOTABLE).unwrap();
    mbr.set_kind(second, MbrType::LINUX).unwrap();
    mbr.resize(ext, 30_000).unwrap();
    assert_eq!(mbr.entry(first).unwrap().len(), 1998);
    assert_eq!(mbr.entry(second).unwrap().flags(), PartitionFlags::BOOTABLE);
    mbr.remove(first).unwrap();
    assert_eq!(mbr.entry(4).unwrap().start(), 12_000);
    mbr.remove(4).unwrap();
    mbr.remove(ext).unwrap();
    assert_eq!(mbr.partitions().count(), 1);
    for _ in 0..3 {
        let start = 20_000 + mbr.partitions().count() as u64 * 1000;
        mbr.add(MbrEntry::new(MbrType::LINUX, start, 100)).unwrap();
    }
    assert_eq!(
        mbr.add(MbrEntry::new(MbrType::LINUX, 50_000, 100))
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
}

#[test]
fn edits_survive_a_write_and_read() {
    let mut dev = MemDevice::new(vec![0u8; 20_000 * 512], B512);
    let layout = DiskLayout::mbr()
        .with_alignment(Alignment::Block)
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Blocks(1000)))
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Blocks(1000)))
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Blocks(1000)))
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Blocks(1000)))
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Blocks(1000)));
    let disk = hadris_part::sync::create(&mut dev, &layout).unwrap();
    assert_eq!(disk.partitions().count(), 5);

    let mut disk = read(&mut dev).unwrap();
    let PartitionTable::Mbr(mbr) = disk.table_mut() else {
        panic!("expected an MBR");
    };
    let ext = mbr.extended().unwrap();
    mbr.resize(ext.index(), ext.len() + 5000).unwrap();
    mbr.add_logical(MbrEntry::new(MbrType::LINUX_SWAP, ext.end() + 100, 4000))
        .unwrap();
    mbr.remove(4).unwrap();
    write(&mut dev, &disk).unwrap();
    let back = read(&mut dev).unwrap();
    assert_eq!(
        back.partitions().collect::<Vec<_>>(),
        disk.partitions().collect::<Vec<_>>()
    );
    assert_eq!(back.partitions().count(), 5);
    assert_eq!(back.partition(4).unwrap().len(), 4000);
}

#[test]
fn hybrid_mirrors_are_checked() {
    let mut table = gpt();
    table
        .add(GptEntry::new(types::EFI_SYSTEM, guid(1), 100, 100))
        .unwrap();
    let mut config = HybridMbr::new().with_protective_slot(3);
    config
        .add_mirrored(0, MbrType::EFI_SYSTEM, PartitionFlags::BOOTABLE)
        .unwrap();
    let hybrid = Hybrid::new(table.clone(), &config).unwrap();
    let slots: Vec<_> = hybrid.mbr_partitions().map(|p| p.index()).collect();
    assert_eq!(slots, [0]);
    assert_eq!(hybrid.raw_entry(3).unwrap().kind, 0xEE);
    assert_eq!(hybrid.raw_entry(3).unwrap().sector_count(), 99);

    for _ in 0..2 {
        config
            .add_mirrored(0, MbrType::FAT32_LBA, PartitionFlags::empty())
            .unwrap();
    }
    assert_eq!(
        config
            .add_mirrored(0, MbrType::FAT32_LBA, PartitionFlags::empty())
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(config.mirrored_count(), 3);

    let mut bad = HybridMbr::new();
    bad.add_mirrored(5, MbrType::LINUX, PartitionFlags::empty())
        .unwrap();
    assert_eq!(
        Hybrid::new(table.clone(), &bad).unwrap_err().detail(),
        Detail::Mirror
    );
    let mut bad = HybridMbr::new();
    bad.add_mirrored(0, MbrType::EXTENDED, PartitionFlags::empty())
        .unwrap();
    assert_eq!(
        Hybrid::new(table.clone(), &bad).unwrap_err().detail(),
        Detail::Mirror
    );
    let mut bad = HybridMbr::new();
    bad.add_mirrored(0, MbrType::LINUX, PartitionFlags::REQUIRED)
        .unwrap();
    assert_eq!(
        Hybrid::new(table.clone(), &bad).unwrap_err().detail(),
        Detail::Flags
    );
    assert_eq!(
        Hybrid::new(table, &HybridMbr::new().with_protective_slot(4))
            .unwrap_err()
            .detail(),
        Detail::Mirror
    );
}

#[test]
fn layout_builder_rules() {
    let bs = B512;
    let gpt = DiskLayout::gpt(guid(7));

    let err = gpt
        .clone()
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Remaining))
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::MiB(1)))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Size);

    let err = gpt
        .clone()
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::GiB(1)))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NoSpace);

    let err = gpt
        .clone()
        .partition(PartitionSpec::new(MbrType::LINUX, Size::MiB(1)))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Kind);
    let err = DiskLayout::mbr()
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::MiB(1)))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Kind);
    let err = DiskLayout::mbr()
        .partition(PartitionSpec::new(MbrType::LINUX, Size::MiB(1)).with_name("x"))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Kind);
    let err = gpt
        .clone()
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::MiB(1)).with_mirror(MbrType::LINUX))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Mirror);

    let err = gpt
        .clone()
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::MiB(1)))
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::MiB(1)).with_start(2100))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Overlap { index: 1, other: 0 });
    let err = gpt
        .clone()
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Blocks(0)))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Size);
    let err = gpt
        .clone()
        .with_alignment(Alignment::Blocks(0))
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.detail(), Detail::Size);

    let disk = gpt
        .clone()
        .with_alignment(Alignment::Blocks(100))
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Bytes(1000)))
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::KiB(3)).with_unique_guid(guid(0xAB)))
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Blocks(5)).with_start(1000))
        .build(1 << 16, bs)
        .unwrap();
    let parts: Vec<_> = disk
        .partitions()
        .map(|p| (p.start(), p.len(), p.unique_guid().unwrap()))
        .collect();
    assert_eq!((parts[0].0, parts[0].1), (100, 2));
    assert_eq!((parts[1].0, parts[1].1, parts[1].2), (200, 6, guid(0xAB)));
    assert_eq!((parts[2].0, parts[2].1), (1000, 5));
    assert_eq!(
        disk,
        gpt.clone()
            .with_alignment(Alignment::Blocks(100))
            .partition(PartitionSpec::new(types::BASIC_DATA, Size::Bytes(1000)))
            .partition(
                PartitionSpec::new(types::BASIC_DATA, Size::KiB(3)).with_unique_guid(guid(0xAB))
            )
            .partition(PartitionSpec::new(types::BASIC_DATA, Size::Blocks(5)).with_start(1000))
            .build(1 << 16, bs)
            .unwrap()
    );

    let empty = DiskLayout::mbr().build(100, bs).unwrap();
    assert_eq!(empty.partitions().count(), 0);
    assert_eq!(Disk::new(Mbr::new(100, bs).unwrap()), empty);

    let err = DiskLayout::hybrid(guid(1))
        .partition(
            PartitionSpec::new(types::BASIC_DATA, Size::KiB(4)).with_mirror(MbrType::FAT32_LBA),
        )
        .partition(
            PartitionSpec::new(types::BASIC_DATA, Size::KiB(4)).with_mirror(MbrType::FAT32_LBA),
        )
        .partition(
            PartitionSpec::new(types::BASIC_DATA, Size::KiB(4)).with_mirror(MbrType::FAT32_LBA),
        )
        .partition(
            PartitionSpec::new(types::BASIC_DATA, Size::KiB(4)).with_mirror(MbrType::FAT32_LBA),
        )
        .build(1 << 16, bs)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn mbr_layouts_past_two_tebibytes_cap_remaining() {
    let blocks = (1u64 << 32) + 10_000;
    let disk = DiskLayout::mbr()
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Remaining))
        .build(blocks, B512)
        .unwrap();
    let p = disk.partition(0).unwrap();
    assert_eq!((p.start(), p.len()), (2048, u64::from(u32::MAX)));
    let err = DiskLayout::mbr()
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Blocks(1)).with_start(1 << 32))
        .build(blocks, B512)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::LimitExceeded);
}
