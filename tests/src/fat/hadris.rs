use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use hadris_fat::format::{FatFormatOptions, FatTypeSelection, FatVolumeFormatter};
use hadris_fat::raw::DirEntryAttrFlags;
use hadris_fat::write::FileWriter;
use hadris_fat::{FatDir, FatType, FatVolume, FatVolumeReadExt, FatVolumeWriteExt};

use super::adapter::FatAdapter;
use super::model::{EntryState, FsState, Operation};
use super::{FatCase, LABEL, MUTABLE_ATTRS};
use crate::harness::tree::EntryData;
use crate::harness::{join_path, split_parent};

pub const NAME: &str = "Hadris";

/// The Hadris FAT implementation as a peer of the external tools.
pub struct HadrisFatAdapter {
    image: PathBuf,
}

impl HadrisFatAdapter {
    pub fn new(image: PathBuf) -> Self {
        Self { image }
    }

    fn open(&self) -> Result<FatVolume<File>, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.image)
            .map_err(|error| error.to_string())?;
        FatVolume::open(file).map_err(|error| error.to_string())
    }

    fn put_file(&self, path: &str, contents: &[u8], replace: bool) -> Result<(), String> {
        let volume = self.open()?;
        let (parent_path, name) = split_parent(path)?;
        let parent = open_dir_path(&volume, parent_path)?;
        let entry = if replace {
            let old = parent
                .find(name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("missing file: {path}"))?;
            volume
                .truncate(&old, 0)
                .map_err(|error| error.to_string())?;
            parent
                .find(name)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("missing truncated file: {path}"))?
        } else {
            volume
                .create_file(&parent, name)
                .map_err(|error| error.to_string())?
        };
        let mut writer = volume
            .write_file(&entry)
            .map_err(|error| error.to_string())?;
        write_all(&mut writer, contents)?;
        writer.finish().map_err(|error| error.to_string())?;
        volume.sync().map_err(|error| error.to_string())
    }
}

impl FatAdapter for HadrisFatAdapter {
    fn apply(&mut self, operation: &Operation) -> Result<(), String> {
        match operation {
            Operation::CreateDir { path } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                volume
                    .create_dir(&parent, name)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::CreateFile { path, data } => self.put_file(path, data, false),
            Operation::ReplaceFile { path, data } => self.put_file(path, data, true),
            Operation::AppendFile { path, data } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing file: {path}"))?;
                let mut writer =
                    FileWriter::new_append(&volume, &entry).map_err(|error| error.to_string())?;
                write_all(&mut writer, data)?;
                writer.finish().map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::TruncateFile { path, len } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing file: {path}"))?;
                volume
                    .truncate(&entry, *len)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::Rename { from, to } => {
                let volume = self.open()?;
                let (source_parent, source_name) = split_parent(from)?;
                let (dest_parent, dest_name) = split_parent(to)?;
                let source_dir = open_dir_path(&volume, source_parent)?;
                let dest_dir = open_dir_path(&volume, dest_parent)?;
                let entry = source_dir
                    .find(source_name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing rename source: {from}"))?;
                volume
                    .rename(&entry, &dest_dir, dest_name)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::SetAttrs { path, attrs } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing attribute target: {path}"))?;
                let immutable = entry.attributes().bits() & !MUTABLE_ATTRS;
                let attributes = DirEntryAttrFlags::from_bits_retain(immutable | attrs);
                volume
                    .set_attributes(&entry, attributes)
                    .map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
            Operation::Delete { path } => {
                let volume = self.open()?;
                let (parent_path, name) = split_parent(path)?;
                let parent = open_dir_path(&volume, parent_path)?;
                let entry = parent
                    .find(name)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing delete target: {path}"))?;
                volume.delete(&entry).map_err(|error| error.to_string())?;
                volume.sync().map_err(|error| error.to_string())
            }
        }
    }

    fn snapshot(&mut self) -> Result<FsState, String> {
        let volume = self.open()?;
        let mut state = FsState {
            label: volume.volume_info().volume_label().trim().to_string(),
            entries: BTreeMap::new(),
        };
        snapshot_dir(&volume, volume.root_dir(), "/", &mut state.entries)?;
        Ok(state)
    }
}

/// Formats `path` with the Hadris formatter and verifies the selected width.
pub fn format(path: &Path, case: FatCase) -> Result<(), String> {
    let (selection, expected) = match case.bits {
        12 => (FatTypeSelection::Fat12, FatType::Fat12),
        16 => (FatTypeSelection::Fat16, FatType::Fat16),
        32 => (FatTypeSelection::Fat32, FatType::Fat32),
        other => return Err(format!("unsupported FAT width {other}")),
    };
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.set_len(case.size).map_err(|error| error.to_string())?;
    let options = FatFormatOptions::new(case.size)
        .fat_type(selection)
        .volume_label(LABEL)
        .volume_id(0x4841_4452);
    let volume = FatVolumeFormatter::format(file, options).map_err(|error| error.to_string())?;
    if volume.fat_type() != expected {
        return Err(format!(
            "{} formatted as {:?}, expected {:?}",
            case.name,
            volume.fat_type(),
            expected
        ));
    }
    drop(volume);
    Ok(())
}

fn open_dir_path<'a>(volume: &'a FatVolume<File>, path: &str) -> Result<FatDir<'a, File>, String> {
    let mut dir = volume.root_dir();
    for component in path.split('/').filter(|part| !part.is_empty()) {
        dir = dir.open_dir(component).map_err(|error| error.to_string())?;
    }
    Ok(dir)
}

fn write_all(writer: &mut FileWriter<'_, File>, mut contents: &[u8]) -> Result<(), String> {
    while !contents.is_empty() {
        let written = writer.write(contents).map_err(|error| error.to_string())?;
        if written == 0 {
            return Err("file writer made no progress".to_string());
        }
        contents = &contents[written..];
    }
    Ok(())
}

fn snapshot_dir(
    volume: &FatVolume<File>,
    dir: FatDir<'_, File>,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> Result<(), String> {
    let children = dir
        .entries()
        .map(|entry| {
            entry
                .map_err(|error| error.to_string())?
                .as_entry()
                .cloned()
                .ok_or_else(|| "unexpected directory entry variant".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    for entry in children {
        let name = entry.name().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let child_path = join_path(path, &name);
        let attrs = entry.attributes().bits() & MUTABLE_ATTRS;
        let data = if entry.is_directory() {
            EntryData::Directory
        } else {
            let mut reader = volume
                .read_file(&entry)
                .map_err(|error| error.to_string())?;
            let mut contents = Vec::with_capacity(entry.len() as usize);
            let mut buffer = [0_u8; 4096];
            loop {
                let count = reader
                    .read(&mut buffer)
                    .map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                contents.extend_from_slice(&buffer[..count]);
            }
            EntryData::File(contents)
        };
        entries.insert(child_path.clone(), EntryState { data, attrs });
        if entry.is_directory() {
            let child = dir.open_entry(&entry).map_err(|error| error.to_string())?;
            snapshot_dir(volume, child, &child_path, entries)?;
        }
    }
    Ok(())
}
