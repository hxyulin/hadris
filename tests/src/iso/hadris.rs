use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hadris_fs::sync::DriverExt;
use hadris_fs::tree::{Content, Tree};
use hadris_iso::sync::{IsoImage, IsoView, plan};
use hadris_iso::{Charset, IsoOptions, Namespace, VolumeIdentifiers};
use hadris_storage::{BlockSize, MemDevice};

use super::adapter::{IsoConsumer, IsoProducer};
use super::model::{IsoState, compare_state, strip_version};
use super::spec;
use crate::harness::join_path;
use crate::harness::tree::EntryData;

pub const NAME: &str = "Hadris";

/// An image held in memory.
pub type Image = IsoImage<MemDevice<Vec<u8>>>;

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

/// Opens an image held in memory. The bytes are padded to whole 512-byte
/// device blocks.
pub fn open(mut bytes: Vec<u8>) -> Result<Image, String> {
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    let dev = MemDevice::new(bytes, BlockSize::new(512).unwrap());
    IsoImage::open(dev).map_err(|error| error.to_string())
}

/// Writes `tree` into memory, sized by `plan`.
pub fn write_tree<C: hadris_fs::Clock>(
    tree: &Tree,
    options: &IsoOptions<C>,
) -> Result<Vec<u8>, String> {
    let size = plan(tree, options)
        .map_err(|error| error.to_string())?
        .size_bytes();
    let size = usize::try_from(size).map_err(|error| error.to_string())?;
    let mut dev = MemDevice::new(vec![0u8; size], BlockSize::new(2048).unwrap());
    hadris_iso::sync::write(&mut dev, tree, options).map_err(|error| error.to_string())?;
    Ok(dev.into_inner())
}

/// The tree of a conformance state.
pub fn tree(state: &IsoState) -> Result<Tree, String> {
    let mut tree = Tree::new();
    for (path, data) in &state.entries {
        let result = match data {
            EntryData::Directory => tree.add_dir(path),
            EntryData::File(contents) => tree.add_file(path, Content::bytes(contents.clone())),
        };
        result.map_err(|error| format!("{path}: {error}"))?;
    }
    Ok(tree)
}

/// Writes a strict Level 1 image in memory.
pub fn write(state: &IsoState) -> Result<Vec<u8>, String> {
    let options = IsoOptions::default()
        .with_volume(
            VolumeIdentifiers::new(state.volume_id.clone()).with_application("HADRIS CONFORMANCE"),
        )
        .with_charset(Charset::Strict);
    write_tree(&tree(state)?, &options)
}

pub fn snapshot(bytes: Vec<u8>) -> Result<IsoState, String> {
    let mut image = open(bytes)?;
    let pvd = image
        .primary_descriptor()
        .map_err(|error| error.to_string())?;
    let volume_id = String::from_utf8_lossy(pvd.volume_identifier.trimmed()).into_owned();
    let mut view = image
        .view(Namespace::Primary)
        .map_err(|error| error.to_string())?;
    let mut entries = BTreeMap::new();
    snapshot_dir(&mut view, "/", &mut entries)?;
    Ok(IsoState { volume_id, entries })
}

/// Checks `bytes` against the raw ECMA-119 oracle and then the Hadris reader.
pub fn verify_image(label: &str, bytes: Vec<u8>, expected: &IsoState) -> Result<(), String> {
    let oracle = spec::snapshot(&bytes)?;
    compare_state(&format!("{label} raw ECMA-119 oracle"), expected, &oracle)?;
    let hadris = snapshot(bytes)?;
    compare_state(&format!("Hadris reading {label}"), expected, &hadris)
}

/// Every entry of the directory `path` of `view`, keyed by path, with the
/// version suffix of each name removed.
pub fn snapshot_dir<D: hadris_storage::sync::BlockDevice>(
    view: &mut IsoView<D>,
    path: &str,
    entries: &mut BTreeMap<String, EntryData>,
) -> Result<(), String> {
    let mut children = Vec::new();
    for item in view.read_dir(path).map_err(|error| error.to_string())? {
        let item = item.map_err(|error| error.to_string())?;
        let name = String::from_utf8_lossy(item.name_bytes()).into_owned();
        children.push((name, item.file_type().is_dir()));
    }
    for (name, is_dir) in children {
        let child_path = join_path(path, strip_version(&name));
        let source = join_path(path, &name);
        if is_dir {
            entries.insert(child_path.clone(), EntryData::Directory);
            snapshot_dir(view, &source, entries)?;
        } else {
            let contents = view
                .read_to_vec(&source)
                .map_err(|error| error.to_string())?;
            entries.insert(child_path, EntryData::File(contents));
        }
    }
    Ok(())
}
