//! Layouts written to a device and read back, checked against the bytes the
//! specifications describe.

use hadris_part::gpt::types;
use hadris_part::sync::{create, open, read, write};
use hadris_part::{
    Alignment, DiskLayout, GptCopy, Guid, MbrType, PartitionFlags, PartitionKind, PartitionSpec,
    PartitionTable, Size,
};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

const B512: BlockSize = BlockSize::new(512).unwrap();
const B4K: BlockSize = BlockSize::new(4096).unwrap();

fn device(blocks: usize, size: BlockSize) -> MemDevice<Vec<u8>> {
    MemDevice::new(vec![0; blocks * size.get() as usize], size)
}

fn bytes(dev: &MemDevice<Vec<u8>>, offset: usize, len: usize) -> &[u8] {
    &dev.get_ref()[offset..offset + len]
}

fn le32(dev: &MemDevice<Vec<u8>>, offset: usize) -> u32 {
    u32::from_le_bytes(bytes(dev, offset, 4).try_into().unwrap())
}

fn le64(dev: &MemDevice<Vec<u8>>, offset: usize) -> u64 {
    u64::from_le_bytes(bytes(dev, offset, 8).try_into().unwrap())
}

/// Bitwise CRC32 (ISO-HDLC), independent of the crate's table-driven one.
fn reference_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn disk_guid() -> Guid {
    "11111111-2222-3333-4444-555555555555".parse().unwrap()
}

/// Checks one GPT header against the fields the UEFI specification lists,
/// computing both CRCs independently.
fn check_header(dev: &MemDevice<Vec<u8>>, lba: u64, alternate: u64, entries_lba: u64) {
    let block = dev.block_size().get() as usize;
    let at = lba as usize * block;
    assert_eq!(bytes(dev, at, 8), b"EFI PART");
    assert_eq!(le32(dev, at + 8), 0x0001_0000);
    let header_size = le32(dev, at + 12) as usize;
    assert_eq!(header_size, 92);
    let mut header = bytes(dev, at, header_size).to_vec();
    header[16..20].fill(0);
    assert_eq!(le32(dev, at + 16), reference_crc32(&header));
    assert_eq!(le64(dev, at + 24), lba);
    assert_eq!(le64(dev, at + 32), alternate);
    assert_eq!(le64(dev, at + 72), entries_lba);
    let count = le32(dev, at + 80) as usize;
    let size = le32(dev, at + 84) as usize;
    let array = bytes(dev, entries_lba as usize * block, count * size);
    assert_eq!(le32(dev, at + 88), reference_crc32(array));
    assert!(bytes(dev, at + 92, block - 92).iter().all(|&b| b == 0));
}

#[test]
fn mbr_layout_roundtrip() {
    let mut dev = device(1 << 15, B512);
    let layout = DiskLayout::mbr()
        .partition(
            PartitionSpec::new(MbrType::FAT32_LBA, Size::MiB(4))
                .with_flags(PartitionFlags::BOOTABLE),
        )
        .partition(PartitionSpec::new(MbrType::LINUX, Size::Remaining));
    let disk = create(&mut dev, &layout).unwrap();

    assert_eq!(bytes(&dev, 510, 2), [0x55, 0xAA]);
    let entry = bytes(&dev, 446, 16);
    assert_eq!(entry[0], 0x80);
    assert_eq!(entry[4], 0x0C);
    assert_eq!(&entry[1..4], &[32, 33, 0]);
    assert_eq!(le32(&dev, 446 + 8), 2048);
    assert_eq!(le32(&dev, 446 + 12), 8192);
    let second = bytes(&dev, 462, 16);
    assert_eq!((second[0], second[4]), (0, 0x83));
    assert_eq!(le32(&dev, 462 + 8), 10_240);
    assert_eq!(le32(&dev, 462 + 12), (1 << 15) - 10_240);
    assert!(bytes(&dev, 478, 32).iter().all(|&b| b == 0));

    let back = read(&mut dev).unwrap();
    assert_eq!(back, disk);
    let parts: Vec<_> = back.partitions().collect();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].kind(), PartitionKind::Mbr(MbrType::FAT32_LBA));
    assert_eq!(parts[0].flags(), PartitionFlags::BOOTABLE);
    assert_eq!(parts[0].size_bytes(), 4 << 20);
    assert_eq!(parts[1].unique_guid(), None);
    assert!(parts[1].name().is_none());
}

#[test]
fn gpt_layout_roundtrip() {
    let blocks = 1 << 16;
    let mut dev = device(blocks, B512);
    let layout = DiskLayout::gpt(disk_guid())
        .partition(
            PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(8))
                .with_name("EFI system")
                .with_flags(PartitionFlags::REQUIRED),
        )
        .partition(PartitionSpec::new(types::LINUX_SWAP, Size::Blocks(4096)))
        .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining).with_name("root"));
    let disk = create(&mut dev, &layout).unwrap();

    let last = blocks as u64 - 1;
    check_header(&dev, 1, last, 2);
    check_header(&dev, last, 1, last - 32);
    assert_eq!(le64(&dev, 512 + 40), 34);
    assert_eq!(le64(&dev, 512 + 48), last - 33);
    assert_eq!(bytes(&dev, 512 + 56, 16), disk_guid().to_bytes());
    assert_eq!(bytes(&dev, 1024, 16), types::EFI_SYSTEM.to_bytes());
    assert_eq!(le64(&dev, 1024 + 32), 2048);
    assert_eq!(le64(&dev, 1024 + 40), 2048 + 16_384 - 1);
    assert_eq!(le64(&dev, 1024 + 48), 1);
    assert_eq!(bytes(&dev, 1024 + 56, 4), [b'E', 0, b'F', 0]);
    assert_eq!(
        bytes(&dev, 2 * 512, 128 * 128),
        bytes(&dev, (last as usize - 32) * 512, 128 * 128)
    );

    let protective = bytes(&dev, 446, 16);
    assert_eq!(protective[4], 0xEE);
    assert_eq!(&protective[1..4], &[0, 2, 0]);
    assert_eq!(le32(&dev, 446 + 8), 1);
    assert_eq!(le32(&dev, 446 + 12), last as u32);

    let back = read(&mut dev).unwrap();
    assert_eq!(back, disk);
    let PartitionTable::Gpt(gpt) = back.table() else {
        panic!("expected a GPT");
    };
    assert_eq!(gpt.damaged_copy(), None);
    assert_eq!(gpt.disk_guid(), disk_guid());
    let parts: Vec<_> = back.partitions().collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].name().unwrap().to_string(), "EFI system");
    assert_eq!(parts[0].flags(), PartitionFlags::REQUIRED);
    assert_eq!(parts[1].start(), 2048 + 16_384);
    assert_eq!(parts[2].end(), last - 32);
    let guids: Vec<_> = parts.iter().map(|p| p.unique_guid().unwrap()).collect();
    assert_ne!(guids[0], guids[1]);
    assert_eq!(layout.build(blocks as u64, B512).unwrap(), disk);
}

#[test]
fn hybrid_layout_roundtrip() {
    let mut dev = device(1 << 15, B512);
    let layout = DiskLayout::hybrid(disk_guid())
        .partition(
            PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(2))
                .with_mirror(MbrType::EFI_SYSTEM)
                .with_flags(PartitionFlags::BOOTABLE),
        )
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Remaining));
    let disk = create(&mut dev, &layout).unwrap();

    let protective = bytes(&dev, 446, 16);
    assert_eq!(protective[4], 0xEE);
    assert_eq!(le32(&dev, 446 + 8), 1);
    assert_eq!(le32(&dev, 446 + 12), 2047);
    let mirror = bytes(&dev, 462, 16);
    assert_eq!((mirror[0], mirror[4]), (0x80, 0xEF));
    assert_eq!(le32(&dev, 462 + 8), 2048);
    assert_eq!(le32(&dev, 462 + 12), 4096);

    let back = read(&mut dev).unwrap();
    assert_eq!(back, disk);
    let PartitionTable::Hybrid(hybrid) = back.table() else {
        panic!("expected a hybrid table");
    };
    let mirrors: Vec<_> = hybrid.mbr_partitions().collect();
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].index(), 1);
    assert_eq!(mirrors[0].start(), 2048);
    assert_eq!(back.partitions().count(), 2);
}

#[test]
fn ebr_chain_roundtrip() {
    let mut dev = device(1 << 16, B512);
    let mut layout = DiskLayout::mbr().with_alignment(Alignment::Blocks(64));
    for (i, kind) in [
        MbrType::FAT32_LBA,
        MbrType::LINUX,
        MbrType::LINUX_SWAP,
        MbrType::LINUX,
        MbrType::NTFS,
        MbrType::LINUX,
    ]
    .into_iter()
    .enumerate()
    {
        let size = if i == 5 {
            Size::Remaining
        } else {
            Size::Blocks(1000)
        };
        layout = layout.partition(PartitionSpec::new(kind, size));
    }
    let disk = create(&mut dev, &layout).unwrap();

    let PartitionTable::Mbr(mbr) = disk.table() else {
        panic!("expected an MBR");
    };
    let extended = mbr.extended().unwrap();
    assert_eq!(extended.index(), 3);
    assert_eq!(extended.kind(), PartitionKind::Mbr(MbrType::EXTENDED_LBA));
    let parts: Vec<_> = disk.partitions().collect();
    assert_eq!(
        parts.iter().map(|p| p.index()).collect::<Vec<_>>(),
        [0, 1, 2, 4, 5, 6]
    );

    let ext_start = extended.start() as usize;
    let mut ebr = ext_start;
    for (k, logical) in parts[3..].iter().enumerate() {
        let at = ebr * 512;
        assert_eq!(bytes(&dev, at + 510, 2), [0x55, 0xAA], "EBR {k}");
        let rel = le32(&dev, at + 446 + 8) as usize;
        assert_eq!(ebr + rel, logical.start() as usize);
        assert_eq!(u64::from(le32(&dev, at + 446 + 12)), logical.len());
        let link = bytes(&dev, at + 462, 16);
        if k + 1 < 3 {
            assert_eq!(link[4], 0x05);
            let next = ext_start + le32(&dev, at + 462 + 8) as usize;
            let next_logical = &parts[3 + k + 1];
            assert_eq!(
                next as u64 + u64::from(le32(&dev, at + 462 + 12)),
                next_logical.end()
            );
            ebr = next;
        } else {
            assert!(link.iter().all(|&b| b == 0));
        }
    }

    let back = read(&mut dev).unwrap();
    assert_eq!(back, disk);
    assert_eq!(back.partitions().collect::<Vec<_>>(), parts);
}

#[test]
fn utf16_names_roundtrip() {
    let mut dev = device(1 << 14, B512);
    let names = [
        "Système",
        "\u{1F4BE} backup",
        "ÜÖÄ-αβγ-中文",
        &"x".repeat(36),
    ];
    let mut layout = DiskLayout::gpt(disk_guid()).with_alignment(Alignment::Block);
    for name in names {
        layout = layout
            .partition(PartitionSpec::new(types::BASIC_DATA, Size::Blocks(8)).with_name(name));
    }
    create(&mut dev, &layout).unwrap();

    let emoji_at = 1024 + 128 + 56;
    assert_eq!(bytes(&dev, emoji_at, 4), [0x3D, 0xD8, 0xBE, 0xDC]);
    let back = read(&mut dev).unwrap();
    let read_names: Vec<String> = back
        .partitions()
        .map(|p| p.name().unwrap().to_string())
        .collect();
    assert_eq!(read_names, names);
    let mut buf = [0u8; 108];
    let last = back.partition(3).unwrap();
    assert_eq!(last.name().unwrap().to_str(&mut buf).unwrap(), names[3]);

    let too_long = DiskLayout::gpt(disk_guid()).partition(
        PartitionSpec::new(types::BASIC_DATA, Size::Blocks(8)).with_name(&"y".repeat(37)),
    );
    assert_eq!(
        too_long.build(1 << 14, B512).unwrap_err().kind(),
        hadris_fs::ErrorKind::NameTooLong
    );
}

#[test]
fn four_kib_blocks_roundtrip() {
    let blocks = 4096;
    let mut dev = device(blocks, B4K);
    let layout = DiskLayout::gpt(disk_guid())
        .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::MiB(4)))
        .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining));
    let disk = create(&mut dev, &layout).unwrap();
    let last = blocks as u64 - 1;
    check_header(&dev, 1, last, 2);
    check_header(&dev, last, 1, last - 4);
    let esp = disk.partition(0).unwrap();
    assert_eq!((esp.start(), esp.len()), (256, 1024));
    assert_eq!(read(&mut dev).unwrap(), disk);

    let mut small = device(blocks, B512);
    assert_eq!(
        write(&mut small, &disk).unwrap_err().kind(),
        hadris_fs::ErrorKind::InvalidInput
    );
}

#[test]
fn open_gives_a_slice_of_the_partition() {
    let mut dev = device(1 << 14, B512);
    let layout = DiskLayout::gpt(disk_guid())
        .partition(PartitionSpec::new(types::BASIC_DATA, Size::Blocks(100)));
    let disk = create(&mut dev, &layout).unwrap();
    let part = disk.partition(0).unwrap();
    {
        let mut slice = open(&mut dev, &part).unwrap();
        assert_eq!(slice.block_count(), 100);
        slice
            .write_blocks(BlockIndex::new(99), &[0xA5; 512])
            .unwrap();
        assert!(slice.write_blocks(BlockIndex::new(100), &[0; 512]).is_err());
    }
    let at = (part.start() as usize + 99) * 512;
    assert_eq!(bytes(&dev, at, 512), [0xA5; 512]);

    let mut short = device(1000, B512);
    assert_eq!(
        open(&mut short, &disk.partition(0).unwrap())
            .unwrap_err()
            .detail(),
        Some(hadris_part::Detail::OutOfBounds { index: 0 })
    );
}

#[test]
fn bootstrap_and_disk_signature_survive_a_rewrite() {
    let mut dev = device(1 << 12, B512);
    let layout = DiskLayout::mbr().partition(PartitionSpec::new(MbrType::LINUX, Size::Remaining));
    let mut disk = layout.build(1 << 12, B512).unwrap();
    let mut code = [0x90u8; 444];
    code[440..].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    disk.set_bootstrap(&code).unwrap();
    write(&mut dev, &disk).unwrap();
    assert_eq!(bytes(&dev, 0, 444), code);
    assert_eq!(le32(&dev, 440), 0xDEAD_BEEF);

    let back = read(&mut dev).unwrap();
    assert_eq!(&back.bootstrap()[..444], &code);
    assert!(disk.set_bootstrap(&[0; 447]).is_err());
}

#[test]
fn runs_are_whole_blocks_and_end_with_block_zero() {
    for size in [B512, B4K] {
        let disk = DiskLayout::gpt(disk_guid())
            .partition(PartitionSpec::new(types::BASIC_DATA, Size::Remaining))
            .build(4096, size)
            .unwrap();
        let runs: Vec<_> = disk.runs().collect();
        assert_eq!(runs.len(), 5);
        for run in &runs {
            assert_eq!(run.bytes().len() % size.get() as usize, 0);
        }
        assert_eq!(runs.last().unwrap().lba(), 0);
        assert_eq!(runs[1].lba(), 4095);
    }
    let gpt = DiskLayout::gpt(disk_guid()).build(4096, B512).unwrap();
    let PartitionTable::Gpt(gpt) = gpt.table() else {
        panic!()
    };
    assert_eq!(gpt.damaged_copy(), None::<GptCopy>);
}
