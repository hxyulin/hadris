#![allow(deprecated)]

use alloc::{collections::BTreeMap, sync::Arc};
use core::fmt;

pub mod estimator;
pub mod writer;

use super::super::boot::{
    BootCatalog, BootInfoTable, BootSectionEntry, ElToritoWriter, Grub2BootInfoTable,
};
use super::super::directory::{DirectoryRecord, DirectoryRef, FileFlags};
use super::super::io::{self, Read, Seek, SeekFrom, Write};
use super::super::io::{IsoCursor, LogicalSector};
use super::super::path::PathTableRef;
use super::super::read::PathSeparator;
use super::super::rrip::{RripBuilder, RripOptions};
use super::super::susp::SplitSu;
use super::super::volume::{
    BootRecordVolumeDescriptor, PrimaryVolumeDescriptor, SupplementaryVolumeDescriptor,
    VolumeDescriptor, VolumeDescriptorHeader, VolumeDescriptorList, VolumeDescriptorType,
};
use crate::file::EntryType;
use crate::joliet::JolietLevel;
use crate::types::{Charset, IsoStr};
use hadris_common::types::{
    endian::{Endian, EndianType},
    number::U32,
};
use hadris_part::{
    Le,
    gpt::{GptPartitionEntry, Guid},
    hybrid::HybridMbrBuilder,
    mbr::{Chs, MasterBootRecord, MbrPartition, MbrPartitionType},
};
use options::PartitionScheme;
use writer::{DirectoryRelocation, PathTableWriter, WrittenDirectory, WrittenFile, WrittenFiles};

use alloc::{collections::VecDeque, string::String, vec, vec::Vec};

pub mod options;
use options::FormatOptions;

#[derive(Debug, thiserror::Error)]
pub enum FileConversionError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Path {0:?} is not a valid UTF-8 string")]
    InvalidUtf8Path(std::path::PathBuf),
    #[error("Unsupported filesystem entry type at {0:?}")]
    UnsupportedFileType(std::path::PathBuf),
}

#[deprecated(since = "2.0.0", note = "use `InputTree::from_fs`")]
impl InputFiles {
    pub fn from_fs(
        root_path: &std::path::Path,
        path_separator: PathSeparator,
    ) -> core::result::Result<Self, FileConversionError> {
        if !root_path.is_dir() {
            return Err(FileConversionError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                alloc::format!("Root path '{root_path:?}' is not a directory"),
            )));
        }

        let children = read_directory_recursively(root_path)?;

        Ok(Self {
            path_separator,
            files: children,
        })
    }
}

/// Recursively reads a directory into the legacy input model.
#[allow(deprecated)]
fn read_directory_recursively(
    current_path: &std::path::Path,
) -> core::result::Result<Vec<File>, FileConversionError> {
    use alloc::string::ToString;
    let mut children_files: Vec<File> = Vec::new();

    for entry_result in std::fs::read_dir(current_path)? {
        let entry = entry_result?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|os_str| os_str.to_str())
            .ok_or_else(|| FileConversionError::InvalidUtf8Path(path.clone()))?
            .to_string();

        if path.is_file() {
            let contents = std::fs::read(&path)?;
            children_files.push(File::File {
                name: Arc::new(name),
                contents,
            });
        } else if path.is_dir() {
            let grand_children = read_directory_recursively(&path)?;
            children_files.push(File::Directory {
                name: Arc::new(name),
                children: grand_children,
            });
        }
        // Else: ignore other file types (e.g., symlinks, pipes) for now
    }

    // Sort files and directories for consistent ISO ordering (optional, but good practice)
    children_files.sort_by_key(|f| f.name().to_ascii_lowercase());

    Ok(children_files)
}

#[deprecated(since = "2.0.0", note = "use `InputTree`")]
pub struct InputFiles {
    pub path_separator: PathSeparator,
    pub files: Vec<File>,
}

#[deprecated(since = "2.0.0", note = "use `InputEntry` and `InputEntryKind`")]
#[derive(Clone, PartialEq, Eq)]
pub enum File {
    File {
        name: Arc<String>,
        contents: Vec<u8>,
    },
    Directory {
        name: Arc<String>,
        children: Vec<File>,
    },
}

#[allow(deprecated)]
impl core::fmt::Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut dbg = f.debug_struct("File");
        match self {
            Self::Directory { name, children } => {
                dbg.field("name", name);
                dbg.field("children", children);
            }
            Self::File { name, contents } => {
                dbg.field("name", name);
                dbg.field("data_len", &contents.len());
            }
        }
        dbg.finish()
    }
}

#[allow(deprecated)]
impl File {
    pub fn name(&self) -> Arc<String> {
        match self {
            File::File { name, .. } => name.clone(),
            File::Directory { name, .. } => name.clone(),
        }
    }
}

/// A metadata-aware tree used to create an ISO image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputTree {
    pub path_separator: PathSeparator,
    pub entries: Vec<InputEntry>,
}

/// Optional POSIX metadata for an input entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputMetadata {
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// Creation time as seconds since the Unix epoch.
    pub created: Option<i64>,
    /// Modification time as seconds since the Unix epoch.
    pub modified: Option<i64>,
    /// Access time as seconds since the Unix epoch.
    pub accessed: Option<i64>,
}

/// The data represented by an [`InputEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEntryKind {
    File(Vec<u8>),
    Directory(Vec<InputEntry>),
    Symlink(String),
    CharacterDevice { major: u32, minor: u32 },
    BlockDevice { major: u32, minor: u32 },
}

/// A named input entry and its optional host metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEntry {
    pub name: Arc<String>,
    pub kind: InputEntryKind,
    pub metadata: InputMetadata,
}

impl InputEntry {
    pub fn file(name: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        Self::new(name, InputEntryKind::File(contents.into()))
    }

    pub fn directory(name: impl Into<String>, children: Vec<Self>) -> Self {
        Self::new(name, InputEntryKind::Directory(children))
    }

    pub fn symlink(name: impl Into<String>, target: impl Into<String>) -> Self {
        Self::new(name, InputEntryKind::Symlink(target.into()))
    }

    pub fn character_device(name: impl Into<String>, major: u32, minor: u32) -> Self {
        Self::new(name, InputEntryKind::CharacterDevice { major, minor })
    }

    pub fn block_device(name: impl Into<String>, major: u32, minor: u32) -> Self {
        Self::new(name, InputEntryKind::BlockDevice { major, minor })
    }

    pub fn with_metadata(mut self, metadata: InputMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn name(&self) -> Arc<String> {
        self.name.clone()
    }

    fn new(name: impl Into<String>, kind: InputEntryKind) -> Self {
        Self {
            name: Arc::new(name.into()),
            kind,
            metadata: InputMetadata::default(),
        }
    }
}

impl InputTree {
    pub fn new(path_separator: PathSeparator, entries: Vec<InputEntry>) -> Self {
        Self {
            path_separator,
            entries,
        }
    }

    pub fn from_fs(
        root_path: &std::path::Path,
        path_separator: PathSeparator,
    ) -> core::result::Result<Self, FileConversionError> {
        if !root_path.is_dir() {
            return Err(FileConversionError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                alloc::format!("Root path '{root_path:?}' is not a directory"),
            )));
        }
        Ok(Self::new(
            path_separator,
            read_input_directory_recursively(root_path)?,
        ))
    }
}

#[allow(deprecated)]
impl From<InputFiles> for InputTree {
    fn from(value: InputFiles) -> Self {
        fn convert(file: File) -> InputEntry {
            match file {
                File::File { name, contents } => InputEntry {
                    name,
                    kind: InputEntryKind::File(contents),
                    metadata: InputMetadata::default(),
                },
                File::Directory { name, children } => InputEntry {
                    name,
                    kind: InputEntryKind::Directory(children.into_iter().map(convert).collect()),
                    metadata: InputMetadata::default(),
                },
            }
        }
        Self::new(
            value.path_separator,
            value.files.into_iter().map(convert).collect(),
        )
    }
}

fn system_time_seconds(value: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    value
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

fn read_input_directory_recursively(
    current_path: &std::path::Path,
) -> core::result::Result<Vec<InputEntry>, FileConversionError> {
    use alloc::string::ToString;
    let mut children = Vec::new();
    for entry in std::fs::read_dir(current_path)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| FileConversionError::InvalidUtf8Path(path.clone()))?
            .to_string();
        let fs_metadata = std::fs::symlink_metadata(&path)?;
        let file_type = fs_metadata.file_type();
        let mut metadata = InputMetadata {
            created: system_time_seconds(fs_metadata.created()),
            modified: system_time_seconds(fs_metadata.modified()),
            accessed: system_time_seconds(fs_metadata.accessed()),
            ..InputMetadata::default()
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.mode = Some(fs_metadata.mode() & 0o7777);
            metadata.uid = Some(fs_metadata.uid());
            metadata.gid = Some(fs_metadata.gid());
        }
        let kind = if file_type.is_file() {
            InputEntryKind::File(std::fs::read(&path)?)
        } else if file_type.is_dir() {
            InputEntryKind::Directory(read_input_directory_recursively(&path)?)
        } else if file_type.is_symlink() {
            let target = std::fs::read_link(&path)?;
            InputEntryKind::Symlink(
                target
                    .to_str()
                    .ok_or_else(|| FileConversionError::InvalidUtf8Path(target.clone()))?
                    .to_string(),
            )
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::{FileTypeExt, MetadataExt};
                let device = fs_metadata.rdev();
                let major = ((device >> 8) & 0xfff) | ((device >> 32) & 0xfffff000);
                let minor = (device & 0xff) | ((device >> 12) & 0xffffff00);
                if file_type.is_char_device() {
                    InputEntryKind::CharacterDevice {
                        major: major as u32,
                        minor: minor as u32,
                    }
                } else if file_type.is_block_device() {
                    InputEntryKind::BlockDevice {
                        major: major as u32,
                        minor: minor as u32,
                    }
                } else {
                    return Err(FileConversionError::UnsupportedFileType(path));
                }
            }
            #[cfg(not(unix))]
            return Err(FileConversionError::UnsupportedFileType(path));
        };
        children.push(InputEntry {
            name: Arc::new(name),
            kind,
            metadata,
        });
    }
    children.sort_by_key(|entry| entry.name.to_ascii_lowercase());
    Ok(children)
}

fn validate_input_tree(tree: &InputTree, rrip: Option<&RripOptions>) -> io::Result<()> {
    fn visit(
        entries: &[InputEntry],
        rrip: Option<&RripOptions>,
        depth: usize,
        path_len: usize,
    ) -> io::Result<()> {
        for entry in entries {
            match &entry.kind {
                InputEntryKind::Directory(children) => {
                    let child_path_len = if path_len == 0 {
                        entry.name.len()
                    } else {
                        path_len + 1 + entry.name.len()
                    };
                    if (depth >= 8 || child_path_len > 255)
                        && !rrip
                            .is_some_and(|options| options.enabled && options.relocate_deep_dirs)
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "directory depth or path length exceeds ISO 9660 limits and RRIP relocation is disabled",
                        ));
                    }
                    visit(children, rrip, depth + 1, child_path_len)?;
                }
                InputEntryKind::Symlink(_) => {
                    if !rrip.is_some_and(|options| options.enabled && options.preserve_symlinks) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "symbolic links require RRIP preserve_symlinks",
                        ));
                    }
                }
                InputEntryKind::CharacterDevice { .. } | InputEntryKind::BlockDevice { .. } => {
                    if !rrip.is_some_and(|options| options.enabled && options.preserve_devices) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "device entries require RRIP preserve_devices",
                        ));
                    }
                }
                InputEntryKind::File(_) => {}
            }
        }
        Ok(())
    }
    visit(&tree.entries, rrip, 1, 0)
}

fn relocate_deep_directories(files: &mut WrittenFiles) {
    fn visit(
        dir: &mut WrittenDirectory,
        physical_depth: usize,
        physical_path_len: usize,
        moved: &mut Vec<WrittenDirectory>,
        internal_id: &mut usize,
    ) {
        let mut retained = Vec::with_capacity(dir.dirs.len());
        for mut child in core::mem::take(&mut dir.dirs) {
            let child_path_len = if physical_path_len == 0 {
                child.name.len()
            } else {
                physical_path_len + 1 + child.name.len()
            };
            if physical_depth + 1 > 8 || child_path_len > 255 {
                let target = child.id;
                let logical_parent = dir.id;
                let original_name = child.rrip_name.clone();
                child.name = Arc::new(alloc::format!("RRD{:06}", *internal_id));
                *internal_id += 1;
                child.relocation = DirectoryRelocation::Moved {
                    id: target,
                    logical_parent,
                };
                let relocated_path_len = "RR_MOVED".len() + 1 + child.name.len();
                visit(&mut child, 3, relocated_path_len, moved, internal_id);
                moved.push(child);

                let mut placeholder = WrittenDirectory::new(original_name);
                placeholder.relocation = DirectoryRelocation::Placeholder { target };
                retained.push(placeholder);
            } else {
                visit(
                    &mut child,
                    physical_depth + 1,
                    child_path_len,
                    moved,
                    internal_id,
                );
                retained.push(child);
            }
        }
        dir.dirs = retained;
    }

    let root = files.get_mut(&files.root_dir());
    let mut moved = Vec::new();
    let mut internal_id = 1;
    visit(root, 1, 0, &mut moved, &mut internal_id);
    if moved.is_empty() {
        return;
    }

    let occupied = root
        .dirs
        .iter()
        .map(|directory| directory.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut relocation_name = String::from("RR_MOVED");
    let mut suffix = 1;
    while occupied.contains(relocation_name.as_str()) {
        relocation_name = alloc::format!("RR_MOVED_{suffix}");
        suffix += 1;
    }
    let mut relocation_dir = WrittenDirectory::new(Arc::new(relocation_name));
    relocation_dir.id = usize::MAX;
    relocation_dir.dirs = moved;
    root.dirs.insert(0, relocation_dir);
}

#[derive(Debug, thiserror::Error)]
pub enum IsoCreationError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Canonical error for ISO creation operations.
pub type Error = IsoCreationError;
/// Canonical result for ISO creation operations.
pub type Result<T> = core::result::Result<T, Error>;
/// Compatibility alias for the ISO creation result.
pub type IsoCreationResult<T> = Result<T>;

pub struct IsoImageWriter<DATA: Read + Write + Seek> {
    data: IsoCursor<DATA>,
    entry_types: Vec<EntryType>,
    ops: FormatOptions,
    written_files: WrittenFiles,
    path_tables: BTreeMap<EntryType, PathTableRef>,
    inode_counter: u32,
    rrip_time: [u8; 7],
}

/// The kind of directory entry, used to select which RRIP entries to emit.
enum RripEntryKind<'a> {
    /// Root directory's "." entry — needs SP + ER + PX + NM(CURRENT)
    RootDot { metadata: InputMetadata, nlink: u32 },
    /// Root directory's ".." entry — needs PX + NM(PARENT)
    RootDotDot { metadata: InputMetadata, nlink: u32 },
    /// Non-root "." entry — needs PX + NM(CURRENT)
    Dot { metadata: InputMetadata, nlink: u32 },
    /// Non-root ".." entry — needs PX + NM(PARENT)
    DotDot { metadata: InputMetadata, nlink: u32 },
    /// A named directory entry
    Directory {
        original_name: &'a str,
        metadata: InputMetadata,
        nlink: u32,
    },
    /// A named file entry
    Entry {
        original_name: &'a str,
        metadata: InputMetadata,
        kind: &'a InputEntryKind,
    },
}

/// Compute the available system use space in a DirectoryRecord given
/// the ISO name length. The record is 256 bytes max; the fixed header
/// is 33 bytes, followed by the name (padded to even).
fn available_su_space(iso_name_len: usize) -> usize {
    let used = (33 + iso_name_len + 1) & !1; // pad to even boundary
    256usize.saturating_sub(used)
}

/// Build complete RRIP entries for a directory record.
///
/// Entries are ordered by priority (most important first, largest last),
/// so that `build_split` keeps the important ones inline and overflows
/// the rest via a CE pointer.
fn rrip_datetime(timestamp: Option<i64>, fallback: &[u8; 7]) -> [u8; 7] {
    use chrono::{Datelike, Timelike};
    let Some(timestamp) = timestamp.and_then(|value| chrono::DateTime::from_timestamp(value, 0))
    else {
        return *fallback;
    };
    [
        (timestamp.year() - 1900).clamp(0, 255) as u8,
        timestamp.month() as u8,
        timestamp.day() as u8,
        timestamp.hour() as u8,
        timestamp.minute() as u8,
        timestamp.second() as u8,
        0,
    ]
}

fn build_rrip_entries(
    kind: RripEntryKind<'_>,
    inode: u32,
    options: &RripOptions,
    fallback_time: &[u8; 7],
) -> RripBuilder {
    let mut builder = RripBuilder::new();
    let add_common = |builder: &mut RripBuilder,
                      metadata: InputMetadata,
                      type_mode: u32,
                      default_permissions: u32,
                      nlink: u32| {
        let permissions = if options.preserve_permissions {
            metadata.mode.unwrap_or(default_permissions)
        } else {
            default_permissions
        };
        let (uid, gid) = if options.preserve_ownership {
            (metadata.uid.unwrap_or(0), metadata.gid.unwrap_or(0))
        } else {
            (0, 0)
        };
        builder.add_px(type_mode | permissions, nlink, uid, gid, inode);
        if options.preserve_timestamps {
            let modified = rrip_datetime(metadata.modified, fallback_time);
            let accessed = rrip_datetime(metadata.accessed, fallback_time);
            builder.add_tf_short(&modified, &accessed);
        }
    };

    match &kind {
        RripEntryKind::RootDot { metadata, nlink } => {
            builder.add_sp(0);
            add_common(&mut builder, *metadata, 0o040000, 0o755, *nlink);
            builder.add_nm_current();
            builder.add_rrip_er(); // full ER, last (largest)
        }
        RripEntryKind::RootDotDot { metadata, nlink } => {
            add_common(&mut builder, *metadata, 0o040000, 0o755, *nlink);
            builder.add_nm_parent();
        }
        RripEntryKind::Dot { metadata, nlink } => {
            add_common(&mut builder, *metadata, 0o040000, 0o755, *nlink);
            builder.add_nm_current();
        }
        RripEntryKind::DotDot { metadata, nlink } => {
            add_common(&mut builder, *metadata, 0o040000, 0o755, *nlink);
            builder.add_nm_parent();
        }
        RripEntryKind::Directory {
            original_name,
            metadata,
            nlink,
        } => {
            add_common(&mut builder, *metadata, 0o040000, 0o755, *nlink);
            builder.add_nm(original_name.as_bytes());
        }
        RripEntryKind::Entry {
            original_name,
            metadata,
            kind,
        } => {
            let (type_mode, default_permissions) = match kind {
                InputEntryKind::File(_) => (0o100000, 0o644),
                InputEntryKind::Symlink(_) => (0o120000, 0o777),
                InputEntryKind::CharacterDevice { .. } => (0o020000, 0o600),
                InputEntryKind::BlockDevice { .. } => (0o060000, 0o600),
                InputEntryKind::Directory(_) => unreachable!(),
            };
            add_common(&mut builder, *metadata, type_mode, default_permissions, 1);
            builder.add_nm(original_name.as_bytes());
            match kind {
                InputEntryKind::Symlink(target) => {
                    builder.add_sl(target);
                }
                InputEntryKind::CharacterDevice { major, minor }
                | InputEntryKind::BlockDevice { major, minor } => {
                    builder.add_pn(*major, *minor);
                }
                _ => {}
            }
        }
    }

    builder
}

/// Apply a deduplication suffix to a name, producing e.g. `READM_1.TXT;1`.
///
/// The suffix `_N` is inserted before the extension (and before any `;1` version
/// suffix). The basename is truncated if needed to stay within format limits.
fn apply_dedup_suffix(name: &[u8], n: usize, ty: EntryType) -> Vec<u8> {
    let suffix = alloc::format!("_{n}");
    let suffix_bytes = suffix.as_bytes();

    match ty {
        EntryType::Joliet { .. } => {
            // Joliet: UTF-16 BE, find the dot (0x00 0x2E) or end
            let mut dot_pos = None;
            let mut i = 0;
            while i + 1 < name.len() {
                if name[i] == 0x00 && name[i + 1] == 0x2E {
                    dot_pos = Some(i);
                }
                i += 2;
            }
            let (basename, ext) = match dot_pos {
                Some(pos) => (&name[..pos], &name[pos..]),
                None => (name, &[][..]),
            };
            // Convert suffix to UTF-16 BE
            let suffix_u16: Vec<u8> = suffix
                .encode_utf16()
                .flat_map(|c| c.to_be_bytes())
                .collect();
            // Max 206 bytes (103 code units) for Joliet
            let max_basename = 206usize.saturating_sub(ext.len() + suffix_u16.len());
            let trunc_basename = &basename[..basename.len().min(max_basename) & !1];
            let mut result =
                Vec::with_capacity(trunc_basename.len() + suffix_u16.len() + ext.len());
            result.extend_from_slice(trunc_basename);
            result.extend_from_slice(&suffix_u16);
            result.extend_from_slice(ext);
            result
        }
        _ => {
            // ASCII-based names (L1, L2, L3)
            // Strip ";1" version suffix if present
            let (base_name, version) = if name.ends_with(b";1") {
                (&name[..name.len() - 2], &b";1"[..])
            } else {
                (name, &[][..])
            };
            // Find the dot separator
            let dot_pos = base_name.iter().rposition(|&b| b == b'.');
            let (basename, ext) = match dot_pos {
                Some(pos) => (&base_name[..pos], &base_name[pos..]),
                None => (base_name, &[][..]),
            };
            // Determine max basename length based on level
            let max_total = match ty {
                EntryType::Level1 { .. } => 8,
                EntryType::Level2 { .. } => 30usize.saturating_sub(ext.len()),
                _ => 207usize.saturating_sub(ext.len() + version.len()),
            };
            let max_basename = max_total.saturating_sub(suffix_bytes.len());
            let trunc_basename = &basename[..basename.len().min(max_basename)];
            let mut result = Vec::with_capacity(
                trunc_basename.len() + suffix_bytes.len() + ext.len() + version.len(),
            );
            result.extend_from_slice(trunc_basename);
            result.extend_from_slice(suffix_bytes);
            result.extend_from_slice(ext);
            result.extend_from_slice(version);
            result
        }
    }
}

/// A pending directory record, built in phase 1 and written in phases 2-3.
struct PendingRecord {
    name: Vec<u8>,
    split: SplitSu,
    dir_ref: DirectoryRef,
    flags: FileFlags,
}

io_transform! {
impl<DATA: Read + Write + Seek> IsoImageWriter<DATA> {
    /// Creates a complete ISO image and returns its output target.
    pub async fn create<T: Into<InputTree>>(
        data: DATA,
        files: T,
        ops: FormatOptions,
    ) -> IsoCreationResult<DATA> {
        let mut files = files.into();
        validate_input_tree(&files, ops.features.rock_ridge.as_ref())?;
        let mut writer = Self::new(data, ops);
        writer.write_volume_descriptors(&mut files).await?;
        let root_dirs = writer.write_files(&files).await?;
        writer.write_path_tables().await?;
        writer.finalize_volume_descriptors(root_dirs).await?;
        Ok(writer.into_inner())
    }

    /// Formats an ISO image while discarding the returned output target.
    #[deprecated(since = "2.0.0", note = "use `create` to recover the output target")]
    pub async fn format_new(
        data: DATA,
        files: InputFiles,
        ops: FormatOptions,
    ) -> IsoCreationResult<()> {
        Self::create(data, files, ops).await.map(|_| ())
    }

    /// Returns the output target.
    pub fn into_inner(self) -> DATA {
        self.data.into_inner()
    }

    fn new(data: DATA, ops: FormatOptions) -> Self {
        let now = super::super::directory::DirDateTime::now();
        let rrip_time = *<&[u8; 7]>::try_from(bytemuck::bytes_of(&now)).unwrap();
        let mut entry_types = Vec::new();
        // The base (PVD) entry type inherits supports_rrip from the filenames config
        entry_types.push(ops.features.filenames.into());
        if ops.features.long_filenames {
            entry_types.push(EntryType::Level3 {
                supports_lowercase: true,
                supports_rrip: false,
            });
        }
        if let Some(joliet) = ops.features.joliet {
            entry_types.push(joliet.into());
        }

        Self {
            data: IsoCursor::new(data, ops.sector_size),
            ops,
            entry_types,
            written_files: WrittenFiles::new(),
            path_tables: BTreeMap::new(),
            inode_counter: 1,
            rrip_time,
        }
    }

    const VOLUME_DESCRIPTOR_SET_START: LogicalSector = LogicalSector(16);

    fn parse_iso_str<C: Charset, const N: usize>(
        &self,
        s: &str,
        field_name: &'static str,
    ) -> io::Result<IsoStr<C, N>> {
        if self.ops.strict_charset {
            IsoStr::from_str_lossy(s)
        } else {
            IsoStr::from_str_unchecked(s)
        }
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, field_name))
    }

    async fn write_volume_descriptors(&mut self, files: &mut InputTree) -> io::Result<()> {
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START).await?;
        let mut volume_descriptors = VolumeDescriptorList::empty();
        for &entry in &self.entry_types {
            match entry {
                EntryType::Level1 { .. } | EntryType::Level2 { .. } => {
                    let mut pvd = PrimaryVolumeDescriptor::new(&self.ops.volume_name, 0);
                    pvd.volume_identifier = self.parse_iso_str(&self.ops.volume_name, "volume name")?;
                    pvd.dir_record.header.len = 34;
                    pvd.dir_record.header.flags = FileFlags::DIRECTORY.bits();
                    pvd.dir_record.header.file_identifier_len = 1;
                    pvd.dir_record.header.volume_sequence_number.write(1);
                    pvd.volume_sequence_number.write(1);
                    if let Some(s) = &self.ops.system_id {
                        pvd.system_identifier = self.parse_iso_str(s, "system identifier")?;
                    }
                    if let Some(s) = &self.ops.volume_set_id {
                        pvd.volume_set_identifier = self.parse_iso_str(s, "volume set identifier")?;
                    }
                    if let Some(s) = &self.ops.publisher_id {
                        pvd.publisher_identifier = self.parse_iso_str(s, "publisher identifier")?;
                    }
                    if let Some(s) = &self.ops.preparer_id {
                        pvd.preparer_identifier = self.parse_iso_str(s, "preparer identifier")?;
                    }
                    if let Some(s) = &self.ops.application_id {
                        pvd.application_identifier = self.parse_iso_str(s, "application identifier")?;
                    }
                    volume_descriptors.push(VolumeDescriptor::Primary(pvd));
                }
                EntryType::Level3 { .. } => {
                    // Version 2 for EVD
                    let mut evd = SupplementaryVolumeDescriptor::new_evd(&self.ops.volume_name, 0);
                    evd.volume_identifier = self.parse_iso_str(&self.ops.volume_name, "volume name")?;
                    evd.dir_record.header.len = 34;
                    evd.dir_record.header.flags = FileFlags::DIRECTORY.bits();
                    evd.dir_record.header.file_identifier_len = 1;
                    evd.dir_record.header.volume_sequence_number.write(1);
                    evd.volume_sequence_number.write(1);
                    volume_descriptors.push(VolumeDescriptor::Supplementary(evd));
                }
                EntryType::Joliet { level, .. } => {
                    let mut svd = SupplementaryVolumeDescriptor::new_svd(
                        &self.ops.volume_name,
                        0,
                        level.escape_sequence(),
                    );
                    svd.dir_record.header.len = 34;
                    svd.dir_record.header.flags = FileFlags::DIRECTORY.bits();
                    svd.dir_record.header.file_identifier_len = 1;
                    svd.dir_record.header.volume_sequence_number.write(1);
                    svd.volume_sequence_number.write(1);
                    if let Some(s) = &self.ops.system_id {
                        svd.system_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
                    }
                    if let Some(s) = &self.ops.volume_set_id {
                        svd.volume_set_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
                    }
                    if let Some(s) = &self.ops.publisher_id {
                        svd.publisher_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
                    }
                    if let Some(s) = &self.ops.preparer_id {
                        svd.preparer_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
                    }
                    if let Some(s) = &self.ops.application_id {
                        svd.application_identifier = SupplementaryVolumeDescriptor::utf16be_str(s);
                    }
                    volume_descriptors.push(VolumeDescriptor::Supplementary(svd));
                }
            }
        }

        if let Some(boot) = &self.ops.features.el_torito {
            let boot_record = ElToritoWriter::create_descriptor(boot, files);
            volume_descriptors.insert(1, VolumeDescriptor::BootRecord(boot_record));
        }

        volume_descriptors.write(&mut self.data).await?;
        Ok(())
    }

    async fn finalize_volume_descriptors(
        &mut self,
        root_dirs: BTreeMap<EntryType, DirectoryRef>,
    ) -> io::Result<()> {
        // Write boot catalog
        let catalog_ptr = if let Some(boot) = &self.ops.features.el_torito {
            let mut catalog = BootCatalog::default();
            let current_sector = self.data.pad_align_sector().await?;

            for (section, entry) in boot.sections() {
                let dir_ref = self
                    .written_files
                    .find_file(&entry.boot_image_path, self.ops.path_separator)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::NotFound,
                            "boot image file not found",
                        )
                    })?;
                let load_size = entry
                    .load_size
                    .map(core::num::NonZeroU16::get)
                    .unwrap_or_else(|| dir_ref.size.div_ceil(512) as u16);
                let boot_image_lba = dir_ref.extent.0 as u32;
                let boot_entry =
                    BootSectionEntry::new(entry.emulation, 0, load_size, boot_image_lba);
                if let Some(section) = section {
                    catalog.add_section(section.platform, vec![boot_entry]);
                } else {
                    catalog.set_default_entry(boot_entry);
                }

                // Handle boot info table (standard El-Torito) or GRUB2 boot info
                // Both use similar format at offset 8 in the boot image
                if entry.boot_info_table || entry.grub2_boot_info {
                    // Boot info table requires at least 64 bytes in the boot image
                    // (header is at offset 8-56/64, checksum covers bytes 64+)
                    if dir_ref.size < 64 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "boot image too small for boot info table (minimum 64 bytes)",
                        ));
                    }

                    let mut checksum = 0u32;
                    let mut buffer = [0u8; 4];
                    let byte_offset = (boot_image_lba as u64) * self.ops.sector_size as u64;
                    self.data
                        .seek(SeekFrom::Start(byte_offset + 64))
                        .await
                        .map_err(io::Error::erase)?;
                    // Calculate checksum for all 4-byte chunks from offset 64 to end
                    let checksum_bytes = dir_ref.size - 64;
                    for _ in 0..(checksum_bytes / 4) {
                        self.data.read_exact(&mut buffer).await?;
                        checksum = checksum.wrapping_add(u32::from_le_bytes(buffer));
                    }

                    const TABLE_OFFSET: u64 = 8;
                    self.data
                        .seek(SeekFrom::Start(byte_offset + TABLE_OFFSET))
                        .await
                        .map_err(io::Error::erase)?;

                    if entry.grub2_boot_info {
                        // GRUB2/ISOLINUX uses extended 56-byte format with reserved bytes
                        let table = Grub2BootInfoTable {
                            pvd_lba: U32::new(16),
                            file_lba: U32::new(dir_ref.extent.0 as u32),
                            file_len: U32::new(dir_ref.size as u32),
                            checksum: U32::new(checksum),
                            reserved: [0u8; 40],
                        };
                        self.data.write_all(bytemuck::bytes_of(&table)).await?;
                    } else {
                        // Standard El-Torito 16-byte format
                        let table = BootInfoTable {
                            iso_start: U32::new(16),
                            file_lba: U32::new(dir_ref.extent.0 as u32),
                            file_len: U32::new(dir_ref.size as u32),
                            checksum: U32::new(checksum),
                        };
                        self.data.write_all(bytemuck::bytes_of(&table)).await?;
                    }
                }
            }

            if boot.write_boot_catalog {
                let dir_ref = self
                    .written_files
                    .find_file("boot.catalog", self.ops.path_separator)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::NotFound,
                            "boot.catalog file not found in written files",
                        )
                    })?;
                self.data.seek_sector(dir_ref.extent).await?;
                if dir_ref.size < catalog.size() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "boot.catalog file too small",
                    ));
                }
                catalog.write(&mut self.data).await?;
                self.data.seek_sector(current_sector).await?;

                Some(dir_ref.extent.0 as u32)
            } else {
                self.data.seek_sector(current_sector).await?;
                catalog.write(&mut self.data).await?;
                self.data.pad_align_sector().await?;
                Some(current_sector.0 as u32)
            }
        } else {
            None
        };

        let end_sector = self.data.pad_align_sector().await?;
        self.data.seek_sector(Self::VOLUME_DESCRIPTOR_SET_START).await?;

        let mut buffer = vec![0u8; self.ops.sector_size];
        loop {
            self.data.read_exact(&mut buffer).await?;
            let header = VolumeDescriptorHeader::from_bytes(&buffer[0..7]);
            let ty = VolumeDescriptorType::from_u8(header.descriptor_type);
            if let VolumeDescriptorType::VolumeSetTerminator = ty {
                break;
            }
            if !header.is_valid() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid volume descriptor header during finalization",
                ));
            }

            match ty {
                VolumeDescriptorType::PrimaryVolumeDescriptor => {
                    let base_type = self
                        .entry_types
                        .iter()
                        .find(|e| matches!(e, EntryType::Level1 { .. } | EntryType::Level2 { .. }))
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "no base Level entry type found for PVD",
                            )
                        })?;
                    let root_dir = root_dirs.get(base_type).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "root directory not found for PVD entry type",
                        )
                    })?;
                    let pt = self.path_tables.get(base_type).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "path table not found for PVD entry type",
                        )
                    })?;
                    let pvd = bytemuck::from_bytes_mut::<PrimaryVolumeDescriptor>(&mut buffer);
                    pvd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                    pvd.dir_record.header.data_len.write(root_dir.size as u32);
                    pvd.type_l_path_table.set(pt.lpt.0 as u32);
                    pvd.type_m_path_table.set(pt.mpt.0 as u32);
                    pvd.path_table_size.write(pt.size as u32);
                    pvd.volume_space_size.write(end_sector.0 as u32);
                }
                VolumeDescriptorType::SupplementaryVolumeDescriptor => {
                    let svd =
                        bytemuck::from_bytes_mut::<SupplementaryVolumeDescriptor>(&mut buffer);
                    match svd.header.version {
                        1 => {
                            for &level in JolietLevel::all() {
                                if svd.escape_sequences == level.escape_sequence() {
                                    let Some(joliet) = self
                                        .entry_types
                                        .iter()
                                        .find(
                                            |e| matches!(e, EntryType::Joliet{ level: jl, ..} if *jl == level),
                                        )
                                    else {
                                        continue;
                                    };
                                    let Some(root_dir) = root_dirs.get(joliet) else {
                                        continue;
                                    };
                                    let Some(pt) = self.path_tables.get(joliet) else {
                                        continue;
                                    };

                                    svd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                                    svd.dir_record.header.data_len.write(root_dir.size as u32);
                                    svd.type_l_path_table.set(pt.lpt.0 as u32);
                                    svd.type_m_path_table.set(pt.mpt.0 as u32);
                                    svd.path_table_size.write(pt.size as u32);
                                    svd.volume_space_size.write(end_sector.0 as u32);
                                }
                            }
                        }
                        2 => {
                            if svd.escape_sequences != [b' '; 32] {
                                // We don't recognize this EVD
                                continue;
                            }

                            let Some(l3) = self
                                .entry_types
                                .iter()
                                .find(|e| matches!(e, EntryType::Level3 { .. }))
                            else {
                                continue;
                            };
                            let Some(root_dir) = root_dirs.get(l3) else {
                                continue;
                            };
                            svd.dir_record.header.extent.write(root_dir.extent.0 as u32);
                            svd.dir_record.header.data_len.write(root_dir.size as u32);
                            svd.volume_space_size.write(end_sector.0 as u32);
                        }

                        // Unknown version
                        _ => {}
                    }
                }
                VolumeDescriptorType::BootRecord => {
                    let Some(catalog_ptr) = catalog_ptr else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "boot record found but no boot catalog was written",
                        ));
                    };
                    let boot_record =
                        bytemuck::from_bytes_mut::<BootRecordVolumeDescriptor>(&mut buffer);
                    boot_record.catalog_ptr.set(catalog_ptr);
                }
                // We don't do anything
                _ => continue,
            }

            // Write the new data
            self.data
                .seek_relative(-(buffer.len() as i64))
                .await
                .map_err(io::Error::erase)?;
            self.data.write_all(&buffer).await?;
        }

        // Now we finalize the partition tables based on hybrid boot options
        self.write_partition_tables(end_sector).await?;

        Ok(())
    }

    async fn write_files(&mut self, files: &InputTree) -> io::Result<BTreeMap<EntryType, DirectoryRef>> {
        let mut next_directory_id = 1usize;
        {
            let walker = FileTreeWalker::new(files);
            let mut current_dir = self.written_files.root_dir();
            for file in walker {
                match file {
                    TreeWalkerItem::EnterDirectory(dir) => {
                        let name = dir.name();
                        let metadata = dir.metadata;
                        let written_dir = self.written_files.get_mut(&current_dir);
                        let index = written_dir.push_dir(name, metadata);
                        written_dir.dirs[index].id = next_directory_id;
                        next_directory_id += 1;
                        current_dir.push(index);
                    }
                    TreeWalkerItem::ExitDirectory(_dir) => {
                        current_dir.pop();
                    }
                    TreeWalkerItem::File(file) => {
                        if let InputEntryKind::File(contents) = &file.kind {
                            // Handle zero-size files specially:
                            // Per ISO 9660, empty files should have extent location of 0
                            // since there is no data to reference.
                            let entry = if contents.is_empty() {
                                DirectoryRef {
                                    extent: LogicalSector(0),
                                    size: 0,
                                }
                            } else {
                                let start = self.data.pad_align_sector().await?;
                                self.data.write_all(contents).await?;
                                DirectoryRef {
                                    extent: start,
                                    size: contents.len(),
                                }
                            };
                            let dir = self.written_files.get_mut(&current_dir);
                            dir.files.push(WrittenFile {
                                name: file.name.clone(),
                                entry,
                                kind: file.kind.clone(),
                                metadata: file.metadata,
                            });
                        } else {
                            let dir = self.written_files.get_mut(&current_dir);
                            dir.files.push(WrittenFile {
                                name: file.name.clone(),
                                entry: DirectoryRef {
                                    extent: LogicalSector(0),
                                    size: 0,
                                },
                                kind: file.kind.clone(),
                                metadata: file.metadata,
                            });
                        }
                    }
                };
            }
        }

        if self
            .ops
            .features
            .rock_ridge
            .is_some_and(|options| options.enabled && options.relocate_deep_dirs)
        {
            relocate_deep_directories(&mut self.written_files);
        }

        fn collect_postorder(
            files: &WrittenFiles,
            id: &writer::DirectoryId,
            output: &mut Vec<writer::DirectoryId>,
        ) {
            let dir = files.get(id);
            for (index, child) in dir.dirs.iter().enumerate() {
                if matches!(child.relocation, DirectoryRelocation::Placeholder { .. }) {
                    continue;
                }
                let mut child_id = id.clone();
                child_id.push(index);
                collect_postorder(files, &child_id, output);
            }
            output.push(id.clone());
        }

        let root_id = self.written_files.root_dir();
        let mut order = Vec::new();
        collect_postorder(&self.written_files, &root_id, &mut order);
        let mut relocation_refs = BTreeMap::new();
        for directory_id in order {
            let is_root = directory_id == root_id;
            for ty in &self.entry_types {
                let dir = self.written_files.get_mut(&directory_id);
                Self::write_directory(
                    &mut self.data,
                    *ty,
                    dir,
                    is_root,
                    &mut self.inode_counter,
                    self.ops.features.rock_ridge.as_ref(),
                    &self.rrip_time,
                    &relocation_refs,
                )
                .await?;
            }
            let dir = self.written_files.get(&directory_id);
            for (ty, reference) in &dir.entries {
                relocation_refs.insert((dir.id, *ty), *reference);
            }
            if let DirectoryRelocation::Moved { id, .. } = dir.relocation {
                for (ty, reference) in &dir.entries {
                    relocation_refs.insert((id, *ty), *reference);
                }
            }
        }
        fn collect_moved(
            directory: &WrittenDirectory,
            output: &mut Vec<(usize, usize, BTreeMap<EntryType, DirectoryRef>)>,
        ) {
            if let DirectoryRelocation::Moved { id, logical_parent } = directory.relocation {
                output.push((id, logical_parent, directory.entries.clone()));
            }
            for child in &directory.dirs {
                collect_moved(child, output);
            }
        }
        let mut moved = Vec::new();
        collect_moved(self.written_files.get(&root_id), &mut moved);
        let directory_end = self
            .data
            .stream_position()
            .await
            .map_err(io::Error::erase)?;
        for (_id, logical_parent, entries) in moved {
            for (ty, directory) in entries {
                if !ty.supports_rrip() {
                    continue;
                }
                let parent = relocation_refs
                    .get(&(logical_parent, ty))
                    .copied()
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "logical parent extent was not written",
                        )
                    })?;
                self.patch_parent_link(directory, parent).await?;
            }
        }
        self.data
            .seek(SeekFrom::Start(directory_end))
            .await
            .map_err(io::Error::erase)?;
        let roots = self.written_files.root_refs().clone();

        let pos = self
            .data
            .stream_position()
            .await
            .map_err(io::Error::erase)?;
        for root in roots.values() {
            self.update_directory(*root, *root).await?;
        }
        // We need to seek back to this position
        self.data
            .seek(SeekFrom::Start(pos))
            .await
            .map_err(io::Error::erase)?;

        Ok(roots)
    }

    async fn write_path_tables(&mut self) -> io::Result<()> {
        for i in 0..self.entry_types.len() {
            let ty = self.entry_types[i];
            let l_ref = self.write_path_table(ty, EndianType::LittleEndian).await?;
            let m_ref = self.write_path_table(ty, EndianType::BigEndian).await?;
            assert_eq!(l_ref.size, m_ref.size);
            self.path_tables.insert(
                ty,
                PathTableRef {
                    lpt: l_ref.extent,
                    mpt: m_ref.extent,
                    size: l_ref.size as u64,
                },
            );
        }
        Ok(())
    }

    async fn write_path_table(&mut self, ty: EntryType, endian: EndianType) -> io::Result<DirectoryRef> {
        let start = self.data.pad_align_sector().await?;
        PathTableWriter {
            written_files: &self.written_files,
            ty,
            endian,
        }
        .write(&mut self.data).await?;
        let size = self
            .data
            .stream_position()
            .await
            .map_err(io::Error::erase)? as usize
            - (start.0 * self.data.sector_size);
        let _end = self.data.pad_align_sector().await?;
        Ok(DirectoryRef {
            extent: start,
            size,
        })
    }

    /// Writes the partition tables (MBR, GPT, or Hybrid) based on configuration.
    async fn write_partition_tables(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        // Calculate disk size in 512-byte sectors (for MBR/GPT compatibility)
        let disk_size_512 = (end_sector.0 * self.data.sector_size / 512) as u64;

        match self
            .ops
            .features
            .hybrid_boot
            .as_ref()
            .map(|h| h.partition_scheme)
        {
            None | Some(PartitionScheme::None) => {
                // No partition table requested - leave the system area empty.
                // Writing an MBR here would cause the kernel to detect a
                // partition table and prevent the ISO from being mounted.
            }
            Some(PartitionScheme::Mbr) => {
                self.write_mbr_boot(end_sector).await?;
            }
            Some(PartitionScheme::Gpt) => {
                self.write_gpt_boot(end_sector, disk_size_512).await?;
            }
            Some(PartitionScheme::Hybrid) => {
                self.write_hybrid_boot(end_sector, disk_size_512).await?;
            }
        }

        Ok(())
    }

    /// Writes a legacy MBR with a protective partition (current behavior).
    #[allow(dead_code)]
    async fn write_legacy_mbr(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        let start_sector = LogicalSector(16);
        let start_block = (start_sector.0 * (self.data.sector_size / 512)) as u32;
        let end_block = (end_sector.0 * (self.data.sector_size / 512)) as u32;

        let mut mbr = MasterBootRecord::default();
        mbr.with_partition_table(|pt| {
            pt[0] = MbrPartition {
                boot_indicator: 0x80,
                start_chs: Chs::new(start_block),
                part_type: MbrPartitionType::Iso9660.to_u8(),
                end_chs: Chs::new(end_block),
                start_lba: Le::<u32>::from_ne(start_block),
                sector_count: Le::<u32>::from_ne(end_block - start_block),
            };
        });

        // Inject bootstrap code if provided
        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot
            && let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap
        {
            let len = bootstrap.len().min(446);
            mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
        }

        self.data
            .seek(SeekFrom::Start(0))
            .await
            .map_err(io::Error::erase)?;
        self.data.write_all(bytemuck::bytes_of(&mbr)).await?;

        Ok(())
    }

    /// Writes an MBR partition table for BIOS USB boot (isohybrid-style).
    async fn write_mbr_boot(&mut self, end_sector: LogicalSector) -> io::Result<()> {
        let end_block = (end_sector.0 * (self.data.sector_size / 512)) as u32;

        let hybrid_opts = self.ops.features.hybrid_boot.as_ref();
        let bootable = hybrid_opts.map(|h| h.bootable).unwrap_or(true);

        let mut mbr = MasterBootRecord::default();
        mbr.with_partition_table(|pt| {
            // Create a partition covering the entire ISO
            // Type 0x17 is ISO9660/Hidden NTFS which is commonly used for hybrid ISOs
            pt[0] = MbrPartition {
                boot_indicator: if bootable { 0x80 } else { 0x00 },
                start_chs: Chs::new(0),
                part_type: MbrPartitionType::Iso9660.to_u8(),
                end_chs: Chs::new(end_block.saturating_sub(1)),
                start_lba: Le::<u32>::from_ne(0),
                sector_count: Le::<u32>::from_ne(end_block),
            };
        });

        // Inject bootstrap code if provided
        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot
            && let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap
        {
            let len = bootstrap.len().min(446);
            mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
        }

        self.data
            .seek(SeekFrom::Start(0))
            .await
            .map_err(io::Error::erase)?;
        self.data.write_all(bytemuck::bytes_of(&mbr)).await?;

        Ok(())
    }

    /// Writes a GPT partition table for UEFI boot.
    async fn write_gpt_boot(&mut self, _end_sector: LogicalSector, disk_size_512: u64) -> io::Result<()> {
        // For GPT, we need:
        // 1. Protective MBR at sector 0
        // 2. Primary GPT header at sector 1
        // 3. GPT partition entries at sectors 2-33 (128 entries * 128 bytes = 32 sectors)
        // 4. Backup GPT entries and header at end of disk

        // Write protective MBR
        let mbr = MasterBootRecord::protective(disk_size_512);
        self.data
            .seek(SeekFrom::Start(0))
            .await
            .map_err(io::Error::erase)?;
        self.data.write_all(bytemuck::bytes_of(&mbr)).await?;

        // Create GPT partition entry for the ISO data
        // Start after GPT structures (sector 34 in 512-byte sectors)
        let iso_start_lba = 34u64;
        let iso_end_lba = disk_size_512.saturating_sub(34); // Leave room for backup GPT

        // Create a deterministic partition GUID based on the volume name
        let partition_guid = Self::generate_guid_from_string(&self.ops.volume_name);
        let disk_guid =
            Self::generate_guid_from_string(&alloc::format!("disk-{}", self.ops.volume_name));

        let mut entries = [GptPartitionEntry::default(); 4];
        entries[0] = GptPartitionEntry::new(
            Guid::BASIC_DATA, // or could use a custom ISO GUID
            partition_guid,
            iso_start_lba,
            iso_end_lba,
        );

        // Calculate CRC32 of partition entries
        let entries_bytes = bytemuck::bytes_of(&entries);
        let entries_crc = Self::crc32(entries_bytes);

        // Create and write primary GPT header
        let header_bytes = Self::write_gpt_header_bytes(
            disk_guid,
            1,                 // my_lba (primary is at sector 1)
            disk_size_512 - 1, // alternate_lba (backup at last sector)
            iso_start_lba,     // first_usable_lba
            iso_end_lba,       // last_usable_lba
            2,                 // partition_entry_lba
            4,                 // num_partition_entries
            entries_crc,
        );

        // Write primary GPT header
        self.data
            .seek(SeekFrom::Start(512))
            .await
            .map_err(io::Error::erase)?; // Sector 1
        self.data.write_all(&header_bytes).await?;

        // Write partition entries (starting at sector 2)
        self.data
            .seek(SeekFrom::Start(1024))
            .await
            .map_err(io::Error::erase)?; // Sector 2
        self.data.write_all(entries_bytes).await?;

        // Note: In a full implementation, we'd also write the backup GPT at the end
        // For now, we skip this as ISOs are typically read-only

        Ok(())
    }

    /// Writes a Hybrid MBR + GPT for dual BIOS/UEFI boot.
    async fn write_hybrid_boot(
        &mut self,
        _end_sector: LogicalSector,
        disk_size_512: u64,
    ) -> io::Result<()> {
        let hybrid_opts = self.ops.features.hybrid_boot.as_ref();
        let bootable = hybrid_opts.map(|h| h.bootable).unwrap_or(true);

        // Create GPT partition entry for the ISO
        let iso_start_lba = 34u64;
        let iso_end_lba = disk_size_512.saturating_sub(34);

        // Create deterministic GUIDs
        let partition_guid = Self::generate_guid_from_string(&self.ops.volume_name);
        let disk_guid =
            Self::generate_guid_from_string(&alloc::format!("disk-{}", self.ops.volume_name));

        let gpt_entries = [
            GptPartitionEntry::new(Guid::BASIC_DATA, partition_guid, iso_start_lba, iso_end_lba),
            GptPartitionEntry::default(),
        ];

        // Build hybrid MBR using hadris-part
        let mut mbr = HybridMbrBuilder::new(disk_size_512)
            .protective_slot(0)
            .mirror_partition(0, MbrPartitionType::Iso9660, bootable)
            .build(&gpt_entries)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid hybrid MBR"))?;

        // Inject bootstrap code if provided
        if let Some(ref hybrid_opts) = self.ops.features.hybrid_boot
            && let Some(ref bootstrap) = hybrid_opts.mbr_bootstrap
        {
            let len = bootstrap.len().min(446);
            mbr.bootstrap[..len].copy_from_slice(&bootstrap[..len]);
        }

        // Write hybrid MBR
        self.data
            .seek(SeekFrom::Start(0))
            .await
            .map_err(io::Error::erase)?;
        self.data.write_all(bytemuck::bytes_of(&mbr)).await?;

        // Calculate CRC32 of partition entries
        let entries_bytes = bytemuck::bytes_of(&gpt_entries);
        let entries_crc = Self::crc32(entries_bytes);

        // Create and write primary GPT header
        let header_bytes = Self::write_gpt_header_bytes(
            disk_guid,
            1,
            disk_size_512 - 1,
            iso_start_lba,
            iso_end_lba,
            2,
            2, // Only 2 entries in our case
            entries_crc,
        );

        // Write primary GPT header
        self.data
            .seek(SeekFrom::Start(512))
            .await
            .map_err(io::Error::erase)?;
        self.data.write_all(&header_bytes).await?;

        // Write partition entries
        self.data
            .seek(SeekFrom::Start(1024))
            .await
            .map_err(io::Error::erase)?;
        self.data.write_all(entries_bytes).await?;

        Ok(())
    }

    /// Simple CRC32 calculation for GPT.
    fn crc32(data: &[u8]) -> u32 {
        // Using the standard CRC-32 polynomial
        let mut crc = !0u32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB88320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// Generates a deterministic GUID from a string (simple hash-based).
    fn generate_guid_from_string(s: &str) -> Guid {
        // Simple FNV-1a hash to generate a deterministic GUID
        let mut hash1: u64 = 0xcbf29ce484222325;
        let mut hash2: u64 = 0x100000001b3;

        for byte in s.bytes() {
            hash1 ^= byte as u64;
            hash1 = hash1.wrapping_mul(0x100000001b3);
            hash2 ^= byte as u64;
            hash2 = hash2.wrapping_mul(0xcbf29ce484222325);
        }

        let mut bytes = [0u8; 16];
        bytes[0..8].copy_from_slice(&hash1.to_le_bytes());
        bytes[8..16].copy_from_slice(&hash2.to_le_bytes());

        // Set version 4 (random) and variant bits
        bytes[6] = (bytes[6] & 0x0f) | 0x40; // Version 4
        bytes[8] = (bytes[8] & 0x3f) | 0x80; // Variant 1

        Guid::from_bytes(bytes)
    }

    /// Writes a GPT header to the given buffer (92 bytes).
    #[allow(clippy::too_many_arguments)]
    fn write_gpt_header_bytes(
        disk_guid: Guid,
        my_lba: u64,
        alternate_lba: u64,
        first_usable_lba: u64,
        last_usable_lba: u64,
        partition_entry_lba: u64,
        num_partition_entries: u32,
        partition_entry_array_crc32: u32,
    ) -> [u8; 92] {
        let mut buf = [0u8; 92];

        // Signature: "EFI PART"
        buf[0..8].copy_from_slice(b"EFI PART");
        // Revision: 1.0
        buf[8..12].copy_from_slice(&0x00010000u32.to_le_bytes());
        // Header size: 92
        buf[12..16].copy_from_slice(&92u32.to_le_bytes());
        // Header CRC32: placeholder, will be calculated
        buf[16..20].copy_from_slice(&0u32.to_le_bytes());
        // Reserved
        buf[20..24].copy_from_slice(&0u32.to_le_bytes());
        // My LBA
        buf[24..32].copy_from_slice(&my_lba.to_le_bytes());
        // Alternate LBA
        buf[32..40].copy_from_slice(&alternate_lba.to_le_bytes());
        // First usable LBA
        buf[40..48].copy_from_slice(&first_usable_lba.to_le_bytes());
        // Last usable LBA
        buf[48..56].copy_from_slice(&last_usable_lba.to_le_bytes());
        // Disk GUID
        buf[56..72].copy_from_slice(&disk_guid.to_bytes());
        // Partition entry LBA
        buf[72..80].copy_from_slice(&partition_entry_lba.to_le_bytes());
        // Number of partition entries
        buf[80..84].copy_from_slice(&num_partition_entries.to_le_bytes());
        // Size of partition entry: 128
        buf[84..88].copy_from_slice(&128u32.to_le_bytes());
        // Partition entry array CRC32
        buf[88..92].copy_from_slice(&partition_entry_array_crc32.to_le_bytes());

        // Calculate and set header CRC32
        let crc = Self::crc32(&buf);
        buf[16..20].copy_from_slice(&crc.to_le_bytes());

        buf
    }

    async fn update_directory(
        &mut self,
        parent: DirectoryRef,
        directory: DirectoryRef,
    ) -> io::Result<()> {
        let start = self.data.seek_sector(directory.extent).await?;
        let mut offset = 0;
        loop {
            if offset >= directory.size as u64 {
                break;
            }
            self.data
                .seek(SeekFrom::Start(start + offset))
                .await
                .map_err(io::Error::erase)?;
            let mut record = DirectoryRecord::parse(&mut self.data).await?;
            if record.header().len == 0 {
                break;
            }

            if record.name() == b"\x00" || record.name() == b"\x01" {
                let dir_ref = [directory, parent][record.name()[0] as usize];
                let header = record.header_mut();
                header.extent.write(dir_ref.extent.0 as u32);
                header.data_len.write(dir_ref.size as u32);
                self.data
                    .seek(SeekFrom::Start(start + offset))
                    .await
                    .map_err(io::Error::erase)?;
                record.write(&mut self.data).await?;
                offset += record.header().len as u64;
                continue;
            }
            offset += record.header().len as u64;

            if FileFlags::from_bits_truncate(record.header().flags).contains(FileFlags::DIRECTORY) {
                let record = DirectoryRef {
                    extent: LogicalSector(record.header().extent.read() as usize),
                    size: record.header().data_len.read() as usize,
                };
                self.update_directory(directory, record).await?;
            }
        }

        Ok(())
    }

    async fn patch_parent_link(
        &mut self,
        directory: DirectoryRef,
        parent: DirectoryRef,
    ) -> io::Result<()> {
        let start = self.data.seek_sector(directory.extent).await?;
        self.data
            .seek(SeekFrom::Start(start))
            .await
            .map_err(io::Error::erase)?;
        let dot = DirectoryRecord::parse(&mut self.data).await?;
        self.data
            .seek(SeekFrom::Start(start + dot.header().len as u64))
            .await
            .map_err(io::Error::erase)?;
        let mut dotdot = DirectoryRecord::parse(&mut self.data).await?;
        let system_use = dotdot.system_use_mut();
        let mut offset = 0;
        while offset + 4 <= system_use.len() {
            let length = system_use[offset + 2] as usize;
            if length < 4 || offset + length > system_use.len() {
                break;
            }
            if &system_use[offset..offset + 2] == b"PL" && length >= 12 {
                let value = crate::types::U32LsbMsb::new(parent.extent.0 as u32);
                system_use[offset + 4..offset + 12]
                    .copy_from_slice(bytemuck::bytes_of(&value));
                self.data
                    .seek(SeekFrom::Start(start + dot.header().len as u64))
                    .await
                    .map_err(io::Error::erase)?;
                dotdot.write(&mut self.data).await?;
                return Ok(());
            }
            offset += length;
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "relocated directory is missing its RRIP PL entry",
        ))
    }

    /// Write a directory using a three-phase approach:
    ///
    /// 1. Build all RRIP entries and split each against available inline space
    /// 2. If any have overflow: write a shared continuation area, patch CE entries
    /// 3. Write directory records with the inline SU bytes
    #[allow(clippy::too_many_arguments)]
    async fn write_directory(
        data: &mut IsoCursor<DATA>,
        ty: EntryType,
        dir: &mut WrittenDirectory,
        is_root: bool,
        inode_counter: &mut u32,
        rrip_options: Option<&RripOptions>,
        fallback_time: &[u8; 7],
        relocation_refs: &BTreeMap<(usize, EntryType), DirectoryRef>,
    ) -> io::Result<()> {
        let rrip_options = rrip_options.filter(|options| options.enabled);
        let has_rrip = ty.supports_rrip() && rrip_options.is_some();
        let options = rrip_options.copied().unwrap_or_else(RripOptions::disabled);
        let directory_nlink = 2 + dir.dirs.len() as u32;

        // ── Phase 1: Build all pending records ──

        let mut records: Vec<PendingRecord> = Vec::new();

        // Dot entry (".")
        let dot_split = if has_rrip {
            let kind = if is_root {
                RripEntryKind::RootDot {
                    metadata: dir.metadata,
                    nlink: directory_nlink,
                }
            } else {
                RripEntryKind::Dot {
                    metadata: dir.metadata,
                    nlink: directory_nlink,
                }
            };
            let max = available_su_space(1); // name is b"\x00"
            build_rrip_entries(kind, 0, &options, fallback_time).build_split(max)
        } else {
            SplitSu::empty()
        };
        records.push(PendingRecord {
            name: vec![0x00],
            split: dot_split,
            dir_ref: DirectoryRef::default(),
            flags: FileFlags::DIRECTORY,
        });

        // Dotdot entry ("..")
        let dotdot_split = if has_rrip {
            let kind = if is_root {
                RripEntryKind::RootDotDot {
                    metadata: dir.metadata,
                    nlink: directory_nlink,
                }
            } else {
                RripEntryKind::DotDot {
                    metadata: dir.metadata,
                    nlink: directory_nlink,
                }
            };
            let max = available_su_space(1); // name is b"\x01"
            let mut builder = build_rrip_entries(kind, 0, &options, fallback_time);
            if let DirectoryRelocation::Moved { logical_parent, .. } = dir.relocation {
                let parent = relocation_refs
                    .get(&(logical_parent, ty))
                    .copied()
                    .unwrap_or_default();
                builder.add_pl(parent.extent.0 as u32);
            }
            builder.build_split(max)
        } else {
            SplitSu::empty()
        };
        records.push(PendingRecord {
            name: vec![0x01],
            split: dotdot_split,
            dir_ref: DirectoryRef::default(),
            flags: FileFlags::DIRECTORY,
        });

        // Directory entries
        for directory in &dir.dirs {
            let WrittenDirectory {
                name,
                rrip_name,
                entries,
                metadata,
                dirs,
                relocation,
                ..
            } = directory;
            let converted_name = ty.convert_name(name);
            let split = if has_rrip {
                let inode = *inode_counter;
                *inode_counter += 1;
                let max = available_su_space(converted_name.as_bytes().len());
                let mut builder = build_rrip_entries(
                    RripEntryKind::Directory {
                        original_name: rrip_name,
                        metadata: *metadata,
                        nlink: 2 + dirs.len() as u32,
                    },
                    inode,
                    &options,
                    fallback_time,
                );
                match relocation {
                    DirectoryRelocation::Placeholder { target } => {
                        let target = relocation_refs.get(&(*target, ty)).ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "relocated directory extent was not written",
                            )
                        })?;
                        builder.add_cl(target.extent.0 as u32);
                    }
                    DirectoryRelocation::Moved { .. } => {
                        builder.add_re();
                    }
                    DirectoryRelocation::None => {}
                }
                builder.build_split(max)
            } else {
                SplitSu::empty()
            };
            records.push(PendingRecord {
                name: converted_name.as_bytes().to_vec(),
                split,
                dir_ref: match relocation {
                    DirectoryRelocation::Placeholder { target } => relocation_refs
                        .get(&(*target, ty))
                        .copied()
                        .unwrap_or_default(),
                    _ => *entries.get(&ty).unwrap(),
                },
                flags: FileFlags::DIRECTORY,
            });
        }

        // File entries
        for file in &dir.files {
            let WrittenFile {
                name,
                entry,
                kind,
                metadata,
            } = file;
            let converted_name = ty.convert_name(name);
            let split = if has_rrip {
                let inode = *inode_counter;
                *inode_counter += 1;
                let max = available_su_space(converted_name.as_bytes().len());
                build_rrip_entries(
                    RripEntryKind::Entry {
                        original_name: name,
                        metadata: *metadata,
                        kind,
                    },
                    inode,
                    &options,
                    fallback_time,
                )
                .build_split(max)
            } else {
                SplitSu::empty()
            };
            records.push(PendingRecord {
                name: converted_name.as_bytes().to_vec(),
                split,
                dir_ref: *entry,
                flags: FileFlags::empty(),
            });
        }

        // ── Phase 1.5: Deduplicate names ──
        // Name mangling can map different original names to the same ISO name.
        // e.g., "readme.txt" and "README.txt" both become "README.TXT;1".
        // Resolve collisions with underscore-based suffixes.
        {
            use std::collections::HashMap;
            let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
            for record in &mut records {
                // Skip dot/dotdot entries
                if record.name.len() == 1 && (record.name[0] == 0x00 || record.name[0] == 0x01) {
                    continue;
                }
                let count = seen.entry(record.name.clone()).or_insert(0);
                *count += 1;
                if *count > 1 {
                    record.name = apply_dedup_suffix(&record.name, *count - 1, ty);
                }
            }
        }

        // ── Phase 2: Write continuation area if any records have overflow ──

        let has_overflow = records.iter().any(|r| r.split.has_overflow());
        if has_overflow {
            let ca_sector = data.pad_align_sector().await?;
            let mut offset = 0u32;
            for record in &mut records {
                if record.split.has_overflow() {
                    record.split.patch_ce(ca_sector.0 as u32, offset);
                    data.write_all(&record.split.overflow).await?;
                    offset += record.split.overflow.len() as u32;
                }
            }
        }

        // ── Phase 3: Write directory records with inline SU bytes ──

        let start = data.pad_align_sector().await?;
        for record in &records {
            DirectoryRecord::new(
                &record.name,
                &record.split.inline,
                record.dir_ref,
                record.flags,
            )
            .write(&mut *data).await?;
        }
        let end = data.pad_align_sector().await?;
        let size = (end.0 - start.0) * data.sector_size;

        dir.entries.insert(
            ty,
            DirectoryRef {
                extent: start,
                size,
            },
        );
        Ok(())
    }
}
} // io_transform!

#[allow(dead_code)]
struct FileTreeWalker<'a> {
    input_files: &'a InputTree,
    stack: VecDeque<StackFrame<'a>>,
}

enum StackFrame<'a> {
    Node(&'a InputEntry),
    DirExit(&'a InputEntry),
}

#[derive(Debug, PartialEq, Eq)]
enum TreeWalkerItem<'a> {
    EnterDirectory(&'a InputEntry),
    File(&'a InputEntry),
    ExitDirectory(&'a InputEntry),
}

impl<'a> FileTreeWalker<'a> {
    pub fn new(input: &'a InputTree) -> Self {
        let mut stack = VecDeque::new();
        for file in input.entries.iter().rev() {
            stack.push_back(StackFrame::Node(file));
        }
        FileTreeWalker {
            input_files: input,
            stack,
        }
    }
}

impl<'a> Iterator for FileTreeWalker<'a> {
    type Item = TreeWalkerItem<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let frame = self.stack.pop_back()?;
        match frame {
            StackFrame::Node(file) => match &file.kind {
                InputEntryKind::Directory(children) => {
                    // Yield that we are entering this directory (pre-order event)
                    let current_dir = file;

                    // Push an Exit frame to signal leaving this directory later
                    self.stack.push_back(StackFrame::DirExit(current_dir));

                    // Push children in reverse order for DFS
                    for child in children.iter().rev() {
                        self.stack.push_back(StackFrame::Node(child));
                    }

                    Some(TreeWalkerItem::EnterDirectory(current_dir))
                }
                _ => Some(TreeWalkerItem::File(file)),
            },
            StackFrame::DirExit(dir) => Some(TreeWalkerItem::ExitDirectory(dir)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*; // Import items from the outer module
    use alloc::vec;

    #[test]
    fn test_depth_first_tree_walk_iterator() {
        // Define a test file hierarchy
        let file_a = InputEntry::file("root/dir1/fileA.txt", Vec::new());
        let file_b = InputEntry::file("root/dir1/fileB.txt", Vec::new());
        let file_c = InputEntry::file("root/fileC.txt", Vec::new());
        let file_d = InputEntry::file("root/dir2/fileD.txt", Vec::new());
        let file_e = InputEntry::file("root/dir2/subdir/fileE.txt", Vec::new());

        let subdir_node = InputEntry::directory("root/dir2/subdir", vec![file_e.clone()]);

        let dir1_node = InputEntry::directory("root/dir1", vec![file_a.clone(), file_b.clone()]);

        let dir2_node = InputEntry::directory(
            "root/dir2",
            vec![
                file_d.clone(),
                subdir_node.clone(), // Subdirectory
            ],
        );

        let root_level_files = vec![dir1_node.clone(), file_c.clone(), dir2_node.clone()];

        let input_tree = InputTree::new(PathSeparator::ForwardSlash, root_level_files);

        // Create the iterator
        let walker = FileTreeWalker::new(&input_tree);

        // Define the expected sequence of events (depth-first, pre-order for Enter, post-order for Exit)
        let expected_sequence = vec![
            TreeWalkerItem::EnterDirectory(&dir1_node),   // Enter dir1
            TreeWalkerItem::File(&file_a),                // Process fileA
            TreeWalkerItem::File(&file_b),                // Process fileB
            TreeWalkerItem::ExitDirectory(&dir1_node),    // Exit dir1
            TreeWalkerItem::File(&file_c),                // Process fileC
            TreeWalkerItem::EnterDirectory(&dir2_node),   // Enter dir2
            TreeWalkerItem::File(&file_d),                // Process fileD
            TreeWalkerItem::EnterDirectory(&subdir_node), // Enter subdir
            TreeWalkerItem::File(&file_e),                // Process fileE
            TreeWalkerItem::ExitDirectory(&subdir_node),  // Exit subdir
            TreeWalkerItem::ExitDirectory(&dir2_node),    // Exit dir2
        ];

        // Collect all items from the iterator
        let actual_sequence: Vec<TreeWalkerItem> = walker.collect();

        // Assert that the actual sequence matches the expected sequence
        assert_eq!(actual_sequence, expected_sequence);
    }

    #[test]
    fn test_dedup_suffix_l1_with_ext() {
        let ty = EntryType::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"README.TXT;1", 1, ty);
        assert_eq!(result, b"README_1.TXT;1");
    }

    #[test]
    fn test_dedup_suffix_l1_no_ext() {
        let ty = EntryType::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"FILENAME;1", 1, ty);
        assert_eq!(result, b"FILENA_1;1");
    }

    #[test]
    fn test_dedup_suffix_l2() {
        let ty = EntryType::Level2 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"LONGFILENAME.EXT;1", 2, ty);
        assert_eq!(result, b"LONGFILENAME_2.EXT;1");
    }

    #[test]
    fn test_dedup_suffix_l3_no_version() {
        let ty = EntryType::Level3 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let result = apply_dedup_suffix(b"README.TXT", 1, ty);
        assert_eq!(result, b"README_1.TXT");
    }

    #[test]
    fn test_dedup_suffix_distinct() {
        let ty = EntryType::Level1 {
            supports_lowercase: false,
            supports_rrip: false,
        };
        let r1 = apply_dedup_suffix(b"README.TXT;1", 1, ty);
        let r2 = apply_dedup_suffix(b"README.TXT;1", 2, ty);
        let r3 = apply_dedup_suffix(b"README.TXT;1", 3, ty);
        assert_ne!(r1, r2);
        assert_ne!(r2, r3);
        assert_ne!(r1, r3);
    }
}
