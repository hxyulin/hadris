use hadris_block::Detail;
use hadris_block::detect::{BlockFormat, FatVariant, PartitionTableKind};
use hadris_block::part::sync::open;
use hadris_block::part::{self, MbrEntry, MbrType};
use hadris_block::sync::OpenVolume;
use hadris_fat::{FatKind, FormatOptions};
use hadris_fs::ErrorKind;
use hadris_fs::sync::DriverExt;
use hadris_fs::{Error, MountError};
use hadris_storage::sync::BlockDevice;
use hadris_storage::{BlockSize, MemDevice, Partition};

const VOLUME_LEN: usize = 2 * 1024 * 1024;
const BLOCK: BlockSize = match BlockSize::new(512) {
    Some(size) => size,
    None => panic!(),
};

fn device(bytes: Vec<u8>) -> MemDevice<Vec<u8>> {
    MemDevice::new(bytes, BLOCK)
}

/// The reason an open failed, and the device it gave back.
fn failure<T, D, E>(result: Result<T, MountError<D, E>>) -> (Error<E>, D) {
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

#[path = "../../hadris-ntfs/tests/support/image.rs"]
mod ntfs_image;

const FAT12: BlockFormat = BlockFormat::Fat(FatVariant::Fat12);

#[test]
fn opens_detected_fat_and_returns_the_device() {
    let mut dev = format_fat12(device(vec![0_u8; VOLUME_LEN]));

    let mut volume = OpenVolume::open(&mut dev).unwrap();
    assert_eq!(volume.format(), FAT12);
    assert!(volume.as_fat().is_some());
    volume.write_file("/a.txt", b"fat").unwrap();
    assert_eq!(volume.read_to_vec("/a.txt").unwrap(), b"fat");
    volume.sync().unwrap();
    let dev = volume.into_inner();
    assert_eq!(dev.get_ref().len(), VOLUME_LEN);
}

#[test]
fn opens_fat_inside_mbr_partition() {
    let sector_count = (VOLUME_LEN / 512) as u64;
    let mut mbr = part::Mbr::new(sector_count + 1, BLOCK).unwrap();
    mbr.add(MbrEntry::new(MbrType::FAT12, 1, sector_count))
        .unwrap();
    let table = part::Disk::new(mbr);
    let entry = table.partition(0).unwrap();

    let mut disk = device(vec![0_u8; VOLUME_LEN + 512]);
    part::sync::write(&mut disk, &table).unwrap();
    format_fat12(open(&mut disk, &entry).unwrap());

    assert_eq!(
        hadris_block::detect::sync::detect(&mut disk).unwrap(),
        Some(BlockFormat::PartitionTable(PartitionTableKind::Mbr))
    );
    let (error, _) = failure(OpenVolume::open(&mut disk));
    assert_eq!(
        error.detail().and_then(Detail::from_code),
        Some(Detail::PartitionedDisk)
    );
    assert_eq!(error.kind(), ErrorKind::InvalidInput);

    let entry = part::sync::read(&mut disk).unwrap().partition(0).unwrap();
    let volume = OpenVolume::open(open(&mut disk, &entry).unwrap()).unwrap();
    assert_eq!(volume.format(), FAT12);
}

#[test]
fn opens_fat_inside_gpt_partition() {
    let start_lba = 2048_u64;
    let sector_count = (VOLUME_LEN / 512) as u64;
    let layout = part::DiskLayout::gpt(part::Guid::from_bytes([0x61; 16])).partition(
        part::PartitionSpec::new(
            part::gpt::types::EFI_SYSTEM,
            part::Size::Blocks(sector_count),
        )
        .with_start(start_lba),
    );
    let mut disk = device(vec![
        0_u8;
        (start_lba as usize * 512) + VOLUME_LEN + 34 * 512
    ]);
    let table = part::sync::create(&mut disk, &layout).unwrap();
    let entry = table.partition(0).unwrap();
    format_fat12(open(&mut disk, &entry).unwrap());

    assert_eq!(
        hadris_block::detect::sync::detect(&mut disk).unwrap(),
        Some(BlockFormat::PartitionTable(PartitionTableKind::Gpt))
    );
    let volume = OpenVolume::open(open(&mut disk, &entry).unwrap()).unwrap();
    assert_eq!(volume.format(), FAT12);
    let partition: Partition<_> = volume.into_inner();
    assert_eq!(partition.offset(), start_lba * 512);
}

#[test]
fn partitions_past_the_disk_are_refused() {
    let mut mbr = part::Mbr::new(100, BLOCK).unwrap();
    mbr.add(MbrEntry::new(MbrType::FAT12, 8, 16)).unwrap();
    let entry = part::Disk::new(mbr).partition(0).unwrap();
    let mut disk = device(vec![0_u8; 16 * 512]);
    assert!(open(&mut disk, &entry).is_err());
}

#[test]
fn rejects_unknown_and_mismatched_formats() {
    let (error, dev) = failure(OpenVolume::open(device(vec![0_u8; 1024])));
    assert_eq!(error.kind(), ErrorKind::NotRecognized);
    assert_eq!(dev.get_ref().len(), 1024);

    let dev = format_fat12(device(vec![0_u8; VOLUME_LEN]));
    let (error, dev) = failure(OpenVolume::open_detected(
        dev,
        BlockFormat::Fat(FatVariant::Fat16),
    ));
    assert_eq!(error.kind(), ErrorKind::Corrupt);
    assert!(matches!(
        error.detail().and_then(Detail::from_code),
        Some(Detail::FormatMismatch)
    ));
    assert_eq!(dev.get_ref().len(), VOLUME_LEN);
    assert_eq!(
        OpenVolume::open(dev).unwrap().format(),
        FAT12,
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

    let err = OpenVolume::open(dev).map(|_| ()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Corrupt);
    assert_eq!(err.device().get_ref().len(), VOLUME_LEN / 2);
    let io: std::io::Error = OpenVolume::open(err.into_device())
        .map(|_| ())
        .unwrap_err()
        .into();
    assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);

    let mut image = format_fat12(device(vec![0_u8; VOLUME_LEN])).into_inner();
    image.truncate(VOLUME_LEN / 2);
    let (error, dev) = failure(OpenVolume::open(device(image)));
    assert_eq!(error.device_error(), None);
    assert_eq!(error.detail().and_then(Detail::from_code), None);
    assert_eq!(error.kind(), ErrorKind::Corrupt);
    assert_eq!(dev.get_ref().len(), VOLUME_LEN / 2);
    let (error, dev) = failure(OpenVolume::open_detected(dev, FAT12));
    assert_eq!(error.detail().and_then(Detail::from_code), None);
    assert_eq!(
        error.detail().and_then(hadris_fat::Detail::from_code),
        Some(hadris_fat::Detail::BootSector)
    );
    assert_eq!(
        format!("{}", failure(OpenVolume::open(dev)).0),
        "volume is larger than the device"
    );
}

const EXFAT: BlockFormat = BlockFormat::Fat(FatVariant::ExFat);

#[test]
fn opens_detected_exfat() {
    let dev = device(vec![0_u8; VOLUME_LEN]);
    let options = hadris_fat::exfat::FormatOptions::new();
    let dev = hadris_fat::exfat::sync::format(dev, options)
        .unwrap()
        .into_inner();

    let mut volume = OpenVolume::open(dev).unwrap();
    assert_eq!(volume.format(), EXFAT);
    assert!(volume.as_fat().is_none());
    assert!(volume.as_exfat_mut().is_some());
    volume.write_file("/Données.txt", b"exfat").unwrap();
    assert_eq!(volume.read_to_vec("/DONNÉES.TXT").unwrap(), b"exfat");
    volume.sync().unwrap();
    let mut dev = volume.into_exfat().ok().unwrap().into_inner();
    assert!(
        hadris_fat::exfat::sync::check(&mut dev, &mut [0u8; 4096], |_| {})
            .unwrap()
            .is_clean()
    );
}

#[test]
fn a_bad_exfat_volume_gives_the_device_back() {
    let mut image = vec![0_u8; 512];
    image[3..11].copy_from_slice(b"EXFAT   ");
    image[510..512].copy_from_slice(&[0x55, 0xaa]);

    let (error, dev) = failure(OpenVolume::open(device(image)));
    assert_eq!(error.detail().and_then(Detail::from_code), None);
    assert_eq!(error.kind(), ErrorKind::Corrupt);
    assert_eq!(dev.get_ref().len(), 512);
}

#[test]
fn devices_smaller_than_a_block_are_unknown() {
    let dev = MemDevice::new(vec![0_u8; 512], BlockSize::new(4096).unwrap());
    let (error, _) = failure(OpenVolume::open(dev));
    assert_eq!(error.kind(), ErrorKind::NotRecognized);
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
    assert_eq!(
        error.detail().and_then(Detail::from_code),
        Some(Detail::PartitionedDisk)
    );
}

#[test]
fn opens_ntfs_read_only() {
    let mut dev = device(ntfs_image::base_image());
    assert_eq!(
        hadris_block::detect::sync::detect(&mut dev).unwrap(),
        Some(BlockFormat::Ntfs)
    );
    let mut volume = OpenVolume::open(dev).unwrap();
    assert_eq!(volume.format(), BlockFormat::Ntfs);
    assert!(volume.as_fat().is_none());
    assert!(!volume.capabilities().is_writable());
    assert_eq!(volume.read_to_vec("/HELLO.TXT").unwrap(), b"hello ntfs");
    assert_eq!(
        volume.write_file("/new.txt", b"x").unwrap_err().kind(),
        ErrorKind::ReadOnly
    );
    hadris_fs::sync::contract::check_read_only(&mut volume).unwrap();
    let volume = volume.into_fat().map(|_| ()).unwrap_err();
    assert_eq!(
        volume.into_inner().into_inner().len(),
        ntfs_image::IMAGE_LEN
    );
}

#[test]
fn fat_passes_the_contract_through_the_opener() {
    let dev = format_fat12(device(vec![0_u8; VOLUME_LEN]));
    let mut volume = OpenVolume::open(dev).unwrap();
    hadris_fs::sync::contract::check(&mut volume).unwrap();
}

#[test]
fn a_corrupt_ntfs_volume_gives_the_device_back() {
    let mut image = ntfs_image::base_image();
    image[11..13].copy_from_slice(&1000u16.to_le_bytes());
    let (error, dev) = failure(OpenVolume::open(device(image)));
    assert_eq!(error.detail().and_then(Detail::from_code), None);
    assert_eq!(error.kind(), ErrorKind::Corrupt);
    assert_eq!(dev.into_inner().len(), ntfs_image::IMAGE_LEN);
}
