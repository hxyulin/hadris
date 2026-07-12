use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::sync::{FatVolume, FatVolumeBuilder};

#[test]
fn canonical_v2_names_open_a_formatted_volume() {
    let mut image = vec![0_u8; 2 * 1024 * 1024];
    let options = FatFormatOptions::new(image.len() as u64)
        .volume_label("HADRIS")
        .fat_type(FatTypeSelection::Fat12)
        .volume_id(42);
    let formatted =
        FatVolumeFormatter::format(std::io::Cursor::new(&mut image[..]), options).unwrap();
    drop(formatted);

    let volume: FatVolume<_> = FatVolumeBuilder::new(std::io::Cursor::new(&image[..]))
        .open()
        .unwrap();
    assert_eq!(volume.fat_type(), hadris_fat::FatType::Fat12);
}

#[test]
fn file_entry_uses_len_vocabulary() {
    fn accepts_entry<DATA>(entry: &hadris_fat::sync::FileEntry)
    where
        DATA: hadris_io::sync::Read + hadris_io::sync::Seek,
    {
        let _: u64 = entry.len();
        let _: bool = entry.is_empty();
    }

    let _ = accepts_entry::<std::io::Cursor<Vec<u8>>>;
}
