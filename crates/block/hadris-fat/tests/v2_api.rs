use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::sync::{FatVolume, FatVolumeBuilder};
use static_assertions::assert_impl_all;
use std::io::Cursor;

assert_impl_all!(FatVolume<Cursor<Vec<u8>>>: Send);
assert_impl_all!(FatVolumeBuilder<Cursor<Vec<u8>>>: Send);
assert_impl_all!(std::sync::Mutex<FatVolume<Cursor<Vec<u8>>>>: Send, Sync);

#[test]
fn canonical_v2_names_open_a_formatted_volume() {
    let mut image = vec![0_u8; 2 * 1024 * 1024];
    let options = FatFormatOptions::new(image.len() as u64)
        .volume_label("HADRIS")
        .fat_type(FatTypeSelection::Fat12)
        .volume_id(42);
    // Bind to `_` so the formatter (which mutably borrows `image`) is dropped
    // at the end of this statement, releasing the borrow before the read below.
    let _ = FatVolumeFormatter::format(std::io::Cursor::new(&mut image[..]), options).unwrap();

    let volume: FatVolume<_> = FatVolumeBuilder::new(std::io::Cursor::new(&image[..]))
        .open()
        .unwrap();
    assert_eq!(volume.fat_type(), hadris_fat::FatType::Fat12);
}

#[test]
fn file_entry_uses_len_vocabulary() {
    fn accepts_entry(entry: &hadris_fat::sync::FileEntry) {
        let _: u64 = entry.len();
        let _: bool = entry.is_empty();
    }

    let _ = accepts_entry;
}

#[test]
fn volume_can_be_moved_into_a_mutex_and_shared_between_threads() {
    let mut image = vec![0_u8; 2 * 1024 * 1024];
    let options = FatFormatOptions::new(image.len() as u64)
        .fat_type(FatTypeSelection::Fat12)
        .volume_id(42);
    let _ = FatVolumeFormatter::format(Cursor::new(&mut image[..]), options).unwrap();

    let volume = FatVolume::open(Cursor::new(image)).unwrap();
    let volume = std::sync::Arc::new(std::sync::Mutex::new(volume));
    let worker_volume = std::sync::Arc::clone(&volume);

    let fat_type = std::thread::spawn(move || worker_volume.lock().unwrap().fat_type())
        .join()
        .unwrap();

    assert_eq!(fat_type, hadris_fat::FatType::Fat12);
}
