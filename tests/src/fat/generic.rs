//! One adapter for every implementation of the `hadris-fs` `FileSystem`
//! trait, and the Hadris FAT driver mounted through it.
//!
//! The adapter drives the node API: it resolves the parent directory and
//! passes the last component to `create`, `remove` and `rename`, so the path
//! layer cannot normalise names such as `.` away before the driver sees them.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use hadris_fat::sync::{FatFs, format as format_fat};
use hadris_fat::{FatKind, FormatOptions, VolumeLabel};
use hadris_fs::sync::{FileSystem, StdMutex, Volume};
use hadris_fs::{
    Attributes, DirCursor, FileType, FsResult, Name, NameBuf, NewNode, NodeId, RemoveKind,
    RenameFlags, SetMetadata,
};

use super::adapter::FatAdapter;
use super::model::{EntryState, FsState, Operation};
use super::{ARCHIVE, FatCase, HIDDEN, LABEL, READ_ONLY, SYSTEM};
use crate::harness::tree::EntryData;
use crate::harness::{join_path, split_parent};

/// FAT attribute bits and the `hadris-fs` attributes they map to.
const ATTRIBUTES: [(u8, Attributes); 4] = [
    (READ_ONLY, Attributes::READ_ONLY),
    (HIDDEN, Attributes::HIDDEN),
    (SYSTEM, Attributes::SYSTEM),
    (ARCHIVE, Attributes::ARCHIVE),
];

/// How to mount an image as a [`FileSystem`].
pub trait Mount {
    type Fs: FileSystem;

    /// Mounts the image at `image` for reading and writing.
    fn mount(&self, image: &Path) -> Result<Self::Fs, String>;

    /// The volume label as the model records it: trimmed, empty when absent.
    fn label(&self, fs: &Self::Fs) -> Result<String, String>;
}

/// A [`FatAdapter`] over any [`FileSystem`]. Every operation mounts the
/// image, runs, syncs and unmounts, so each step is committed to disk. A
/// failed operation is synced too, as a clean unmount after an error would
/// be.
pub struct FsAdapter<M> {
    image: PathBuf,
    mount: M,
}

impl<M: Mount> FsAdapter<M> {
    pub fn with_mount(image: PathBuf, mount: M) -> Self {
        Self { image, mount }
    }
}

impl<M: Mount + Default> FsAdapter<M> {
    pub fn new(image: PathBuf) -> Self {
        Self::with_mount(image, M::default())
    }
}

impl<M: Mount> FatAdapter for FsAdapter<M>
where
    <M::Fs as FileSystem>::DeviceError: Display,
{
    fn apply(&mut self, operation: &Operation) -> Result<(), String> {
        let fs = self.mount.mount(&self.image)?;
        let mut pins = Vec::new();
        let result = apply(&fs, operation, &mut pins);
        for node in pins {
            fs.forget(node);
        }
        let synced = fs.sync().map_err(|error| error.to_string());
        result?;
        synced
    }

    fn snapshot(&mut self) -> Result<FsState, String> {
        let fs = self.mount.mount(&self.image)?;
        let mut state = FsState {
            label: self.mount.label(&fs)?,
            entries: BTreeMap::new(),
        };
        snapshot_dir(&fs, fs.root(), "/", &mut state.entries).map_err(|error| error.to_string())?;
        Ok(state)
    }
}

fn name(text: &str) -> Result<&Name, String> {
    Name::new(text).map_err(|error| format!("{text:?}: {error:?}"))
}

/// Resolves `path` and records the pin.
fn resolve<F: FileSystem>(fs: &F, path: &str, pins: &mut Vec<NodeId>) -> Result<NodeId, String>
where
    F::DeviceError: Display,
{
    let node = fs
        .resolve(path)
        .map_err(|error| format!("{path}: {error}"))?;
    pins.push(node);
    Ok(node)
}

/// The parent directory of `path`, pinned, and the last component.
fn parent<'p, F: FileSystem>(
    fs: &F,
    path: &'p str,
    pins: &mut Vec<NodeId>,
) -> Result<(NodeId, &'p Name), String>
where
    F::DeviceError: Display,
{
    let (dir, last) = split_parent(path)?;
    Ok((resolve(fs, dir, pins)?, name(last)?))
}

fn write_all<F: FileSystem>(
    fs: &F,
    node: NodeId,
    mut offset: u64,
    mut data: &[u8],
) -> FsResult<(), F::DeviceError> {
    while !data.is_empty() {
        let written = fs.write_at(node, offset, data)?;
        if written == 0 {
            return Err(hadris_fs::ErrorKind::NoSpace.into());
        }
        offset += written as u64;
        data = &data[written..];
    }
    Ok(())
}

fn apply<F: FileSystem>(fs: &F, operation: &Operation, pins: &mut Vec<NodeId>) -> Result<(), String>
where
    F::DeviceError: Display,
{
    let err = |error: hadris_fs::Error<F::DeviceError>| error.to_string();
    let meta = SetMetadata::new();
    match operation {
        Operation::CreateDir { path } => {
            let (dir, last) = parent(fs, path, pins)?;
            pins.push(fs.create(dir, last, NewNode::Dir, &meta).map_err(err)?);
        }
        Operation::CreateFile { path, data } => {
            let (dir, last) = parent(fs, path, pins)?;
            let node = fs.create(dir, last, NewNode::File, &meta).map_err(err)?;
            pins.push(node);
            write_all(fs, node, 0, data).map_err(err)?;
        }
        Operation::ReplaceFile { path, data } => {
            let node = resolve(fs, path, pins)?;
            fs.set_len(node, 0).map_err(err)?;
            write_all(fs, node, 0, data).map_err(err)?;
        }
        Operation::AppendFile { path, data } => {
            let node = resolve(fs, path, pins)?;
            let len = fs.node_metadata(node).map_err(err)?.len();
            write_all(fs, node, len, data).map_err(err)?;
        }
        Operation::TruncateFile { path, len } => {
            let node = resolve(fs, path, pins)?;
            fs.set_len(node, *len as u64).map_err(err)?;
        }
        Operation::Rename { from, to } => {
            let (from_dir, from_name) = parent(fs, from, pins)?;
            let (to_dir, to_name) = parent(fs, to, pins)?;
            fs.rename(
                from_dir,
                from_name,
                to_dir,
                to_name,
                RenameFlags::NO_REPLACE,
            )
            .map_err(err)?;
        }
        Operation::SetAttrs { path, attrs } => {
            let node = resolve(fs, path, pins)?;
            let mut attributes = Attributes::empty();
            for (bit, flag) in ATTRIBUTES {
                if attrs & bit != 0 {
                    attributes |= flag;
                }
            }
            fs.set_metadata(node, &SetMetadata::new().with_attributes(attributes))
                .map_err(err)?;
        }
        Operation::Delete { path } => {
            let (dir, last) = parent(fs, path, pins)?;
            fs.remove(dir, last, RemoveKind::Any).map_err(err)?;
        }
    }
    Ok(())
}

fn snapshot_dir<F: FileSystem>(
    fs: &F,
    dir: NodeId,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> FsResult<(), F::DeviceError> {
    let mut children = Vec::new();
    let mut cursor = DirCursor::start();
    let mut buf = NameBuf::new();
    while let Some(entry) = fs.read_dir_entry(dir, &mut cursor, &mut buf)? {
        let text = String::from_utf8_lossy(buf.as_bytes()).into_owned();
        children.push((text, entry.file_type()));
    }
    for (text, file_type) in children {
        let child_path = join_path(path, &text);
        let node = fs.lookup(dir, Name::new(&text).map_err(|error| error.kind())?)?;
        let result = snapshot_node(fs, node, file_type, &child_path, entries);
        fs.forget(node);
        result?;
    }
    Ok(())
}

fn snapshot_node<F: FileSystem>(
    fs: &F,
    node: NodeId,
    file_type: FileType,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> FsResult<(), F::DeviceError> {
    let meta = fs.node_metadata(node)?;
    let mut attrs = 0;
    for (bit, flag) in ATTRIBUTES {
        if meta.attributes().contains(flag) {
            attrs |= bit;
        }
    }
    let data = if file_type == FileType::Dir {
        EntryData::Directory
    } else {
        let mut contents = vec![0u8; meta.len() as usize];
        let mut done = 0;
        while done < contents.len() {
            let n = fs.read_at(node, done as u64, &mut contents[done..])?;
            if n == 0 {
                break;
            }
            done += n;
        }
        contents.truncate(done);
        EntryData::File(contents)
    };
    entries.insert(path.to_string(), EntryState { data, attrs });
    if file_type == FileType::Dir {
        snapshot_dir(fs, node, path, entries)?;
    }
    Ok(())
}

pub const NAME: &str = "Hadris";

/// The Hadris FAT driver, `FatFs`, on the image file, shared through a
/// `Volume`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HadrisFat;

impl Mount for HadrisFat {
    type Fs = Volume<FatFs<File>, StdMutex>;

    fn mount(&self, image: &Path) -> Result<Self::Fs, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(image)
            .map_err(|error| error.to_string())?;
        let fs = FatFs::open(file).map_err(|error| error.to_string())?;
        Ok(Volume::new(fs))
    }

    fn label(&self, fs: &Self::Fs) -> Result<String, String> {
        let label = fs.lock().label().map_err(|error| error.to_string())?;
        Ok(label
            .map(|label| label.as_str().to_string())
            .unwrap_or_default())
    }
}

pub type HadrisFatAdapter = FsAdapter<HadrisFat>;

/// Formats `path` with the Hadris formatter and verifies the selected width.
pub fn format(path: &Path, case: FatCase) -> Result<(), String> {
    let kind = match case.bits {
        12 => FatKind::Fat12,
        16 => FatKind::Fat16,
        32 => FatKind::Fat32,
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
    let label = VolumeLabel::new(LABEL).map_err(|error| error.to_string())?;
    let options = FormatOptions::new()
        .with_kind(kind)
        .with_label(label)
        .with_volume_id(0x4841_4452);
    let fs = format_fat(file, options).map_err(|error| error.to_string())?;
    if fs.kind() != kind {
        return Err(format!(
            "{} formatted as {:?}, expected {kind:?}",
            case.name,
            fs.kind()
        ));
    }
    Ok(())
}
