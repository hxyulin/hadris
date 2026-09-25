//! One adapter for every implementation of the `hadris-fs` `FileSystem`
//! trait, and the Hadris FAT driver mounted through it.
//!
//! The adapter drives the node API: it resolves the parent directory and
//! passes the last component to `create`, `mkdir`, `unlink`, `rmdir` and
//! `rename`, so the path layer cannot normalise names such as `.` away
//! before the driver sees them.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use hadris_fat::sync::{FatFs, format as format_fat};
use hadris_fat::{FatKind, FatOptions, VolumeLabel};
use hadris_fs::sync::FileSystem;
use hadris_fs::{
    Attributes, DirCursor, ErrorKind, FileType, FsResult, MountOptions, Name, NodeId, OpenMode,
    RenameMode, Resolve, SetAttr,
};
use hadris_storage::host::FileDevice;

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
    fn label(&self, fs: &mut Self::Fs) -> Result<String, String>;
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
        let mut fs = self.mount.mount(&self.image)?;
        let mut pins = Vec::new();
        let result = apply(&mut fs, operation, &mut pins);
        for node in pins {
            fs.forget(node, 1);
        }
        let synced = fs.sync().map_err(|error| error.to_string());
        result?;
        synced
    }

    fn snapshot(&mut self) -> Result<FsState, String> {
        let mut fs = self.mount.mount(&self.image)?;
        let mut state = FsState {
            label: self.mount.label(&mut fs)?,
            entries: BTreeMap::new(),
        };
        let root = fs.root();
        snapshot_dir(&mut fs, root, "/", &mut state.entries).map_err(|error| error.to_string())?;
        Ok(state)
    }
}

/// Resolves `path` and records the pin.
fn resolve<F: FileSystem>(fs: &mut F, path: &str, pins: &mut Vec<NodeId>) -> Result<NodeId, String>
where
    F::DeviceError: Display,
{
    let node = fs
        .resolve(path.as_bytes(), Resolve::Lexical)
        .map_err(|error| format!("{path}: {error}"))?;
    pins.push(node);
    Ok(node)
}

/// The parent directory of `path`, pinned, and the last component.
fn parent<'p, F: FileSystem>(
    fs: &mut F,
    path: &'p str,
    pins: &mut Vec<NodeId>,
) -> Result<(NodeId, &'p Name), String>
where
    F::DeviceError: Display,
{
    let (dir, last) = split_parent(path)?;
    Ok((resolve(fs, dir, pins)?, Name::new(last)))
}

/// Opens `node` for writing, runs `job` and closes it again.
fn written<F: FileSystem>(
    fs: &mut F,
    node: NodeId,
    job: impl FnOnce(&mut F) -> FsResult<(), F::DeviceError>,
) -> FsResult<(), F::DeviceError> {
    fs.open(node, OpenMode::Write)?;
    let done = job(fs);
    let closed = fs.close(node);
    done.and(closed)
}

fn write_all<F: FileSystem>(
    fs: &mut F,
    node: NodeId,
    mut offset: u64,
    mut data: &[u8],
) -> FsResult<(), F::DeviceError> {
    while !data.is_empty() {
        let written = fs.write(node, offset, data)?;
        if written == 0 {
            return Err(hadris_fs::ErrorKind::NoSpace.into());
        }
        offset += written as u64;
        data = &data[written..];
    }
    Ok(())
}

fn apply<F: FileSystem>(
    fs: &mut F,
    operation: &Operation,
    pins: &mut Vec<NodeId>,
) -> Result<(), String>
where
    F::DeviceError: Display,
{
    let err = |error: hadris_fs::Error<F::DeviceError>| error.to_string();
    let none = SetAttr::new();
    match operation {
        Operation::CreateDir { path } => {
            let (dir, last) = parent(fs, path, pins)?;
            pins.push(fs.mkdir(dir, last, &none).map_err(err)?);
        }
        Operation::CreateFile { path, data } => {
            let (dir, last) = parent(fs, path, pins)?;
            let node = fs.create(dir, last, &none).map_err(err)?;
            pins.push(node);
            written(fs, node, |fs| write_all(fs, node, 0, data)).map_err(err)?;
        }
        Operation::ReplaceFile { path, data } => {
            let node = resolve(fs, path, pins)?;
            written(fs, node, |fs| {
                fs.truncate(node, 0)?;
                write_all(fs, node, 0, data)
            })
            .map_err(err)?;
        }
        Operation::AppendFile { path, data } => {
            let node = resolve(fs, path, pins)?;
            written(fs, node, |fs| {
                let len = fs.stat(node)?.len();
                write_all(fs, node, len, data)
            })
            .map_err(err)?;
        }
        Operation::TruncateFile { path, len } => {
            let node = resolve(fs, path, pins)?;
            written(fs, node, |fs| fs.truncate(node, *len as u64)).map_err(err)?;
        }
        Operation::Rename { from, to } => {
            let (from_dir, from_name) = parent(fs, from, pins)?;
            let (to_dir, to_name) = parent(fs, to, pins)?;
            fs.rename(from_dir, from_name, to_dir, to_name, RenameMode::NoReplace)
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
            fs.setattr(node, &SetAttr::new().with_attributes(attributes))
                .map_err(err)?;
        }
        Operation::Delete { path } => {
            let (dir, last) = parent(fs, path, pins)?;
            match fs.unlink(dir, last) {
                Err(error) if error.kind() == ErrorKind::IsADirectory => fs.rmdir(dir, last),
                other => other,
            }
            .map_err(err)?;
        }
    }
    Ok(())
}

fn snapshot_dir<F: FileSystem>(
    fs: &mut F,
    dir: NodeId,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> FsResult<(), F::DeviceError> {
    let mut children = Vec::new();
    let mut cursor = DirCursor::START;
    while let Some(entry) = fs.readdir(dir, cursor)? {
        cursor = entry.next_cursor();
        children.push((entry.name().as_bytes().to_vec(), entry.file_type()));
    }
    for (bytes, file_type) in children {
        let child_path = join_path(path, &String::from_utf8_lossy(&bytes));
        let node = fs.lookup(dir, Name::new(&bytes))?;
        let result = snapshot_node(fs, node, file_type, &child_path, entries);
        fs.forget(node, 1);
        result?;
    }
    Ok(())
}

fn snapshot_node<F: FileSystem>(
    fs: &mut F,
    node: NodeId,
    file_type: FileType,
    path: &str,
    entries: &mut BTreeMap<String, EntryState>,
) -> FsResult<(), F::DeviceError> {
    let meta = fs.stat(node)?;
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
        fs.open(node, OpenMode::Read)?;
        let read = loop {
            if done == contents.len() {
                break Ok(());
            }
            match fs.read(node, done as u64, &mut contents[done..]) {
                Ok(0) => break Ok(()),
                Ok(n) => done += n,
                Err(error) => break Err(error),
            }
        };
        let closed = fs.close(node);
        read?;
        closed?;
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

/// The Hadris FAT driver, `FatFs`, on the image file.
#[derive(Debug, Default, Clone, Copy)]
pub struct HadrisFat;

/// The volume label through the trait, as UTF-8 with trailing spaces
/// removed, or empty when absent.
pub fn label_of<F: FileSystem>(fs: &mut F) -> Result<String, String>
where
    F::DeviceError: Display,
{
    let mut buf = [0u8; 384];
    let label = fs.label(&mut buf).map_err(|error| error.to_string())?;
    Ok(label
        .map(|label| label.trim_end().to_string())
        .unwrap_or_default())
}

impl Mount for HadrisFat {
    type Fs = FatFs<FileDevice>;

    fn mount(&self, image: &Path) -> Result<Self::Fs, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(image)
            .and_then(FileDevice::new)
            .map_err(|error| error.to_string())?;
        FatFs::mount(file, MountOptions::new()).map_err(|error| error.to_string())
    }

    fn label(&self, fs: &mut Self::Fs) -> Result<String, String> {
        label_of(fs)
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
    let mut file = FileDevice::new(file).map_err(|error| error.to_string())?;
    let label = VolumeLabel::new(LABEL).map_err(|error| error.to_string())?;
    let options = FatOptions::new()
        .with_kind(kind)
        .with_label(label)
        .with_serial(0x4841_4452);
    let geometry = format_fat(&mut file, &options).map_err(|error| error.to_string())?;
    if geometry.kind() != kind {
        return Err(format!(
            "{} formatted as {:?}, expected {kind:?}",
            case.name,
            geometry.kind()
        ));
    }
    Ok(())
}
