//! Reading damaged, crafted and hostile disks: the backup GPT fallback, EBR
//! chains, and inputs found by fuzzing and audits. Every case returns an
//! error or a table, never a panic.

use core::ops::ControlFlow;

use hadris_fs::ErrorKind;
use hadris_part::gpt::types;
use hadris_part::sync::{create, read, scan, write};
use hadris_part::{
    Detail, DiskLayout, GptCopy, Guid, MbrType, PartitionKind, PartitionSpec, PartitionTable, Size,
    TableKind,
};
use hadris_storage::{BlockSize, MemDevice};

const B512: BlockSize = BlockSize::new(512).unwrap();

type Dev = MemDevice<Vec<u8>>;

fn device(bytes: Vec<u8>) -> Dev {
    MemDevice::new(bytes, B512)
}

fn crc32(data: &[u8]) -> u32 {
    hadris_part::raw::crc32(data)
}

fn gpt_disk(blocks: usize) -> Dev {
    let mut dev = device(vec![0; blocks * 512]);
    let layout = DiskLayout::gpt(Guid::from_bytes([0x42; 16]))
        .with_alignment(hadris_part::Alignment::Block)
        .partition(PartitionSpec::new(types::EFI_SYSTEM, Size::Blocks(100)).with_name("EFI"))
        .partition(PartitionSpec::new(types::LINUX_FILESYSTEM, Size::Remaining));
    create(&mut dev, &layout).unwrap();
    dev
}

fn gpt(dev: &mut Dev) -> hadris_part::Gpt {
    match read(dev).unwrap().into_table() {
        PartitionTable::Gpt(gpt) => gpt,
        other => panic!("expected a GPT, got {other:?}"),
    }
}

#[test]
fn backup_gpt_replaces_a_corrupt_primary() {
    for corruption in [512usize, 512 + 16, 512 + 88, 1024 + 40, 1024 + 128 * 127] {
        let mut dev = gpt_disk(4096);
        let clean = read(&mut dev).unwrap();
        dev.get_mut()[corruption] ^= 0x01;

        let disk = read(&mut dev).unwrap();
        let PartitionTable::Gpt(table) = disk.table() else {
            panic!("expected a GPT");
        };
        assert_eq!(
            table.damaged_copy(),
            Some(GptCopy::Primary),
            "at {corruption}"
        );
        assert_eq!(
            disk.partitions().collect::<Vec<_>>(),
            clean.partitions().collect::<Vec<_>>()
        );

        write(&mut dev, &disk).unwrap();
        assert_eq!(gpt(&mut dev).damaged_copy(), None);
        assert_eq!(read(&mut dev).unwrap(), clean);
    }
}

#[test]
fn a_corrupt_backup_is_reported_and_repaired() {
    let mut dev = gpt_disk(4096);
    let clean = read(&mut dev).unwrap();
    dev.get_mut()[4095 * 512 + 24] ^= 0x01;
    let damaged = read(&mut dev).unwrap();
    let PartitionTable::Gpt(table) = damaged.table() else {
        panic!("expected a GPT");
    };
    assert_eq!(table.damaged_copy(), Some(GptCopy::Backup));
    write(&mut dev, &damaged).unwrap();
    assert_eq!(read(&mut dev).unwrap(), clean);

    let mut truncated = device(gpt_disk(4096).into_inner()[..4095 * 512].to_vec());
    let table = gpt(&mut truncated);
    assert_eq!(table.damaged_copy(), Some(GptCopy::Backup));
    assert_eq!(table.backup_lba(), 4094);
    assert_eq!(table.partitions().count(), 2);
    let disk = read(&mut truncated).unwrap();
    assert_eq!(
        write(&mut truncated, &disk).unwrap_err().kind(),
        ErrorKind::NoSpace
    );
}

#[test]
fn a_backup_that_disagrees_counts_as_damaged() {
    let mut dev = gpt_disk(4096);
    let header = 4095 * 512;
    let bytes = dev.get_mut();
    bytes[header + 56] ^= 0xFF;
    bytes[header + 16..header + 20].fill(0);
    let fixed = crc32(&bytes[header..header + 92]);
    bytes[header + 16..header + 20].copy_from_slice(&fixed.to_le_bytes());
    assert_eq!(gpt(&mut dev).damaged_copy(), Some(GptCopy::Backup));
}

#[test]
fn both_copies_damaged_is_corrupt() {
    let mut dev = gpt_disk(4096);
    dev.get_mut()[512] = b'X';
    dev.get_mut()[4095 * 512] = b'X';
    let err = read(&mut dev).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
    assert_eq!(Detail::of(&err), Some(Detail::GptHeader));

    let mut dev = gpt_disk(4096);
    dev.get_mut()[1024] ^= 1;
    dev.get_mut()[(4095 - 32) * 512] ^= 1;
    assert_eq!(
        Detail::of(&read(&mut dev).unwrap_err()),
        Some(Detail::GptEntriesCrc)
    );
}

#[test]
fn block_zero_without_a_signature_has_no_table() {
    let mut dev = device(vec![0; 4096]);
    let err = read(&mut dev).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotRecognized);
    assert_eq!(Detail::of(&err), Some(Detail::NoTable));

    let mut short = device(vec![0; 100]);
    assert_eq!(
        read(&mut short).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn invalid_boot_indicators_and_double_extended_partitions_are_corrupt() {
    let mut image = vec![0u8; 4096];
    image[510..512].copy_from_slice(&[0x55, 0xAA]);
    image[446] = 0x01;
    image[446 + 4] = 0x83;
    assert_eq!(
        Detail::of(&read(&mut device(image.clone())).unwrap_err()),
        Some(Detail::MbrEntry)
    );
    image[446] = 0;
    image[446 + 4] = 0x05;
    assert_eq!(
        Detail::of(&read(&mut device(image.clone())).unwrap_err()),
        Some(Detail::EbrChain)
    );
    image[446 + 8] = 1;
    image[462 + 4] = 0x0F;
    image[462 + 8] = 2;
    assert_eq!(
        Detail::of(&read(&mut device(image)).unwrap_err()),
        Some(Detail::MbrEntry)
    );
}

fn mbr_entry(image: &mut [u8], at: usize, kind: u8, start: u32, count: u32) {
    image[at + 4] = kind;
    image[at + 8..at + 12].copy_from_slice(&start.to_le_bytes());
    image[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
}

/// An extended partition at block 100 with logical partitions at 110 and
/// 210, the second EBR at 200, written by hand.
fn ebr_image() -> Vec<u8> {
    let mut image = vec![0u8; 512 * 400];
    mbr_entry(&mut image, 446, 0x0C, 10, 50);
    mbr_entry(&mut image, 462, 0x0F, 100, 300);
    image[510..512].copy_from_slice(&[0x55, 0xAA]);
    let first = 100 * 512;
    mbr_entry(&mut image, first + 446, 0x83, 10, 20);
    mbr_entry(&mut image, first + 462, 0x05, 100, 40);
    image[first + 510..first + 512].copy_from_slice(&[0x55, 0xAA]);
    let second = 200 * 512;
    mbr_entry(&mut image, second + 446, 0x82, 10, 30);
    image[second + 510..second + 512].copy_from_slice(&[0x55, 0xAA]);
    image
}

#[test]
fn ebr_chain_is_followed() {
    let mut dev = device(ebr_image());
    let disk = read(&mut dev).unwrap();
    let parts: Vec<_> = disk
        .partitions()
        .map(|p| (p.index(), p.start(), p.len(), p.kind()))
        .collect();
    assert_eq!(
        parts,
        [
            (0, 10, 50, PartitionKind::Mbr(MbrType::FAT32_LBA)),
            (4, 110, 20, PartitionKind::Mbr(MbrType::LINUX)),
            (5, 210, 30, PartitionKind::Mbr(MbrType::LINUX_SWAP)),
        ]
    );

    let mut scanned = Vec::new();
    let kind = scan(&mut dev, |p| {
        scanned.push((p.index(), p.start(), p.len(), p.kind()));
        ControlFlow::Continue(())
    })
    .unwrap();
    assert_eq!(kind, TableKind::Mbr);
    assert_eq!(scanned, parts);

    write(&mut dev, &disk).unwrap();
    assert_eq!(read(&mut dev).unwrap(), disk);
    let original = ebr_image();
    for at in [446, 462, 100 * 512 + 446, 100 * 512 + 462, 200 * 512 + 446] {
        assert_eq!(dev.get_ref()[at + 4], original[at + 4], "type at {at}");
        assert_eq!(
            dev.get_ref()[at + 8..at + 16],
            original[at + 8..at + 16],
            "at {at}"
        );
    }
}

#[test]
fn broken_ebr_chains_are_corrupt() {
    let cases: [(usize, u32); 4] = [
        (100 * 512 + 462 + 8, 0),
        (100 * 512 + 462 + 8, 300),
        (200 * 512 + 510, 0),
        (100 * 512 + 446 + 4, 0x05),
    ];
    for (at, value) in cases {
        let mut image = ebr_image();
        if matches!(at % 512, 450 | 510) {
            image[at] = value as u8;
        } else {
            image[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let err = read(&mut device(image)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt, "at {at}");
        assert_eq!(Detail::of(&err), Some(Detail::EbrChain), "at {at}");
    }

    let mut unsigned = ebr_image();
    unsigned[100 * 512 + 510] = 0;
    let disk = read(&mut device(unsigned)).unwrap();
    assert_eq!(disk.partitions().count(), 1);
}

/// A GPT image written by hand: a protective MBR, the primary header at 1
/// with `count` entries of `size` bytes from block 2, and a matching backup
/// in the last block, all with valid CRCs.
fn crafted_gpt(blocks: u64, count: u32, size: u32, entries: &[(usize, [u8; 128])]) -> Vec<u8> {
    let mut image = vec![0u8; blocks as usize * 512];
    mbr_entry(&mut image, 446, 0xEE, 1, blocks as u32 - 1);
    image[510..512].copy_from_slice(&[0x55, 0xAA]);
    let array_bytes = count as usize * size as usize;
    let array_blocks = array_bytes.div_ceil(512) as u64;
    let mut array = vec![0u8; array_bytes];
    for (slot, entry) in entries {
        let at = slot * size as usize;
        array[at..at + 128].copy_from_slice(entry);
    }
    let backup_array = blocks - 1 - array_blocks;
    image[1024..1024 + array_bytes].copy_from_slice(&array);
    let at = backup_array as usize * 512;
    image[at..at + array_bytes].copy_from_slice(&array);
    let array_crc = crc32(&array);
    for (lba, alternate, entries_lba) in [(1, blocks - 1, 2), (blocks - 1, 1, backup_array)] {
        let h = lba as usize * 512;
        image[h..h + 8].copy_from_slice(b"EFI PART");
        image[h + 8..h + 12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        image[h + 12..h + 16].copy_from_slice(&92u32.to_le_bytes());
        image[h + 24..h + 32].copy_from_slice(&lba.to_le_bytes());
        image[h + 32..h + 40].copy_from_slice(&alternate.to_le_bytes());
        image[h + 40..h + 48].copy_from_slice(&(2 + array_blocks).to_le_bytes());
        image[h + 48..h + 56].copy_from_slice(&(blocks - 2 - array_blocks).to_le_bytes());
        image[h + 56..h + 72].copy_from_slice(&[0x5A; 16]);
        image[h + 72..h + 80].copy_from_slice(&entries_lba.to_le_bytes());
        image[h + 80..h + 84].copy_from_slice(&count.to_le_bytes());
        image[h + 84..h + 88].copy_from_slice(&size.to_le_bytes());
        image[h + 88..h + 92].copy_from_slice(&array_crc.to_le_bytes());
        let crc = crc32(&image[h..h + 92]);
        image[h + 16..h + 20].copy_from_slice(&crc.to_le_bytes());
    }
    image
}

fn entry(type_byte: u8, first: u64, last: u64) -> [u8; 128] {
    let mut e = [0u8; 128];
    e[..16].fill(type_byte);
    e[16..32].fill(0xDC);
    e[32..40].copy_from_slice(&first.to_le_bytes());
    e[40..48].copy_from_slice(&last.to_le_bytes());
    e
}

#[test]
fn crafted_gpt_parses_and_large_entries_are_supported() {
    let image = crafted_gpt(100, 4, 128, &[(1, entry(0xAF, 34, 66))]);
    let mut dev = device(image);
    let disk = read(&mut dev).unwrap();
    let p = disk.partition(0).unwrap();
    assert_eq!(
        (p.index(), p.start(), p.len(), p.size_bytes()),
        (1, 34, 33, 33 * 512)
    );

    let image = crafted_gpt(
        100,
        8,
        256,
        &[(0, entry(0xAF, 40, 49)), (5, entry(0xBF, 50, 59))],
    );
    let mut dev = device(image);
    let disk = read(&mut dev).unwrap();
    let starts: Vec<_> = disk.partitions().map(|p| (p.index(), p.start())).collect();
    assert_eq!(starts, [(0, 40), (5, 50)]);
    write(&mut dev, &disk).unwrap();
    assert_eq!(read(&mut dev).unwrap(), disk);
    assert_eq!(gpt(&mut dev).damaged_copy(), None);
}

#[test]
fn full_range_and_inverted_entries_saturate() {
    let image = crafted_gpt(
        100,
        4,
        128,
        &[(0, entry(0xAF, 0, u64::MAX)), (1, entry(0xBF, 60, 50))],
    );
    let disk = read(&mut device(image)).unwrap();
    let parts: Vec<_> = disk.partitions().collect();
    assert_eq!(parts[0].len(), u64::MAX);
    assert_eq!(parts[0].size_bytes(), u64::MAX);
    assert_eq!(parts[0].end(), u64::MAX);
    assert_eq!(parts[1].len(), 0);
    assert!(parts[1].is_empty());
}

#[test]
fn hostile_entry_arrays_are_rejected_without_allocating() {
    for (count, size) in [(u32::MAX, 128u32), (4, 100), (4, 0), (4, 384)] {
        let mut image = crafted_gpt(100, 4, 128, &[]);
        for h in [512usize, 99 * 512] {
            image[h + 80..h + 84].copy_from_slice(&count.to_le_bytes());
            image[h + 84..h + 88].copy_from_slice(&size.to_le_bytes());
            image[h + 16..h + 20].fill(0);
            let crc = crc32(&image[h..h + 92]);
            image[h + 16..h + 20].copy_from_slice(&crc.to_le_bytes());
        }
        let err = read(&mut device(image)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Corrupt);
        assert_eq!(
            Detail::of(&err),
            Some(Detail::GptEntries),
            "{count} x {size}"
        );
    }

    for field in [72usize, 32, 40, 48] {
        let mut image = crafted_gpt(100, 4, 128, &[]);
        let h = 512;
        image[h + field..h + field + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        image[h + 16..h + 20].fill(0);
        let crc = crc32(&image[h..h + 92]);
        image[h + 16..h + 20].copy_from_slice(&crc.to_le_bytes());
        let table = gpt(&mut device(image));
        if field == 32 {
            assert_eq!(table.damaged_copy(), Some(GptCopy::Backup));
        } else {
            assert_eq!(
                table.damaged_copy(),
                Some(GptCopy::Primary),
                "field {field}"
            );
        }
    }
}

#[test]
fn mbr_entries_past_32_bits_saturate() {
    let mut image = vec![0u8; 512];
    mbr_entry(&mut image, 446, 0x83, 0xFFFF_FF00, 0xFFFF_FFFF);
    image[510..512].copy_from_slice(&[0x55, 0xAA]);
    let disk = read(&mut device(image)).unwrap();
    let p = disk.partition(0).unwrap();
    assert_eq!(p.end(), 0xFFFF_FF00 + 0xFFFF_FFFF);
    let mut dev = device(vec![0; 512]);
    assert!(hadris_part::sync::open(&mut dev, &p).is_err());
}

#[test]
fn unsupported_block_sizes_are_refused() {
    let mut dev = MemDevice::new(vec![0u8; 520 * 8], BlockSize::new(520).unwrap());
    let err = read(&mut dev).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert_eq!(Detail::of(&err), Some(Detail::BlockSize));
    let mut dev = MemDevice::new(vec![0u8; 256 * 8], BlockSize::new(256).unwrap());
    assert_eq!(read(&mut dev).unwrap_err().kind(), ErrorKind::Unsupported);

    let mut big = MemDevice::new(vec![0u8; 8192 * 8], BlockSize::new(8192).unwrap());
    assert_eq!(
        scan(&mut big, |_| ControlFlow::Continue(()))
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn scan_matches_read_and_stops_on_break() {
    let mut dev = gpt_disk(4096);
    let listed: Vec<_> = read(&mut dev).unwrap().partitions().collect();
    let mut scanned = Vec::new();
    let kind = scan(&mut dev, |p| {
        scanned.push(p);
        ControlFlow::Continue(())
    })
    .unwrap();
    assert_eq!(kind, TableKind::Gpt);
    assert_eq!(scanned, listed);

    let mut first = None;
    scan(&mut dev, |p| {
        first = Some(p);
        ControlFlow::Break(())
    })
    .unwrap();
    assert_eq!(first, Some(listed[0]));

    dev.get_mut()[512] ^= 1;
    let mut recovered = 0;
    scan(&mut dev, |_| {
        recovered += 1;
        ControlFlow::Continue(())
    })
    .unwrap();
    assert_eq!(recovered, 2);
}

#[test]
fn errors_convert_to_io_errors_with_their_detail() {
    let err = read(&mut device(vec![0; 1024])).unwrap_err();
    let io: std::io::Error = err.into();
    assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(io.to_string(), "no partition table signature");

    let err = read(&mut device(vec![0; 10])).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.device_error().is_none());
}

#[test]
fn rewriting_keeps_where_the_backup_array_lives() {
    let mut image = crafted_gpt(100, 4, 128, &[(0, entry(0xAF, 10, 20))]);
    let array = image[98 * 512..99 * 512].to_vec();
    image[98 * 512..99 * 512].fill(0);
    image[95 * 512..96 * 512].copy_from_slice(&array);
    for (h, entries) in [(512usize, None), (99 * 512, Some(95u64))] {
        image[h + 48..h + 56].copy_from_slice(&90u64.to_le_bytes());
        if let Some(lba) = entries {
            image[h + 72..h + 80].copy_from_slice(&lba.to_le_bytes());
        }
        image[h + 16..h + 20].fill(0);
        let crc = crc32(&image[h..h + 92]);
        image[h + 16..h + 20].copy_from_slice(&crc.to_le_bytes());
    }
    let mut dev = device(image);
    let disk = read(&mut dev).unwrap();
    assert_eq!(gpt(&mut dev).damaged_copy(), None);

    let mut copy = device(vec![0; 100 * 512]);
    write(&mut copy, &disk).unwrap();
    let bytes = copy.get_ref();
    let backup_entries = &bytes[99 * 512 + 72..99 * 512 + 80];
    assert_eq!(u64::from_le_bytes(backup_entries.try_into().unwrap()), 95);
    assert_eq!(bytes[95 * 512..96 * 512], array[..]);
    assert_eq!(read(&mut copy).unwrap(), disk);
}

fn patch_header(image: &mut [u8], h: usize, field: usize, value: u64) {
    image[h + field..h + field + 8].copy_from_slice(&value.to_le_bytes());
    image[h + 16..h + 20].fill(0);
    let crc = crc32(&image[h..h + 92]);
    image[h + 16..h + 20].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn a_primary_naming_itself_as_backup_has_a_damaged_backup() {
    let mut image = crafted_gpt(100, 4, 128, &[(0, entry(0xAF, 10, 20))]);
    patch_header(&mut image, 512, 32, 1);
    let mut dev = device(image);
    let disk = read(&mut dev).unwrap();
    let PartitionTable::Gpt(table) = disk.table() else {
        panic!("expected a GPT");
    };
    assert_eq!(table.damaged_copy(), Some(GptCopy::Backup));
    assert_eq!(table.backup_lba(), 99);
    write(&mut dev, &disk).unwrap();
    assert_eq!(gpt(&mut dev).damaged_copy(), None);
}

#[test]
fn a_backup_whose_primary_array_cannot_fit_is_not_used() {
    let mut image = crafted_gpt(100, 4, 128, &[(0, entry(0xAF, 10, 20))]);
    image[512] = b'X';
    patch_header(&mut image, 99 * 512, 40, 2);
    let err = read(&mut device(image)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
}
