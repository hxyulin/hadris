use std::path::Path;
use std::path::PathBuf;

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::{FatType, FatVolume};
use tempfile::TempDir;

#[derive(Clone, Copy)]
pub struct FatCase {
    pub name: &'static str,
    pub size: u64,
    pub selection: FatTypeSelection,
    pub expected: FatType,
}

pub const FAT_CASES: [FatCase; 3] = [
    FatCase {
        name: "fat12",
        size: 2 * 1024 * 1024,
        selection: FatTypeSelection::Fat12,
        expected: FatType::Fat12,
    },
    FatCase {
        name: "fat16",
        size: 16 * 1024 * 1024,
        selection: FatTypeSelection::Fat16,
        expected: FatType::Fat16,
    },
    FatCase {
        name: "fat32",
        size: 64 * 1024 * 1024,
        selection: FatTypeSelection::Fat32,
        expected: FatType::Fat32,
    },
];

pub struct FatImage {
    _temp: TempDir,
    path: PathBuf,
}

impl FatImage {
    pub fn new(case: FatCase) -> Self {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(format!("{}.img", case.name));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.set_len(case.size).unwrap();
        let options = FatFormatOptions::new(case.size)
            .fat_type(case.selection)
            .volume_label("HADRIS");
        let volume = FatVolumeFormatter::format(file, options).unwrap();
        assert_eq!(volume.fat_type(), case.expected);
        drop(volume);
        Self { _temp: temp, path }
    }

    pub fn open(&self) -> FatVolume<std::fs::File> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .unwrap();
        FatVolume::open(file).unwrap()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
