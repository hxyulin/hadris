use hadris_block::detect::{BlockFormat, FatVariant, PartitionTableKind};
use hadris_block::partition::{gpt_partition_view, mbr_partition_view};
use hadris_block::sync::OpenVolume;
use hadris_block::{Error, part};
use hadris_fat::format::{FatTypeSelection, FatVolumeFormatter, FormatOptions};

const VOLUME_LEN: usize = 2 * 1024 * 1024;

fn format_fat12(
    source: impl hadris_io::sync::Read + hadris_io::sync::Write + hadris_io::sync::Seek,
) {
    let options = FormatOptions::new(VOLUME_LEN as u64).with_fat_type(FatTypeSelection::Fat12);
    let volume = FatVolumeFormatter::format(source, options).unwrap();
    drop(volume);
}

#[test]
fn opens_detected_fat_and_returns_source_borrow() {
    let mut image = std::io::Cursor::new(vec![0_u8; VOLUME_LEN]);
    format_fat12(hadris_io::sync::Borrowed::new(&mut image));

    let volume = OpenVolume::open(&mut image, 512).unwrap();
    assert_eq!(volume.format(), FatVariant::Fat12);
    assert!(volume.as_fat().is_some());
    let source = volume.into_inner();
    assert_eq!(source.get_ref().len(), VOLUME_LEN);
}

#[test]
fn opens_fat_inside_mbr_partition_view() {
    let start_lba = 1_u32;
    let sector_count = (VOLUME_LEN / 512) as u32;
    let entry = part::MbrPartition::new(part::MbrPartitionType::Fat12, start_lba, sector_count);
    let mut table = part::MbrPartitionTable::new();
    table.partitions[0] = entry;
    let mbr = part::MasterBootRecord::new(table);

    let mut image = std::io::Cursor::new(vec![0_u8; VOLUME_LEN + 512]);
    std::io::Write::write_all(&mut image, bytemuck::bytes_of(&mbr)).unwrap();
    {
        let view = mbr_partition_view(&mut image, &entry, 512).unwrap();
        format_fat12(view);
    }

    assert_eq!(
        hadris_block::detect::sync::detect(&mut image, 512).unwrap(),
        Some(BlockFormat::PartitionTable(PartitionTableKind::Mbr))
    );
    assert!(matches!(
        OpenVolume::open(&mut image, 512),
        Err(Error::PartitionedDisk(PartitionTableKind::Mbr))
    ));

    let mut view = mbr_partition_view(&mut image, &entry, 512).unwrap();
    let volume = OpenVolume::open(&mut view, 512).unwrap();
    assert_eq!(volume.format(), FatVariant::Fat12);
}

#[test]
fn opens_fat_inside_gpt_partition_view() {
    let start_lba = 2048_u64;
    let sector_count = (VOLUME_LEN / 512) as u64;
    let entry = part::GptPartitionEntry::new(
        part::Guid::EFI_SYSTEM,
        part::Guid::UNUSED,
        start_lba,
        start_lba + sector_count - 1,
    );
    let mut image = std::io::Cursor::new(vec![0_u8; (start_lba as usize * 512) + VOLUME_LEN]);
    {
        let view = gpt_partition_view(&mut image, &entry, 512).unwrap();
        format_fat12(view);
    }

    let mut view = gpt_partition_view(&mut image, &entry, 512).unwrap();
    let volume = OpenVolume::open(&mut view, 512).unwrap();
    assert_eq!(volume.format(), FatVariant::Fat12);
}

#[test]
fn rejects_unknown_and_mismatched_formats_without_consuming_source() {
    let mut unknown = std::io::Cursor::new(vec![0_u8; 1024]);
    assert!(matches!(
        OpenVolume::open(&mut unknown, 512),
        Err(Error::UnknownFormat)
    ));
    unknown.set_position(7);

    let mut image = std::io::Cursor::new(vec![0_u8; VOLUME_LEN]);
    format_fat12(hadris_io::sync::Borrowed::new(&mut image));
    assert!(matches!(
        OpenVolume::open_detected(&mut image, FatVariant::Fat16),
        Err(Error::DetectedFormatMismatch { .. })
    ));
    image.set_position(9);
}
