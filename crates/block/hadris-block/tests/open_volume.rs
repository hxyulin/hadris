use hadris_block::detect::{BlockFormat, FatVariant, PartitionTableKind};
use hadris_block::partition::sync::{gpt_partition, mbr_partition};
use hadris_block::sync::OpenVolume;
use hadris_block::{Error, OpenError, part};
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

/// The reason an open failed, and the device it gave back.
fn failure<T, D, E>(result: Result<T, OpenError<D, E>>) -> (Error<E>, D) {
    let Err(err) = result else {
        panic!("the volume opened");
    };
    let (error, dev) = err.into_parts();
    (error, dev)
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
    let (error, _) = failure(OpenVolume::open(&mut disk));
    assert!(matches!(
        error,
        Error::PartitionedDisk(PartitionTableKind::Mbr)
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
    assert_eq!(slice.first(), BlockIndex::new(start_lba));
}

#[test]
fn partitions_past_the_disk_are_refused() {
    let entry = part::MbrPartition::new(part::MbrPartitionType::Fat12, 8, 16);
    let mut disk = device(vec![0_u8; 16 * 512]);
    assert!(mbr_partition(&mut disk, &entry).is_err());
}

#[test]
fn rejects_unknown_and_mismatched_formats() {
    let (error, dev) = failure(OpenVolume::open(device(vec![0_u8; 1024])));
    assert!(matches!(error, Error::UnknownFormat));
    assert_eq!(dev.get_ref().len(), 1024);

    let dev = format_fat12(device(vec![0_u8; VOLUME_LEN]));
    let (error, dev) = failure(OpenVolume::open_detected(dev, FatVariant::Fat16));
    assert!(matches!(
        error,
        Error::DetectedFormatMismatch {
            detected: FatVariant::Fat16,
            opened: FatVariant::Fat12,
        }
    ));
    assert_eq!(dev.get_ref().len(), VOLUME_LEN);
    assert_eq!(
        OpenVolume::open(dev).unwrap().format(),
        FatVariant::Fat12,
        "the returned device is untouched"
    );
}

#[test]
fn a_volume_that_fails_to_mount_gives_the_device_back() {
    let mut image = format_fat12(device(vec![0_u8; VOLUME_LEN])).into_inner();
    image.truncate(VOLUME_LEN / 2);
    let mut dev = device(image);
    assert_eq!(
        hadris_block::detect::sync::detect(&mut dev).unwrap(),
        Some(BlockFormat::Fat(FatVariant::Fat12))
    );

    let (error, dev) = failure(OpenVolume::open(dev));
    let Error::Fat(error) = error else {
        panic!("{error:?}");
    };
    assert_eq!(error.kind(), hadris_fs::ErrorKind::Corrupt);
    assert_eq!(dev.get_ref().len(), VOLUME_LEN / 2);
    let (error, dev) = failure(OpenVolume::open_detected(dev, FatVariant::Fat12));
    assert!(matches!(error, Error::Fat(_)));
    assert_eq!(
        format!("{}", failure(OpenVolume::open(dev)).0),
        "FAT open failed: corrupt filesystem data"
    );
}

#[test]
fn detects_exfat_but_rejects_unified_opening() {
    let mut image = vec![0_u8; 512];
    image[3..11].copy_from_slice(b"EXFAT   ");
    image[510..512].copy_from_slice(&[0x55, 0xaa]);

    let (error, dev) = failure(OpenVolume::open(device(image)));
    assert!(matches!(
        error,
        Error::UnsupportedFormat(BlockFormat::Fat(FatVariant::ExFat))
    ));
    assert_eq!(dev.get_ref().len(), 512);
}

#[test]
fn devices_smaller_than_a_block_are_unknown() {
    let dev = MemDevice::new(vec![0_u8; 512], BlockSize::new(4096).unwrap());
    let (error, _) = failure(OpenVolume::open(dev));
    assert!(matches!(error, Error::UnknownFormat));
}

#[test]
fn detects_gpt_on_4096_byte_blocks() {
    let block = BlockSize::new(4096).unwrap();
    let mut image = vec![0_u8; 4 * 4096];
    image[446 + 4] = 0xee;
    image[446 + 12..446 + 16].copy_from_slice(&3u32.to_le_bytes());
    image[510..512].copy_from_slice(&[0x55, 0xaa]);
    image[512..520].copy_from_slice(b"EFI PART");
    assert_eq!(
        hadris_block::detect::sync::detect(&mut MemDevice::new(&image[..], block)).unwrap(),
        None,
        "on a 4Kn disk the GPT header is in block 1, not at byte 512"
    );

    image[512..520].fill(0);
    image[4096..4104].copy_from_slice(b"EFI PART");
    let mut dev = MemDevice::new(image, block);
    assert_eq!(
        hadris_block::detect::sync::detect(&mut dev).unwrap(),
        Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
    );
    let (error, _) = failure(OpenVolume::open(dev));
    assert!(matches!(
        error,
        Error::PartitionedDisk(PartitionTableKind::Gpt)
    ));
}
