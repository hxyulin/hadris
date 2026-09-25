use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hadris_fs::sync::FileSystem;
use hadris_fs::{Content, Node, Tree};
use hadris_fs::{MountOptions, NodeId};
use hadris_iso::plan;
use hadris_iso::sync::IsoFs;
use hadris_iso::{Charset, IsoOptions, Namespace, VolumeIdentifiers};
use hadris_storage::{BlockSize, MemDevice};

use super::adapter::{IsoConsumer, IsoProducer};
use super::model::{IsoState, compare_state, strip_version};
use super::spec;
use crate::harness::files::{entries, read_node};
use crate::harness::join_path;
use crate::harness::tree::EntryData;

pub const NAME: &str = "Hadris";

/// An image held in memory.
pub type Image = IsoFs<MemDevice<Vec<u8>>>;

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

/// Mounts an image held in memory with its most capable tree. The bytes
/// are padded to whole 512-byte device blocks.
pub fn open(bytes: Vec<u8>) -> Result<Image, String> {
    open_namespace(bytes, Namespace::Preferred)
}

/// Mounts an image held in memory with the tree `namespace` names.
pub fn open_namespace(mut bytes: Vec<u8>, namespace: Namespace) -> Result<Image, String> {
    bytes.resize(bytes.len().next_multiple_of(512), 0);
    let dev = MemDevice::new(bytes, BlockSize::new(512).unwrap());
    IsoFs::mount_namespace(dev, MountOptions::new(), namespace).map_err(|error| error.to_string())
}

/// Writes `tree` into memory, sized by `plan`.
pub fn write_tree(tree: &Tree, options: &IsoOptions) -> Result<Vec<u8>, String> {
    let size = plan(tree, options)
        .map_err(|error| error.to_string())?
        .size();
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
            EntryData::Directory => tree.insert(path, Node::dir()),
            EntryData::File(contents) => {
                tree.insert(path, Node::file(Content::bytes(contents.clone())))
            }
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
    let mut view = open_namespace(bytes, Namespace::Primary)?;
    let pvd = view
        .primary_descriptor()
        .map_err(|error| error.to_string())?;
    let volume_id = String::from_utf8_lossy(pvd.volume_identifier.trimmed()).into_owned();
    let mut entries = BTreeMap::new();
    let root = view.root();
    snapshot_dir(&mut view, root, "/", &mut entries)?;
    Ok(IsoState { volume_id, entries })
}

/// Checks `bytes` against the raw ECMA-119 oracle and then the Hadris reader.
pub fn verify_image(label: &str, bytes: Vec<u8>, expected: &IsoState) -> Result<(), String> {
    let oracle = spec::snapshot(&bytes)?;
    compare_state(&format!("{label} raw ECMA-119 oracle"), expected, &oracle)?;
    let hadris = snapshot(bytes)?;
    compare_state(&format!("Hadris reading {label}"), expected, &hadris)
}

/// Every entry of the directory `dir` at `path` of `view`, keyed by path,
/// with the version suffix of each name removed.
pub fn snapshot_dir<D: hadris_storage::sync::BlockDevice>(
    view: &mut IsoFs<D>,
    dir: NodeId,
    path: &str,
    out: &mut BTreeMap<String, EntryData>,
) -> Result<(), String> {
    for entry in entries(view, dir).map_err(|error| error.to_string())? {
        let name = String::from_utf8_lossy(entry.name().as_bytes()).into_owned();
        let child_path = join_path(path, strip_version(&name));
        let node = view
            .lookup(dir, entry.name())
            .map_err(|error| error.to_string())?;
        let result = if entry.file_type().is_dir() {
            out.insert(child_path.clone(), EntryData::Directory);
            snapshot_dir(view, node, &child_path, out)
        } else {
            read_node(view, node)
                .map(|contents| {
                    out.insert(child_path, EntryData::File(contents));
                })
                .map_err(|error| error.to_string())
        };
        view.forget(node, 1);
        result?;
    }
    Ok(())
}
