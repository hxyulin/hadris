use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fatfs::{
    DefaultTimeProvider, Dir, FileSystem, FormatVolumeOptions, FsOptions, LossyOemCpConverter,
    StdIoWrapper,
};

use super::adapter::FatAdapter;
use super::model::{EntryState, FsState, Operation};
use super::{FatCase, MUTABLE_ATTRS};
use crate::harness::join_path;
use crate::harness::tree::EntryData;

pub const NAME: &str = "rust-fatfs";

/// The independent `fatfs` crate. It cannot set attributes, so
/// [`Operation::SetAttrs`] is accepted and ignored.
pub struct FatfsAdapter {
    image: PathBuf,
}

impl FatfsAdapter {
    pub fn new(image: PathBuf) -> Self {
        Self { image }
    }

    fn open(&self) -> Result<FileSystem<StdIoWrapper<File>>, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.image)
            .map_err(|error| error.to_string())?;
        FileSystem::new(file, FsOptions::new()).map_err(|error| error.to_string())
    }
}

impl FatAdapter for FatfsAdapter {
    fn apply(&mut self, operation: &Operation) -> Result<(), String> {
        let fs = self.open()?;
        let root = fs.root_dir();
        match operation {
            Operation::CreateDir { path } => {
                root.create_dir(relative(path))
                    .map_err(|error| error.to_string())?;
            }
            Operation::CreateFile { path, data } => {
                let mut file = root
                    .create_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.write_all(data).map_err(|error| error.to_string())?;
            }
            Operation::ReplaceFile { path, data } => {
                let mut file = root
                    .open_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.seek(SeekFrom::Start(0))
                    .map_err(|error| error.to_string())?;
                file.truncate().map_err(|error| error.to_string())?;
                file.write_all(data).map_err(|error| error.to_string())?;
            }
            Operation::AppendFile { path, data } => {
                let mut file = root
                    .open_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.seek(SeekFrom::End(0))
                    .map_err(|error| error.to_string())?;
                file.write_all(data).map_err(|error| error.to_string())?;
            }
            Operation::TruncateFile { path, len } => {
                let mut file = root
                    .open_file(relative(path))
                    .map_err(|error| error.to_string())?;
                file.seek(SeekFrom::Start(*len as u64))
                    .map_err(|error| error.to_string())?;
                file.truncate().map_err(|error| error.to_string())?;
            }
            Operation::Rename { from, to } => root
                .rename(relative(from), &root, relative(to))
                .map_err(|error| error.to_string())?,
            Operation::SetAttrs { .. } => {}
            Operation::Delete { path } => root
                .remove(relative(path))
                .map_err(|error| error.to_string())?,
        }
        drop(root);
        fs.unmount().map_err(|error| error.to_string())
    }

    fn snapshot(&mut self) -> Result<FsState, String> {
        snapshot(&self.image)
    }
}

pub fn snapshot(path: &Path) -> Result<FsState, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let fs = FileSystem::new(file, FsOptions::new()).map_err(|error| error.to_string())?;
    let mut state = FsState {
        label: fs.volume_label().trim().to_string(),
        entries: BTreeMap::new(),
    };
    snapshot_dir(&fs.root_dir(), "/", &mut state.entries)?;
    Ok(state)
}

fn snapshot_dir(
    dir: &Dir<'_, StdIoWrapper<File>, DefaultTimeProvider, LossyOemCpConverter>,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> Result<(), String> {
    let children = dir
        .iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for entry in children {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let child_path = join_path(path, &name);
        let attrs = entry.attributes().bits() & MUTABLE_ATTRS;
        let data = if entry.is_dir() {
            EntryData::Directory
        } else {
            let mut contents = Vec::with_capacity(entry.len() as usize);
            entry
                .to_file()
                .read_to_end(&mut contents)
                .map_err(|error| error.to_string())?;
            EntryData::File(contents)
        };
        entries.insert(child_path.clone(), EntryState { data, attrs });
        if entry.is_dir() {
            snapshot_dir(&entry.to_dir(), &child_path, entries)?;
        }
    }
    Ok(())
}

pub fn format(path: &Path, case: FatCase) -> Result<(), String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    let fat_type = match case.bits {
        12 => fatfs::FatType::Fat12,
        16 => fatfs::FatType::Fat16,
        32 => fatfs::FatType::Fat32,
        other => return Err(format!("unsupported FAT width {other}")),
    };
    let mut storage = StdIoWrapper::new(file);
    fatfs::format_volume(
        &mut storage,
        FormatVolumeOptions::new()
            .fat_type(fat_type)
            .volume_label(*b"HADRISCONF "),
    )
    .map_err(|error| error.to_string())
}

fn relative(path: &str) -> &str {
    path.trim_start_matches('/')
}
