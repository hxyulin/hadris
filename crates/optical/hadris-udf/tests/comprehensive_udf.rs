use std::io::Cursor;

use hadris_udf::descriptor::TagIdentifier;
use hadris_udf::dir::{FileCharacteristics, FileIdentifierDescriptor, decode_filename};
use hadris_udf::write::{SimpleDir, SimpleFile, UdfWriteOptions, UdfWriter};
use hadris_udf::{SECTOR_SIZE, UdfVolume};

fn image_with_file(name: &str, contents: &[u8]) -> Vec<u8> {
    let mut root = SimpleDir::root();
    root.add_file(SimpleFile::new(name, contents.to_vec()));
    UdfWriter::create(Cursor::new(Vec::new()), &root, UdfWriteOptions::default())
        .unwrap()
        .target
        .into_inner()
}

#[test]
fn invalid_and_truncated_images_are_rejected() {
    for image in [
        Vec::new(),
        vec![0_u8; 16 * SECTOR_SIZE],
        vec![0_u8; 257 * SECTOR_SIZE],
    ] {
        assert!(UdfVolume::open(Cursor::new(image)).is_err());
    }

    let mut image = image_with_file("file.txt", b"contents");
    image[16 * SECTOR_SIZE + 1..16 * SECTOR_SIZE + 6].copy_from_slice(b"WRONG");
    assert!(UdfVolume::open(Cursor::new(image)).is_err());
}

#[test]
fn tag_identifier_conversions_cover_volume_and_file_descriptors() {
    for (identifier, raw) in [
        (TagIdentifier::PrimaryVolumeDescriptor, 1),
        (TagIdentifier::AnchorVolumeDescriptorPointer, 2),
        (TagIdentifier::LogicalVolumeIntegrityDescriptor, 9),
        (TagIdentifier::FileSetDescriptor, 256),
        (TagIdentifier::FileIdentifierDescriptor, 257),
        (TagIdentifier::FileEntry, 261),
    ] {
        assert_eq!(identifier.to_u16(), raw);
        assert_eq!(TagIdentifier::from_u16(raw), identifier);
    }
    assert_eq!(TagIdentifier::from_u16(0xfffe), TagIdentifier::Unknown);
}

#[test]
fn writer_fids_are_aligned_parseable_and_roundtrip_the_filename() {
    let image = image_with_file("aligned-name.txt", b"payload");
    let volume = UdfVolume::open(Cursor::new(&image)).unwrap();
    let partition_start = volume.info().partition_start as usize;
    let root = volume.root_dir().unwrap();
    let entry = root.find("aligned-name.txt").unwrap();
    assert_eq!(volume.read_file(entry).unwrap(), b"payload");

    let fids = &image[(partition_start + 2) * SECTOR_SIZE..];
    let (parent, _) = FileIdentifierDescriptor::from_bytes(fids).unwrap();
    assert_eq!(parent.total_size(), 40);
    assert!(
        FileCharacteristics::from_bits_retain(parent.file_characteristics)
            .contains(FileCharacteristics::PARENT)
    );

    let child_offset = parent.total_size();
    let (child, variable) = FileIdentifierDescriptor::from_bytes(&fids[child_offset..]).unwrap();
    assert_eq!(child.total_size() % 4, 0);
    assert_eq!(
        decode_filename(&variable[..child.file_identifier_length as usize]),
        "aligned-name.txt"
    );
}

#[test]
fn writer_closes_lvid_with_partition_size_metadata() {
    let image = image_with_file("file.txt", b"contents");
    let volume = UdfVolume::open(Cursor::new(&image)).unwrap();
    let partition_length = volume.info().partition_length;
    let lvid = &image[289 * SECTOR_SIZE..290 * SECTOR_SIZE];

    assert_eq!(
        u16::from_le_bytes(lvid[0..2].try_into().unwrap()),
        TagIdentifier::LogicalVolumeIntegrityDescriptor.to_u16()
    );
    assert_eq!(u32::from_le_bytes(lvid[28..32].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(lvid[72..76].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(lvid[80..84].try_into().unwrap()), 0);
    assert_eq!(
        u32::from_le_bytes(lvid[84..88].try_into().unwrap()),
        partition_length
    );
}
