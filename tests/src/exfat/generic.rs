//! The Hadris exFAT driver, `ExFatFs`, mounted through the generic
//! `FileSystem` adapter of the FAT suite.

use std::fs::{File, OpenOptions};
use std::path::Path;

use hadris_fat::exfat::sync::{ExFatFs, format as format_exfat};
use hadris_fat::exfat::{FormatOptions, VolumeLabel};
use hadris_fs::sync::{StdMutex, Volume};

use super::ExFatCase;
use crate::fat::LABEL;
use crate::fat::generic::{FsAdapter, Mount};

pub const NAME: &str = "Hadris";

/// `ExFatFs` on the image file, shared through a `Volume`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HadrisExFat;

impl Mount for HadrisExFat {
    type Fs = Volume<ExFatFs<File>, StdMutex>;

    fn mount(&self, image: &Path) -> Result<Self::Fs, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(image)
            .map_err(|error| error.to_string())?;
        let fs = ExFatFs::open(file).map_err(|error| error.to_string())?;
        Ok(Volume::new(fs))
    }

    fn label(&self, fs: &Self::Fs) -> Result<String, String> {
        let label = fs.lock().label().map_err(|error| error.to_string())?;
        Ok(label.map(|label| label.to_string()).unwrap_or_default())
    }
}

pub type HadrisExFatAdapter = FsAdapter<HadrisExFat>;

/// Formats `path` with the Hadris formatter for `case`.
pub fn format(path: &Path, case: ExFatCase) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    let label = VolumeLabel::new(LABEL).map_err(|error| error.to_string())?;
    let options = FormatOptions::new()
        .with_label(label)
        .with_cluster_size(case.cluster)
        .with_fat_count(case.fats)
        .with_volume_id(0x4841_4452);
    let fs = format_exfat(file, options).map_err(|error| error.to_string())?;
    if fs.cluster_size() != case.cluster {
        return Err(format!(
            "{} formatted with {}-byte clusters",
            case.name,
            fs.cluster_size()
        ));
    }
    Ok(())
}
