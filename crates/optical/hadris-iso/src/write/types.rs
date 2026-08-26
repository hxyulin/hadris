use core::fmt;

use alloc::{collections::{BTreeMap, BTreeSet,VecDeque}, string::String, sync::Arc, vec::Vec};
use alloc::vec;

use crate::{directory::{DirectoryRef, FileFlags}, file::EntryType, io::{self}, read::PathSeparator, rrip::RripOptions, susp::SplitSu, write::{utils::*, writer::*}};

/// Canonical error for ISO creation operations.
pub type Error = IsoCreationError;
/// Canonical result for ISO creation operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Represents a file in the write order: (directory ID, file index).
pub type FileOrder = (DirectoryId, usize);

/// Maps (directory ID, entry type) to directory reference for relocation.
pub type RelocationMap = BTreeMap<(usize, EntryType), DirectoryRef>;

/// RRIP timestamp (7 bytes: year, month, day, hour, minute, second, offset).
pub type RripTime = [u8; 7];

#[derive(Debug, thiserror::Error)]
/// Identifies a IsoCreationError value.
pub enum IsoCreationError {
    #[error(transparent)]
    /// The `Io` variant.
    Io(#[from] crate::io::Error),
}

#[derive(Debug, thiserror::Error)]
/// Identifies a FileConversionError value.
pub enum FileConversionError {
    #[error("I/O error: {0}")]
    /// The `Io` variant.
    Io(#[from] std::io::Error),
    #[error("Path {0:?} is not a valid UTF-8 string")]
    /// The `InvalidUtf8Path` variant.
    InvalidUtf8Path(std::path::PathBuf),
    #[error("Unsupported filesystem entry type at {0:?}")]
    /// The `UnsupportedFileType` variant.
    UnsupportedFileType(std::path::PathBuf),
}

/// A compact input tree for callers that do not need per-entry metadata.
///
/// [`InputTree`] is the richer model for Rock Ridge metadata and host
/// filesystem imports. Both models are accepted by [`IsoImageWriter::create`].
pub struct InputFiles {
    /// Separator used by paths referenced from writer options.
    pub path_separator: PathSeparator,
    /// Root-level files and directories.
    pub files: Vec<File>,
}

#[derive(Clone, PartialEq, Eq)]
/// A file or directory in the compact [`InputFiles`] model.
pub enum File {
    /// The `File` variant.
    File {
        /// The `name` field.
        name: Arc<String>,
        /// The `contents` field.
        contents: Vec<u8>,
    },
    /// The `Directory` variant.
    Directory {
        /// The `name` field.
        name: Arc<String>,
        /// The `children` field.
        children: Vec<File>,
    },
}

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

impl File {
    /// Performs the `name` operation.
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
    /// The `path_separator` field.
    pub path_separator: PathSeparator,
    /// The `entries` field.
    pub entries: Vec<InputEntry>,
}

impl InputTree {

    /// Recursive validation.
    ///
    /// Checks:
    /// - Depth: max 8 directories (unless RRIP deep dirs enabled)
    /// - Path length: max 255 bytes (unless RRIP deep dirs enabled)
    /// - Symlinks: require RRIP preserve_symlinks
    /// - Devices: require RRIP preserve_devices
    /// - File size: max 4 GiB
    pub fn validate(&self, rrip: Option<&RripOptions>) -> crate::io::Result<()> {
        Self::visit(&self.entries, rrip, 1, 0)
    }

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

                    Self::visit(children, rrip, depth + 1, child_path_len)?;
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
                InputEntryKind::File(_contents) => {
                    // if contents.len() as u64 > Self::MAX_SINGLE_EXTENT_FILE_LEN {
                    //     return Err(io::Error::new(
                    //         io::ErrorKind::InvalidInput,
                    //         "file exceeds 4 GiB; the ISO writer stores each file in a single \
                    //         extent and cannot yet emit multi-extent records",
                    //     ));
                    // }
                }
            }
        }
        Ok(())
    }
}

/// Optional POSIX metadata for an input entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputMetadata {
    /// The `mode` field.
    pub mode: Option<u32>,
    /// The `uid` field.
    pub uid: Option<u32>,
    /// The `gid` field.
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
    /// The `File` variant.
    File(Vec<u8>),
    /// The `Directory` variant.
    Directory(Vec<InputEntry>),
    /// The `Symlink` variant.
    Symlink(String),
    /// A POSIX character device.
    CharacterDevice {
        /// Device-class identifier.
        major: u32,
        /// Device identifier within the class.
        minor: u32,
    },
    /// A POSIX block device.
    BlockDevice {
        /// Device-class identifier.
        major: u32,
        /// Device identifier within the class.
        minor: u32,
    },
}

/// A named input entry and its optional host metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEntry {
    /// The `name` field.
    pub name: Arc<String>,
    /// The `kind` field.
    pub kind: InputEntryKind,
    /// The `metadata` field.
    pub metadata: InputMetadata,
}

impl InputEntry {
    /// Performs the `file` operation.
    pub fn file(name: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        Self::new(name, InputEntryKind::File(contents.into()))
    }

    /// Performs the `directory` operation.
    pub fn directory(name: impl Into<String>, children: Vec<Self>) -> Self {
        Self::new(name, InputEntryKind::Directory(children))
    }

    /// Performs the `symlink` operation.
    pub fn symlink(name: impl Into<String>, target: impl Into<String>) -> Self {
        Self::new(name, InputEntryKind::Symlink(target.into()))
    }

    /// Performs the `character_device` operation.
    pub fn character_device(name: impl Into<String>, major: u32, minor: u32) -> Self {
        Self::new(name, InputEntryKind::CharacterDevice { major, minor })
    }

    /// Performs the `block_device` operation.
    pub fn block_device(name: impl Into<String>, major: u32, minor: u32) -> Self {
        Self::new(name, InputEntryKind::BlockDevice { major, minor })
    }

    /// Performs the `with_metadata` operation.
    pub fn with_metadata(mut self, metadata: InputMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Performs the `name` operation.
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
    /// Performs the `new` operation.
    pub fn new(path_separator: PathSeparator, entries: Vec<InputEntry>) -> Self {
        Self {
            path_separator,
            entries,
        }
    }

    /// Performs the `from_fs` operation.
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

/// Depth-first tree walker.
pub struct FileTreeWalker<'a>(VecDeque<StackFrame<'a>>);

/// Walker internal state.
enum StackFrame<'a> {
    Node(&'a InputEntry),
    DirExit(&'a InputEntry),
}

/// Walker events.
#[derive(Debug, PartialEq, Eq)]
pub enum TreeWalkerItem<'a> {
    /// Entering a directory.
    EnterDirectory(&'a InputEntry),
    /// A file.
    File(&'a InputEntry),
    /// Exiting a directory.
    ExitDirectory(&'a InputEntry),
}

impl<'a> FileTreeWalker<'a> {
    /// New walker.
    pub fn new(input: &'a InputTree) -> Self {
        let mut stack = VecDeque::new();
        for file in input.entries.iter().rev() {
            stack.push_back(StackFrame::Node(file));
        }
        FileTreeWalker(stack)
    }

    /// Walk the tree and build WrittenFiles.
    pub fn walk(self, written_files: &mut WrittenFiles) {
        let mut next_directory_id = 1usize;
        let mut current_dir = written_files.root_dir();

        for file in self {
            match file {
                TreeWalkerItem::EnterDirectory(dir) => {
                    let name = dir.name();
                    let metadata = dir.metadata;
                    let written_dir = written_files.get_mut(&current_dir);
                    let index = written_dir.push_dir(name, metadata);
                    written_dir.dirs[index].id = next_directory_id;
                    next_directory_id += 1;
                    current_dir.push(index);
                }
                TreeWalkerItem::ExitDirectory(_dir) => {
                    current_dir.pop();
                }
                TreeWalkerItem::File(file) => {
                    // Extents are assigned in the planning pass below.
                    // Empty files keep extent 0 (per ISO 9660 they have no
                    // data to reference).
                    let dir = written_files.get_mut(&current_dir);
                    dir.push_file(WrittenFile::new(
                        file.name.clone(),
                        file.kind.clone(),
                        file.metadata,
                    ));
                }
            };
        }
    }
}

impl<'a> Iterator for FileTreeWalker<'a> {
    type Item = TreeWalkerItem<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let frame = self.0.pop_back()?;
        match frame {
            StackFrame::Node(file) => match &file.kind {
                InputEntryKind::Directory(children) => {
                    // Yield that we are entering this directory (pre-order event)
                    let current_dir = file;

                    // Push an Exit frame to signal leaving this directory later
                    self.0.push_back(StackFrame::DirExit(current_dir));

                    // Push children in reverse order for DFS
                    for child in children.iter().rev() {
                        self.0.push_back(StackFrame::Node(child));
                    }

                    Some(TreeWalkerItem::EnterDirectory(current_dir))
                }
                _ => Some(TreeWalkerItem::File(file)),
            },
            StackFrame::DirExit(dir) => Some(TreeWalkerItem::ExitDirectory(dir)),
        }
    }
}

/// Selects which RRIP fields to emit for a directory entry.
pub enum RripEntryKind<'a> {
    /// Root "." entry: SP + ER + PX + NM(CURRENT)
    RootDot {
        /// File metadata.
        metadata: InputMetadata,
        /// Number of hard links.
        nlink: u32,
    },
    /// Root ".." entry: PX + NM(PARENT)
    RootDotDot {
        /// File metadata.
        metadata: InputMetadata,
        /// Number of hard links.
        nlink: u32,
    },
    /// Non-root "." entry: PX + NM(CURRENT)
    Dot {
        /// File metadata.
        metadata: InputMetadata,
        /// Number of hard links.
        nlink: u32,
    },
    /// Non-root ".." entry: PX + NM(PARENT)
    DotDot {
        /// File metadata.
        metadata: InputMetadata,
        /// Number of hard links.
        nlink: u32,
    },
    /// Named directory entry.
    Directory {
        /// Original filename.
        original_name: &'a str,
        /// File metadata.
        metadata: InputMetadata,
        /// Number of hard links.
        nlink: u32,
    },
    /// Named file entry.
    Entry {
        /// Original filename.
        original_name: &'a str,
        /// File metadata.
        metadata: InputMetadata,
        /// File contents.
        kind: &'a InputEntryKind,
    },
}

/// Pending directory records ready for layout planning and writing.
///
/// # Overview
/// These records represent a single directory's contents, including:
/// - "." and ".." entries
/// - Child directories
/// - Files (including multi-extent files)
///
/// # RRIP System Use
/// Each record can have inline System Use data (RRIP extensions).
/// If the data exceeds the available inline space, overflow is tracked
/// and written to a separate continuation area.
///
/// # Multi-Extent Files
/// Files larger than 4 GiB are split into multiple extents.
/// Each extent gets its own directory record with the same name.
/// The first record contains the RRIP data; subsequent extents
/// have empty System Use.
///
/// # Processing Phases
/// 1. Build all records with RRIP entries split against inline space
/// 2. If any record has overflow: allocate continuation area, patch CE entries
/// 3. Write directory records with inline SU bytes
///
/// # Ordering
/// Records are ordered by File Identifier (ECMA-119 9.3):
/// - "." (0x00) first
/// - ".." (0x01) second
/// - All other entries sorted by name
pub struct PendingRecords(Vec<PendingRecord>);

impl PendingRecords {

    /// Build the pending records for one directory: dot/dotdot, child
    /// directories, and files, with RRIP system-use areas split against the
    /// inline budget, names deduplicated, and records ordered by File
    /// Identifier (ECMA-119 9.3). Sizes never depend on extent values, so
    /// the planning pass calls this with placeholder refs and the write pass
    /// rebuilds with the real ones.
    pub fn new(
        ty: EntryType,
        dir: &WrittenDirectory,
        is_root: bool,
        inode_counter: &mut u32,
        rrip_options: Option<&RripOptions>,
        fallback_time: &RripTime,
        relocation_refs: &BTreeMap<(usize, EntryType), DirectoryRef>,
    ) -> io::Result<Self> {
        let rrip_options = rrip_options.filter(|options| options.enabled);
        let has_rrip = ty.supports_rrip() && rrip_options.is_some();
        let options = rrip_options.copied().unwrap_or_else(RripOptions::disabled);
        let directory_nlink = 2 + dir.dirs.len() as u32;

        let mut records: Vec<PendingRecord> = Vec::new();

        records.push(Self::build_dot_record(has_rrip, is_root, &dir.metadata, directory_nlink, &options, fallback_time));
        records.push(Self::build_dotdot_record(has_rrip, is_root, dir, ty, &options, fallback_time, relocation_refs)?);

        Self::build_directory_records(
            &mut records,
            &dir.dirs,
            ty,
            has_rrip,
            &options,
            inode_counter,
            fallback_time,
            relocation_refs,
        )?;

        Self::build_file_records(
            &mut records,
            &dir.files,
            ty,
            has_rrip,
            &options,
            inode_counter,
            fallback_time,
        )?;

        Self::deduplicate_names(&mut records, ty);
        Self::sort_records(&mut records);

        Ok(Self(records))
    }

    fn build_dot_record(
        has_rrip: bool,
        is_root: bool,
        metadata: &InputMetadata,
        nlink: u32,
        options: &RripOptions,
        fallback_time: &RripTime,
    ) -> PendingRecord {
        let split = if has_rrip {
            let kind = if is_root {
                RripEntryKind::RootDot { metadata: *metadata, nlink }
            } else {
                RripEntryKind::Dot { metadata: *metadata, nlink }
            };
            let max = available_su_space(1); // name is b"\x00"
            build_rrip_entries(kind, 0, options, fallback_time).build_split(max)
        } else {
            SplitSu::empty()
        };
        PendingRecord::current_dir(split)
    }

    fn build_dotdot_record(
        has_rrip: bool,
        is_root: bool,
        dir: &WrittenDirectory,
        ty: EntryType,
        options: &RripOptions,
        fallback_time: &RripTime,
        relocation_refs: &BTreeMap<(usize, EntryType), DirectoryRef>,
    ) -> io::Result<PendingRecord> {
        let split = if has_rrip {
            let kind = if is_root {
                RripEntryKind::RootDotDot {
                    metadata: dir.metadata,
                    nlink: 2 + dir.dirs.len() as u32,
                }
            } else {
                RripEntryKind::DotDot {
                    metadata: dir.metadata,
                    nlink: 2 + dir.dirs.len() as u32,
                }
            };
            let max = available_su_space(1); // name is b"\x01"
            let mut builder = build_rrip_entries(kind, 0, options, fallback_time);
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
        Ok(PendingRecord::parent_dir(split))
    }

    fn build_directory_records(
        records: &mut Vec<PendingRecord>,
        dirs: &[WrittenDirectory],
        ty: EntryType,
        has_rrip: bool,
        options: &RripOptions,
        inode_counter: &mut u32,
        fallback_time: &RripTime,
        relocation_refs: &BTreeMap<(usize, EntryType), DirectoryRef>,
    ) -> io::Result<()> {
        for directory in dirs {
            let converted_name = ty.convert_directory_name(&directory.name);
            let split = if has_rrip {
                let inode = *inode_counter;
                *inode_counter += 1;
                let max = available_su_space(converted_name.as_bytes().len());
                let mut builder = build_rrip_entries(
                    RripEntryKind::Directory {
                        original_name: &directory.rrip_name,
                        metadata: directory.metadata,
                        nlink: 2 + directory.dirs.len() as u32,
                    },
                    inode,
                    options,
                    fallback_time,
                );
                match directory.relocation {
                    DirectoryRelocation::Placeholder { target } => {
                        let target = relocation_refs.get(&(target, ty)).ok_or_else(|| {
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

            let record = PendingRecord {
                name: converted_name.as_bytes().to_vec(),
                split,
                dir_ref: match directory.relocation {
                    DirectoryRelocation::Placeholder { target } => relocation_refs
                        .get(&(target, ty))
                        .copied()
                        .unwrap_or_default(),
                    _ => *directory.entries.get(&ty).unwrap(),
                },
                flags: FileFlags::DIRECTORY,
            };

            records.push(record);
        }
        Ok(())
    }

    fn build_file_records(
        records: &mut Vec<PendingRecord>,
        files: &[WrittenFile],
        ty: EntryType,
        has_rrip: bool,
        options: &RripOptions,
        inode_counter: &mut u32,
        fallback_time: &RripTime,
    ) -> io::Result<()> {
        for file in files {
            let converted_name = ty.convert_name(&file.name);
            
            // Main file entry (with RRIP if enabled)
            let split = if has_rrip {
                let inode = *inode_counter;
                *inode_counter += 1;
                let max = available_su_space(converted_name.as_bytes().len());
                let kind = RripEntryKind::Entry {
                    original_name: &file.name,
                    metadata: file.metadata,
                    kind: &file.kind,
                };
                build_rrip_entries(
                    kind,
                    inode,
                    options,
                    fallback_time,
                )
                .build_split(max)
            } else {
                SplitSu::empty()
            };

            let first_flags = if file.additional_extents.is_empty() {
                FileFlags::empty()
            } else {
                FileFlags::NOT_FINAL
            };
            
            let first = PendingRecord {
                name: converted_name.as_bytes().to_vec(),
                split,
                dir_ref: file.entry,
                flags: first_flags,
            };
            records.push(first);

            // Push additional extents (multi-extent files)
            // ECMA-119 9.1.4: Each extent gets its own directory record
            // with the same file identifier.
            let len = file.additional_extents.len();
            for (i, ext) in file.additional_extents.iter().enumerate() {

                let flags = if i == len - 1 {
                    FileFlags::empty()  // Last extent
                } else {
                    FileFlags::NOT_FINAL  // Middle extents
                };
                
                let record = PendingRecord {
                    name: converted_name.as_bytes().to_vec(),
                    split: SplitSu::empty(),
                    dir_ref: *ext,
                    flags
                };

                records.push(record);
            }
        }
        Ok(())
    }

    fn deduplicate_names(records: &mut Vec<PendingRecord>, ty: EntryType) {
        
        let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
        for record in records {
            // Skip dot/dotdot entries
            if record.name.len() == 1 && (record.name[0] == 0x00 || record.name[0] == 0x01) {
                continue;
            }

            if record.flags.contains(FileFlags::NOT_FINAL) {
                continue;
            }
                
            if seen.insert(record.name.clone()) {
                continue;
            }

            let original = record.name.clone();
            let mut suffix = 1;
            loop {
                let candidate = apply_dedup_suffix(&original, suffix, ty);
                suffix += 1;
                if seen.insert(candidate.clone()) {
                    record.name = candidate;
                    break;
                }
            }
        }
    }

    fn sort_records(records: &mut Vec<PendingRecord>) {
        records.sort_by(|a, b| {
            let rank = |name: &[u8]| match name {
                [0x00] => 0,
                [0x01] => 1,
                _ => 2,
            };
            rank(&a.name)
                .cmp(&rank(&b.name))
                .then_with(|| a.name.cmp(&b.name))
        });
    }

    /// Total length of all overflow (continuation area) data.
    pub fn overflow_len(&self) -> u64 {
        self.0
            .iter()
            .filter(|r| r.split.has_overflow())
            .map(|r| r.split.overflow.len() as u64)
            .sum()
    }

    /// Returns true if any record has RRIP overflow (needs continuation area).
    pub fn has_overflow(&self) -> bool {
        self.0.iter().any(|r| r.split.has_overflow())
    }

    /// Iterates over pending records.
    pub fn iter(&self) -> impl Iterator<Item = &PendingRecord> {
        self.0.iter()
    }

    /// Mutably iterates over pending records.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut PendingRecord> {
        self.0.iter_mut()
    }
}

/// A pending directory record, built in phase 1 and written in phases 2-3.
#[derive(Debug, Clone)]
pub struct PendingRecord {
    /// File or directory name.
    pub name: Vec<u8>,
    /// System Use data (split into inline and overflow).
    pub split: SplitSu,
    /// Location and size of the file/dir.
    pub dir_ref: DirectoryRef,
    /// File flags (directory, hidden, etc.).
    pub flags: FileFlags,
}

impl PendingRecord {
    /// Current directory "." entry.
    pub fn current_dir(dot_split: SplitSu) -> Self {
        Self {
            name: vec![0x00],
            split: dot_split,
            dir_ref: DirectoryRef::default(),
            flags: FileFlags::DIRECTORY,
        }
    }

    /// Parent directory ".." entry.
    pub fn parent_dir(dotdot_split: SplitSu) -> Self {
        Self {
            name: vec![0x01],
            split: dotdot_split,
            dir_ref: DirectoryRef::default(),
            flags: FileFlags::DIRECTORY,
        }
    }
}