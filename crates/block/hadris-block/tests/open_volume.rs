use hadris_block::detect::{BlockFormat, FatVariant, PartitionTableKind};
use hadris_block::partition::sync::{gpt_partition, mbr_partition};
use hadris_block::sync::OpenVolume;
use hadris_block::{Error, part};
use hadris_fat::{FatKind, FormatOptions};
use hadris_storage::sync::{BlockDevice, Slice};
use hadris_storage::{BlockIndex, BlockSize, MemDevice};

const VOLUME_LEN: usize = 2 * 1024 * 1024;
const BLOCK: BlockSize = match BlockSize::new(512) {
    Some(size) => size,
    None => panic!(),
};

fn device(bytes: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(bytes, BLOCK)
}

fn format_fat12<D: BlockDevice>(dev: D) -> D {
    let options = FormatOptions::new().with_kind(FatKind::Fat12);
    hadris_fat::sync::format(dev, options).unwrap().into_inner()
}

#[test]
fn opens_detected_fat_and_returns_the_device() {
    let mut dev = format_fat12(device(vec![0_u8; VOLUME_LEN]));

    let volume = OpenVolume::open(&mut dev).unwrap();
    assert_eq!(volume.format(), FatVariant::Fat12);
    assert!(volume.as_fat().is_some());
    let dev = volume.into_inner();
    assert_eq!(dev.get_ref().len(), VOLUME_LEN);
}

#[test]
fn opens_fat_inside_mbr_partition() {
    let start_lba = 1_u32;
    let sector_count = (VOLUME_LEN / 512) as u32;
    let entry = part::MbrPartition::new(part::MbrPartitionType::Fat12, start_lba, sector_count);
    let mut table = part::MbrPartitionTable::new();
    table.partitions[0] = entry;
    let mbr = part::MasterBootRecord::new(table);

    let mut bytes = vec![0_u8; VOLUME_LEN + 512];
    bytes[..512].copy_from_slice(bytemuck::bytes_of(&mbr));
    let mut disk = device(bytes);
    format_fat12(mbr_partition(&mut disk, &entry).unwrap());

    assert_eq!(
        hadris_block::detect::sync::detect(&mut disk).unwrap(),
        Some(BlockFormat::PartitionTable(PartitionTableKind::Mbr))
    );
    assert!(matches!(
        OpenVolume::open(&mut disk),
        Err(Error::PartitionedDisk(PartitionTableKind::Mbr))
    ));

    let volume = OpenVolume::open(mbr_partition(&mut disk, &entry).unwrap()).unwrap();
    assert_eq!(volume.format(), FatVariant::Fat12);
}

#[test]
fn opens_fat_inside_gpt_partition() {
    let start_lba = 2048_u64;
    let sector_count = (VOLUME_LEN / 512) as u64;
    let entry = part::GptPartitionEntry::new(
        part::Guid::EFI_SYSTEM,
        part::Guid::UNUSED,
        start_lba,
        start_lba + sector_count - 1,
    );
    let mut disk = device(vec![0_u8; (start_lba as usize * 512) + VOLUME_LEN]);
    format_fat12(gpt_partition(&mut disk, &entry).unwrap());

    let volume = OpenVolume::open(gpt_partition(&mut disk, &entry).unwrap()).unwrap();
    assert_eq!(volume.format(), FatVariant::Fat12);
    let slice: Slice<_> = volume.into_inner();
    assert_eq!(slice.first(), BlockIndex(start_lba));
}

#[test]
fn partitions_past_the_disk_are_refused() {
    let entry = part::MbrPartition::new(part::MbrPartitionType::Fat12, 8, 16);
    let mut disk = device(vec![0_u8; 16 * 512]);
    assert!(mbr_partition(&mut disk, &entry).is_err());
}

#[test]
fn rejects_unknown_and_mismatched_formats() {
    assert!(matches!(
        OpenVolume::open(device(vec![0_u8; 1024])),
        Err(Error::UnknownFormat)
    ));

    let dev = format_fat12(device(vec![0_u8; VOLUME_LEN]));
    assert!(matches!(
        OpenVolume::open_detected(dev, FatVariant::Fat16),
        Err(Error::DetectedFormatMismatch {
            detected: FatVariant::Fat16,
            opened: FatVariant::Fat12,
        })
    ));
}

#[test]
fn detects_exfat_but_rejects_unified_opening() {
    let mut image = vec![0_u8; 512];
    image[3..11].copy_from_slice(b"EXFAT   ");
    image[510..512].copy_from_slice(&[0x55, 0xaa]);

    assert!(matches!(
        OpenVolume::open(device(image)),
        Err(Error::UnsupportedFormat(BlockFormat::Fat(
            FatVariant::ExFat
        )))
    ));
}

#[test]
fn devices_smaller_than_a_block_are_unknown() {
    let dev = MemDevice::new(vec![0_u8; 512], BlockSize::new(4096).unwrap());
    assert!(matches!(OpenVolume::open(dev), Err(Error::UnknownFormat)));
}
