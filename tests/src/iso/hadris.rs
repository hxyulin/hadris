use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use hadris_iso::directory::DirectoryRef;
use hadris_iso::read::{IsoImage, PathSeparator};
use hadris_iso::write::options::{CreationFeatures, IsoFormatOptions};
use hadris_iso::write::{File as IsoFile, InputFiles, IsoImageWriter};

use super::adapter::{IsoConsumer, IsoProducer};
use super::model::{IsoState, compare_state, strip_version};
use super::{SECTOR_SIZE, spec};
use crate::harness::join_path;
use crate::harness::tree::EntryData;

pub const NAME: &str = "Hadris";

/// The Hadris ISO implementation as a peer of the external tools.
pub struct HadrisIso;

impl IsoProducer for HadrisIso {
    fn name(&self) -> String {
        NAME.to_string()
    }

    fn produce(&self, state: &IsoState, _workspace: &Path, image: &Path) -> Result<(), String> {
        fs::write(image, write(state)?).map_err(|error| error.to_string())
    }
}

impl IsoConsumer for HadrisIso {
    fn name(&self) -> String {
        NAME.to_string()
    }

    fn snapshot(&self, image: &Path, _workspace: &Path) -> Result<IsoState, String> {
        snapshot(fs::read(image).map_err(|error| error.to_string())?)
    }
}

/// Writes a strict Level 1 image in memory.
pub fn write(state: &IsoState) -> Result<Vec<u8>, String> {
    let options = IsoFormatOptions {
        volume_name: state.volume_id.clone(),
        system_id: None,
        volume_set_id: None,
        publisher_id: None,
        preparer_id: None,
        application_id: Some("HADRIS CONFORMANCE".to_string()),
        sector_size: SECTOR_SIZE,
        path_separator: PathSeparator::ForwardSlash,
        features: CreationFeatures::default(),
        strict_charset: true,
    };
    IsoImageWriter::create(Cursor::new(Vec::new()), input_files(state), options)
        .map(|cursor| cursor.into_inner())
        .map_err(|error| error.to_string())
}

pub fn snapshot(bytes: Vec<u8>) -> Result<IsoState, String> {
    let image = IsoImage::open(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let volume_id = image
        .read_pvd()
        .map_err(|error| error.to_string())?
        .volume_identifier
        .to_str()
        .trim_end()
        .to_string();
    let mut entries = BTreeMap::new();
    snapshot_dir(&image, image.root_dir().dir_ref(), "/", &mut entries)?;
    Ok(IsoState { volume_id, entries })
}

/// Checks `bytes` against the raw ECMA-119 oracle and then the Hadris reader.
pub fn verify_image(label: &str, bytes: Vec<u8>, expected: &IsoState) -> Result<(), String> {
    let oracle = spec::snapshot(&bytes)?;
    compare_state(&format!("{label} raw ECMA-119 oracle"), expected, &oracle)?;
    let hadris = snapshot(bytes)?;
    compare_state(&format!("Hadris reading {label}"), expected, &hadris)
}

fn input_files(state: &IsoState) -> InputFiles {
    fn children(state: &IsoState, parent: &str) -> Vec<IsoFile> {
        state
            .entries
            .iter()
            .filter_map(|(path, data)| {
                let relative = path.strip_prefix(parent)?;
                if relative.is_empty() || relative.contains('/') {
                    return None;
                }
                let name = Arc::new(relative.to_string());
                Some(match data {
                    EntryData::Directory => IsoFile::Directory {
                        name,
                        children: children(state, &format!("{path}/")),
                    },
                    EntryData::File(contents) => IsoFile::File {
                        name,
                        contents: contents.clone(),
                    },
                })
            })
            .collect()
    }

    InputFiles {
        path_separator: PathSeparator::ForwardSlash,
        files: children(state, "/"),
    }
}

fn snapshot_dir(
    image: &IsoImage<Cursor<Vec<u8>>>,
    directory: DirectoryRef,
    path: &str,
    entries: &mut BTreeMap<String, EntryData>,
) -> Result<(), String> {
    for entry in image.open_dir(directory).entries() {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry.is_special() {
            continue;
        }
        let display_name = entry.display_name();
        let name = strip_version(&display_name);
        let child_path = join_path(path, name);
        if entry.is_directory() {
            let child = entry.as_dir_ref(image).map_err(|error| error.to_string())?;
            entries.insert(child_path.clone(), EntryData::Directory);
            snapshot_dir(image, child, &child_path, entries)?;
        } else {
            let contents = image.read_file(&entry).map_err(|error| error.to_string())?;
            entries.insert(child_path, EntryData::File(contents));
        }
    }
    Ok(())
}
