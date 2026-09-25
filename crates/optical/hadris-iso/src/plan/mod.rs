//! Lays a tree out as an image, without I/O.
//!
//! The writer of each mode measures the contents, calls [`plan`] and then
//! writes the [`Region`]s it returns in ascending order. Every structure is
//! computed here, so the writer writes each block once and in order.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;

use hadris_fs::{
    Content, DateTime, DeviceNumber, ErrorKind, Extent, Field, FileType, Name, Node, PathError,
    Report, SetAttr, Tree, TreeEntry, Warning, WarningKind,
};
use hadris_part::gpt::types as part_types;
use hadris_part::{
    Disk, Gpt, GptEntry, Guid, Hybrid as HybridTable, HybridMbr, Mbr, MbrEntry, MbrType,
    PartitionFlags, PartitionName, TableError,
};
use hadris_storage::BlockSize;

use crate::boot::Platform;
use crate::error::{Detail, Error};
use crate::namespace::JolietLevel;
use crate::options::{
    BootEntry, BootInfo, Hybrid, IsoDate, IsoId, IsoLevel, IsoOptions, PartitionScheme, Preserve,
    Relocation,
};
use crate::raw::{
    self, BootRecordVolumeDescriptor, DecDateTime, DirDateTime, DirectoryRecord, FileFlags, IsoStr,
    PathTableHeader, PrimaryVolumeDescriptor, RootDirectoryRecord, SECTOR_SIZE,
    SupplementaryVolumeDescriptor, U16Both, U32Be, U32Both, U32Le, VolumeDescriptorHeader,
    VolumeDescriptorSetTerminator,
};
use crate::rock_ridge::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFLNK, S_IFREG};

pub(crate) mod names;
pub(crate) mod susp;

use names::Rules;
use susp::{SplitSu, SuBuilder, inline_space};

type PlanError = Error<Infallible>;
type PlanResult<T> = Result<T, PlanError>;
/// The system area, and the backup GPT region with its first block.
type SystemArea = (Vec<u8>, Option<(u64, Vec<u8>)>);

const SECTOR: u64 = SECTOR_SIZE as u64;
/// The largest extent: the biggest multiple of the block size below 4 GiB.
const MAX_EXTENT: u64 = (u32::MAX as u64 / SECTOR) * SECTOR;
/// The directory depth ECMA-119 allows, the root included.
const MAX_DEPTH: usize = 8;
/// The longest path ECMA-119 allows.
const MAX_PATH: usize = 255;
const PART_BLOCK: BlockSize = match BlockSize::new(512) {
    Some(size) => size,
    None => panic!("512 is not zero"),
};
const BACKUP_GPT_SECTORS: u64 = 33;
/// Zero blocks after the data, counted in the volume, as `mkisofs -pad`
/// and xorriso write by default. Readers such as `isoinfo` read ahead
/// past the last structure and fail on shorter images.
pub(crate) const PADDING_BLOCKS: u64 = 150;
const ISO_DATA_START_512: u64 = 64;

/// What the writer learned about a file's content before planning.
#[derive(Debug, Clone)]
pub(crate) struct ContentInfo {
    pub(crate) len: u64,
    /// Extents already on the output, for session writes.
    pub(crate) stored: Option<Vec<Extent>>,
}

/// Where a plan starts on the output.
#[derive(Debug, Clone)]
pub(crate) struct Base {
    /// The block of the first volume descriptor.
    pub(crate) descriptors: u64,
    /// Directories and files go at or after this block.
    pub(crate) first_block: u64,
    /// Write zeros into the blocks no structure covers.
    pub(crate) fill_gaps: bool,
    /// Write the system area and the partition tables.
    pub(crate) system_area: bool,
    /// Keep an existing boot catalog at this block when the options have no
    /// El Torito.
    pub(crate) keep_catalog: Option<u32>,
    /// Leave room for a backup GPT after the data, for partition tables the
    /// caller writes itself.
    pub(crate) gpt_backup: bool,
}

impl Base {
    pub(crate) const fn image() -> Self {
        Self {
            descriptors: raw::DESCRIPTOR_START as u64,
            first_block: 0,
            fill_gaps: true,
            system_area: true,
            keep_catalog: None,
            gpt_backup: false,
        }
    }
}

/// Where a boot information table goes in a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InfoTable {
    pub(crate) kind: BootInfo,
    pub(crate) block: u32,
    pub(crate) len: u32,
}

/// One piece of the image, written by the mode's writer.
#[derive(Debug)]
pub(crate) enum Region {
    /// Bytes, padded with zeros to whole blocks.
    Bytes { block: u64, data: Vec<u8> },
    /// The content of the file at a tree path, padded to whole blocks.
    File {
        block: u64,
        path: String,
        len: u64,
        info: Option<InfoTable>,
    },
    /// An appended partition's content, padded to whole blocks.
    Content {
        block: u64,
        content: Content,
        info: Option<InfoTable>,
    },
}

impl Region {
    pub(crate) fn block(&self) -> u64 {
        match self {
            Self::Bytes { block, .. } | Self::File { block, .. } | Self::Content { block, .. } => {
                *block
            }
        }
    }

    pub(crate) fn blocks(&self) -> u64 {
        let len = match self {
            Self::Bytes { data, .. } => data.len() as u64,
            Self::File { len, .. } => *len,
            Self::Content { content, .. } => content.len(),
        };
        len.div_ceil(SECTOR)
    }
}

/// The layout of an image.
#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) regions: Vec<Region>,
    /// The block after the ISO data, where a backup GPT region starts.
    pub(crate) end_blocks: u64,
    pub(crate) total_blocks: u64,
    pub(crate) fill_gaps: bool,
    pub(crate) report: Report,
}

// ---------------------------------------------------------------------------
// The tree as the planner sees it.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Data {
        node: usize,
        len: u64,
    },
    Symlink {
        node: usize,
    },
    Device {
        node: usize,
        kind: FileType,
        number: DeviceNumber,
    },
    Catalog {
        len: u64,
    },
}

#[derive(Debug)]
struct PFile {
    name: String,
    path: String,
    kind: FileKind,
    meta: SetAttr,
    links: u32,
}

impl PFile {
    fn node(&self) -> Option<usize> {
        match self.kind {
            FileKind::Data { node, .. }
            | FileKind::Symlink { node }
            | FileKind::Device { node, .. } => Some(node),
            FileKind::Catalog { .. } => None,
        }
    }
}

#[derive(Debug)]
struct PDir {
    name: String,
    /// The primary identifier's source: `RRD000001` for a relocated
    /// directory, the name otherwise.
    iso_name: String,
    path: String,
    meta: SetAttr,
    parent: usize,
    dirs: Vec<usize>,
    files: Vec<usize>,
    /// The primary tree's parent, when it differs from the parent.
    moved_to: Option<usize>,
    /// Directories moved away from this one; each leaves a `CL` record.
    placeholders: Vec<usize>,
    /// The directories the primary tree holds here, relocated ones included.
    physical: Vec<usize>,
    serial: u32,
}

/// The trees an image gets, in descriptor order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeKind {
    Primary,
    Enhanced,
    Joliet(JolietLevel),
}

struct Planner<'a> {
    tree: &'a Tree,
    opts: &'a IsoOptions,
    contents: &'a BTreeMap<usize, ContentInfo>,
    base: Base,
    now: DateTime,
    dirs: Vec<PDir>,
    files: Vec<PFile>,
    trees: Vec<(TreeKind, Rules)>,
    rock_ridge: bool,
    rr_moved: Option<usize>,
    serials: BTreeMap<usize, u32>,
    warnings: Vec<Warning>,
    /// The first extent and length of each data file, by tree node; the
    /// visible catalog is `usize::MAX`.
    extents: BTreeMap<usize, Vec<(u32, u64)>>,
    /// The first block and length of each appended partition.
    appended: Vec<(u32, u64)>,
    dir_refs: BTreeMap<(usize, usize), (u32, u32)>,
}

const CATALOG: usize = usize::MAX;

fn invalid(detail: Detail) -> PlanError {
    detail.invalid()
}

fn too_large() -> PlanError {
    Detail::ImageTooLarge.invalid()
}

fn part_error(_: TableError) -> PlanError {
    Detail::HybridBoot.invalid()
}

fn block_of(offset: u64) -> PlanResult<u32> {
    u32::try_from(offset / SECTOR).map_err(|_| too_large())
}

/// A tree path in the form the planner keys it by: `/` separated, with a
/// leading `/` and no empty components.
pub(crate) fn normalize(path: &str) -> String {
    let mut out = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        out.push('/');
        out.push_str(part);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// A tree name as text; names that are not UTF-8 get U+FFFD and a
/// warning.
fn text(name: &Name, path: &str, warnings: &mut Vec<Warning>) -> String {
    match core::str::from_utf8(name.as_bytes()) {
        Ok(text) => String::from(text),
        Err(_) => {
            let text = String::from_utf8_lossy(name.as_bytes()).into_owned();
            warnings.push(
                Warning::new(WarningKind::Renamed, "iso 9660 names are unicode")
                    .with_path(path)
                    .with_stored_as(&text),
            );
            text
        }
    }
}

/// Measures every file of `tree` without I/O: content lengths are fixed
/// when the content is made. Stored content is accepted only when `stored`
/// is set, for sessions.
pub(crate) fn measure(
    tree: &Tree,
    stored: bool,
) -> Result<BTreeMap<usize, ContentInfo>, PathError> {
    let mut out = BTreeMap::new();
    let mut pending = vec![(Vec::new(), tree.root())];
    while let Some((dir_path, dir)) = pending.pop() {
        for (name, child) in dir.children() {
            let mut path = dir_path.clone();
            path.push(b'/');
            path.extend_from_slice(name.as_bytes());
            let node = child.node();
            if node.file_type() == FileType::Dir {
                pending.push((path, child));
                continue;
            }
            let Some(content) = node.content() else {
                continue;
            };
            if out.contains_key(&child.id()) {
                continue;
            }
            let info = match content.stored_extents() {
                Some(extents) if stored => ContentInfo {
                    len: content.len(),
                    stored: Some(extents.to_vec()),
                },
                Some(_) => {
                    return Err(PathError::from(
                        Detail::StoredContent.error::<Infallible>(ErrorKind::Unsupported),
                    )
                    .with_path(path));
                }
                None => ContentInfo {
                    len: content.len(),
                    stored: None,
                },
            };
            out.insert(child.id(), info);
        }
    }
    Ok(out)
}

fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        alloc::format!("/{name}")
    } else {
        alloc::format!("{parent}/{name}")
    }
}

/// Plans writing `tree` as an ISO 9660 image, without I/O, and returns the
/// report `write` returns: the image size, where each file's data goes, and
/// what the image cannot store as the tree asks.
///
/// Size an output device with [`Report::size`]; the image includes the
/// 150 zero blocks after the data that xorriso and `mkisofs -pad` write,
/// and any backup GPT. Fails as `write` does before it writes anything:
/// [`ErrorKind::InvalidInput`] for a missing boot image, a diskette image
/// of the wrong size, a load size of zero, an identifier that does not
/// fit, a relocation clash or a tree too deep without Rock Ridge;
/// [`ErrorKind::LimitExceeded`] for MBR boot code over 446 bytes;
/// [`ErrorKind::FileTooLarge`] for a file of 4 GiB or more below Level 3;
/// and [`ErrorKind::Unsupported`] for content stored on another image.
pub fn plan(tree: &Tree, opts: &IsoOptions) -> Result<Report, PathError> {
    let contents = measure(tree, false)?;
    Ok(lay_out(tree, opts, &contents, Base::image())?.report)
}

/// Lays `tree` out as `opts` says, from `base`.
pub(crate) fn lay_out(
    tree: &Tree,
    opts: &IsoOptions,
    contents: &BTreeMap<usize, ContentInfo>,
    base: Base,
) -> PlanResult<Plan> {
    let rock_ridge = opts.rock_ridge();
    let case = opts.name_case();
    let level = opts.level();
    let mut trees = vec![(TreeKind::Primary, Rules::Primary { level, case })];
    if opts.iso1999() {
        trees.push((TreeKind::Enhanced, Rules::Enhanced));
    }
    if opts.joliet() {
        trees.push((TreeKind::Joliet(JolietLevel::L3), Rules::Joliet));
    }
    let mut planner = Planner {
        tree,
        opts,
        contents,
        base,
        now: opts.time(),
        dirs: Vec::new(),
        files: Vec::new(),
        trees,
        rock_ridge,
        rr_moved: None,
        serials: BTreeMap::new(),
        warnings: Vec::new(),
        extents: BTreeMap::new(),
        appended: Vec::new(),
        dir_refs: BTreeMap::new(),
    };
    planner.gather()?;
    planner.check_boot()?;
    planner.insert_catalog()?;
    planner.check_depth()?;
    planner.relocate()?;
    planner.assign_serials();
    planner.layout()
}

impl Planner<'_> {
    // -----------------------------------------------------------------------
    // Gathering the tree.

    fn gather(&mut self) -> PlanResult<()> {
        let root = self.tree.root();
        self.dirs.push(PDir {
            name: String::new(),
            iso_name: String::new(),
            path: String::from("/"),
            meta: *root.node().attrs(),
            parent: 0,
            dirs: Vec::new(),
            files: Vec::new(),
            moved_to: None,
            placeholders: Vec::new(),
            physical: Vec::new(),
            serial: 0,
        });
        let mut pending = vec![(0usize, root)];
        while let Some((dir, entry)) = pending.pop() {
            let path = self.dirs[dir].path.clone();
            let mut children = Vec::new();
            for (raw_name, child) in entry.children() {
                let raw_path = join(&path, &String::from_utf8_lossy(raw_name.as_bytes()));
                let name = text(raw_name, &raw_path, &mut self.warnings);
                let child_path = join(&path, &name);
                let node = child.node();
                let kind = match node.file_type() {
                    FileType::Dir => {
                        let id = self.dirs.len();
                        self.dirs.push(PDir {
                            name: name.clone(),
                            iso_name: name,
                            path: child_path,
                            meta: *node.attrs(),
                            parent: dir,
                            dirs: Vec::new(),
                            files: Vec::new(),
                            moved_to: None,
                            placeholders: Vec::new(),
                            physical: Vec::new(),
                            serial: 0,
                        });
                        self.dirs[dir].dirs.push(id);
                        children.push((id, child));
                        continue;
                    }
                    FileType::File => {
                        let len = match self.contents.get(&child.id()) {
                            Some(info) => info.len,
                            None => node.content().map_or(0, Content::len),
                        };
                        if len >= MAX_EXTENT && !matches!(self.opts.level(), IsoLevel::L3) {
                            return Err(Detail::ImageTooLarge.error(ErrorKind::FileTooLarge));
                        }
                        FileKind::Data {
                            node: child.id(),
                            len,
                        }
                    }
                    FileType::Symlink if self.rock_ridge => FileKind::Symlink { node: child.id() },
                    kind @ (FileType::CharDevice | FileType::BlockDevice)
                        if self.rock_ridge && node.device().is_some() =>
                    {
                        FileKind::Device {
                            node: child.id(),
                            kind,
                            number: node.device().unwrap_or(DeviceNumber::new(0, 0)),
                        }
                    }
                    _ => {
                        self.warnings.push(
                            Warning::new(
                                WarningKind::Skipped,
                                match self.rock_ridge {
                                    true => "iso 9660 stores no fifos or sockets",
                                    false => "symlinks and device nodes need rock ridge",
                                },
                            )
                            .with_path(&child_path),
                        );
                        continue;
                    }
                };
                let id = self.files.len();
                self.files.push(PFile {
                    name,
                    path: child_path,
                    kind,
                    meta: *node.attrs(),
                    links: u32::try_from(child.links()).unwrap_or(u32::MAX),
                });
                self.dirs[dir].files.push(id);
            }
            for child in children.into_iter().rev() {
                pending.push(child);
            }
        }
        self.dirs
            .iter_mut()
            .for_each(|dir| dir.physical = dir.dirs.clone());
        Ok(())
    }

    /// The tree entry of the file at `path`.
    fn file_entry(&self, path: &str) -> Option<TreeEntry<'_>> {
        self.tree
            .entry(path)
            .filter(|entry| entry.node().file_type() == FileType::File)
    }

    /// Checks the boot options against the tree before anything is laid
    /// out: boot images exist and fit their emulation, a load size is not
    /// zero, and the MBR boot code fits. Warns about load sizes past the
    /// image and a hybrid table that gets no EFI system partition.
    fn check_boot(&mut self) -> PlanResult<()> {
        let appended = self.opts.hybrid().map_or(&[][..], Hybrid::appended);
        if let Some(hybrid) = self.opts.hybrid() {
            if hybrid.bootstrap().is_some_and(|code| code.len() > 446) {
                return Err(Detail::HybridBoot.error(ErrorKind::LimitExceeded));
            }
            if (hybrid.scheme() == PartitionScheme::Mbr && !appended.is_empty())
                || appended
                    .iter()
                    .any(|partition| partition.content().is_empty())
            {
                return Err(invalid(Detail::HybridBoot));
            }
        }
        let Some(el_torito) = self.opts.el_torito() else {
            return Ok(());
        };
        if el_torito.entries().is_empty() {
            return Err(invalid(Detail::BootImage));
        }
        for entry in el_torito.entries() {
            let len = match (entry.image(), entry.appended()) {
                (Some(path), _) => {
                    let node = self.file_entry(path).ok_or(invalid(Detail::BootImage))?;
                    self.contents.get(&node.id()).map_or(0, |info| info.len)
                }
                (None, Some(index)) => appended
                    .get(index)
                    .ok_or(invalid(Detail::BootImage))?
                    .content()
                    .len(),
                (None, None) => return Err(invalid(Detail::BootImage)),
            };
            let floppy = match entry.emulation() {
                crate::Emulation::Floppy12 => Some(1_228_800),
                crate::Emulation::Floppy144 => Some(1_474_560),
                crate::Emulation::Floppy288 => Some(2_949_120),
                _ => None,
            };
            if floppy.is_some_and(|size| size != len) || entry.load_size() == Some(0) {
                return Err(invalid(Detail::BootImage));
            }
            if entry.emulation() == crate::Emulation::NoEmulation
                && let Some(load) = entry.load_size()
                && u64::from(load) > len.div_ceil(512)
            {
                let mut warning = Warning::new(
                    WarningKind::Boot,
                    "the load size runs past the boot image; firmware loads the bytes after it too",
                );
                if let Some(path) = entry.image() {
                    warning = warning.with_path(normalize(path));
                }
                self.warnings.push(warning);
            }
        }
        if let Some(hybrid) = self.opts.hybrid()
            && hybrid.scheme() != PartitionScheme::Mbr
            && appended.is_empty()
        {
            let mut uefi = el_torito
                .entries()
                .iter()
                .skip(1)
                .filter(|entry| entry.platform() == Platform::Efi);
            if let (Some(first), Some(_)) = (uefi.next(), uefi.next()) {
                let mut warning = Warning::new(
                    WarningKind::Boot,
                    "several UEFI boot entries and no appended partition: the partition table has no EFI system partition",
                );
                if let Some(path) = first.image() {
                    warning = warning.with_path(normalize(path));
                }
                self.warnings.push(warning);
            }
        }
        Ok(())
    }

    /// Adds the visible boot catalog as a file, where the V2 writer put it:
    /// before the last entry that precedes the first directory.
    fn insert_catalog(&mut self) -> PlanResult<()> {
        let Some(el_torito) = self.opts.el_torito() else {
            return Ok(());
        };
        let Some(path) = el_torito.catalog_path() else {
            return Ok(());
        };
        let path = normalize(path);
        let (parent_path, name) = path
            .rsplit_once('/')
            .filter(|(_, name)| !name.is_empty())
            .ok_or(invalid(Detail::CatalogPath))?;
        let parent_path = if parent_path.is_empty() {
            "/"
        } else {
            parent_path
        };
        let parent = self
            .dirs
            .iter()
            .position(|dir| dir.path == parent_path)
            .ok_or(invalid(Detail::CatalogPath))?;
        let node = self
            .tree
            .entry(&self.dirs[parent].path)
            .ok_or(invalid(Detail::CatalogPath))?;
        if node.child(name).is_some() {
            return Err(invalid(Detail::CatalogPath));
        }
        let first_dir = node
            .children()
            .position(|(_, child)| child.node().file_type() == FileType::Dir);
        let insert_at = first_dir.unwrap_or(0).saturating_sub(1);
        let files_before = node
            .children()
            .take(insert_at)
            .filter(|(child_name, child)| {
                child.node().file_type() != FileType::Dir
                    && self.dirs[parent]
                        .files
                        .iter()
                        .any(|&f| self.files[f].name.as_bytes() == child_name.as_bytes())
            })
            .count();
        let entries = el_torito.entries().len().saturating_sub(1);
        let len = ((96 + entries * 64) as u64).div_ceil(SECTOR) * SECTOR;
        let id = self.files.len();
        self.files.push(PFile {
            name: name.to_string(),
            path: path.clone(),
            kind: FileKind::Catalog { len },
            meta: SetAttr::new(),
            links: 1,
        });
        self.dirs[parent].files.insert(files_before, id);
        Ok(())
    }

    /// Without Rock Ridge relocation, a tree deeper than ECMA-119 allows
    /// fails.
    fn check_depth(&self) -> PlanResult<()> {
        if self.rock_ridge && self.relocation().and_then(Relocation::directory).is_some() {
            return Ok(());
        }
        let mut pending = vec![(0usize, 1usize, 0usize)];
        while let Some((dir, depth, path_len)) = pending.pop() {
            for &child in &self.dirs[dir].dirs {
                let name_len = self.dirs[child].name.len();
                let child_len = if path_len == 0 {
                    name_len
                } else {
                    path_len + 1 + name_len
                };
                if depth >= MAX_DEPTH || child_len > MAX_PATH {
                    return Err(invalid(Detail::Relocation));
                }
                pending.push((child, depth + 1, child_len));
            }
        }
        Ok(())
    }

    fn relocation(&self) -> Option<Relocation> {
        self.opts.rock_ridge().then(|| self.opts.relocation())
    }

    /// Moves directories deeper than ECMA-119 allows into the relocation
    /// directory of the primary tree.
    fn relocate(&mut self) -> PlanResult<()> {
        let Some(rr_name) = self.relocation().and_then(Relocation::directory) else {
            return Ok(());
        };
        let mut moved = Vec::new();
        let mut pending = vec![(0usize, 1usize, 0usize)];
        let mut counter = 1usize;
        while let Some((dir, depth, path_len)) = pending.pop() {
            let mut retained = Vec::new();
            let mut next = Vec::new();
            for &child in &self.dirs[dir].dirs.clone() {
                let name_len = self.dirs[child].name.len();
                let child_len = if path_len == 0 {
                    name_len
                } else {
                    path_len + 1 + name_len
                };
                if depth + 1 > MAX_DEPTH || child_len > MAX_PATH {
                    let iso_name = alloc::format!("RRD{counter:06}");
                    counter += 1;
                    let relocated_len = rr_name.len() + 1 + iso_name.len();
                    self.dirs[child].iso_name = iso_name;
                    self.dirs[dir].placeholders.push(child);
                    moved.push(child);
                    next.push((child, 3, relocated_len));
                } else {
                    retained.push(child);
                    next.push((child, depth + 1, child_len));
                }
            }
            self.dirs[dir].physical = retained;
            for item in next.into_iter().rev() {
                pending.push(item);
            }
        }
        if moved.is_empty() {
            return Ok(());
        }
        let rules = self.trees[0].1;
        let other = if rr_name == "rr_moved" {
            ".rr_moved"
        } else {
            "rr_moved"
        };
        if let Some(&dir) = self.dirs[0]
            .dirs
            .iter()
            .find(|&&dir| self.dirs[dir].name == other)
            && rules.directory(&self.dirs[dir].iso_name) < rules.directory(rr_name)
        {
            return Err(invalid(Detail::Relocation));
        }
        let existing = self.dirs[0]
            .dirs
            .iter()
            .copied()
            .find(|&dir| self.dirs[dir].name == rr_name);
        let id = match existing {
            Some(id) => id,
            None if self.tree.root().child(rr_name).is_some() => {
                return Err(invalid(Detail::Relocation));
            }
            None => {
                let id = self.dirs.len();
                self.dirs.push(PDir {
                    name: String::from(rr_name),
                    iso_name: String::from(rr_name),
                    path: join("/", rr_name),
                    meta: SetAttr::new(),
                    parent: 0,
                    dirs: Vec::new(),
                    files: Vec::new(),
                    moved_to: None,
                    placeholders: Vec::new(),
                    physical: Vec::new(),
                    serial: 0,
                });
                self.rr_moved = Some(id);
                id
            }
        };
        for &dir in &moved {
            let stored = join(&self.dirs[id].path, &self.dirs[dir].iso_name);
            self.warnings.push(
                Warning::new(WarningKind::Relocated, "rock ridge relocation")
                    .with_path(&self.dirs[dir].path)
                    .with_stored_as(stored),
            );
        }
        self.dirs[0].physical.retain(|&dir| dir != id);
        self.dirs[0].physical.insert(0, id);
        for &dir in &moved {
            self.dirs[dir].moved_to = Some(id);
        }
        self.dirs[id].physical.extend(moved);
        Ok(())
    }

    /// The primary tree's directories, parents before children.
    fn preorder(&self) -> Vec<usize> {
        let mut order = Vec::new();
        let mut pending = vec![0usize];
        while let Some(dir) = pending.pop() {
            order.push(dir);
            for &child in self.dirs[dir].physical.iter().rev() {
                pending.push(child);
            }
        }
        order
    }

    /// The files in image order: by directory in preorder, then as listed.
    fn file_order(&self, order: &[usize]) -> Vec<usize> {
        order
            .iter()
            .flat_map(|&dir| self.dirs[dir].files.iter().copied())
            .collect()
    }

    fn assign_serials(&mut self) {
        let order = self.preorder();
        let mut next = 1u32;
        for &dir in &order {
            self.dirs[dir].serial = next;
            next += 1;
        }
        for file in self.file_order(&order) {
            if let Some(node) = self.files[file].node()
                && !self.serials.contains_key(&node)
            {
                self.serials.insert(node, next);
                next += 1;
            }
        }
    }

    fn file_serial(&self, file: &PFile) -> u32 {
        file.node()
            .and_then(|node| self.serials.get(&node).copied())
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Directory records.

    fn record_time(&self, meta: &SetAttr) -> DirDateTime {
        DirDateTime::from_datetime(meta.modified().unwrap_or(self.now))
    }

    fn rr_time(time: DateTime) -> [u8; 7] {
        DirDateTime::from_datetime(time).to_bytes()
    }

    /// `PX` and `TF` for a node.
    fn posix(
        &self,
        builder: &mut SuBuilder,
        meta: &SetAttr,
        type_mode: u32,
        default: u32,
        links: u32,
        serial: u32,
    ) {
        let preserve = if self.opts.rock_ridge() {
            self.opts.preserve()
        } else {
            Preserve::empty()
        };
        let mode = if preserve.contains(Preserve::PERMISSIONS) {
            meta.permissions().map_or(default, |mode| mode.bits())
        } else {
            default
        };
        let (uid, gid) = if preserve.contains(Preserve::OWNERS) {
            meta.owner()
                .map_or((0, 0), |owner| (owner.uid(), owner.gid()))
        } else {
            (0, 0)
        };
        builder.px(type_mode | mode, links, uid, gid, serial);
        if preserve.contains(Preserve::TIMES) {
            let modified = Self::rr_time(meta.modified().unwrap_or(self.now));
            let accessed = Self::rr_time(meta.accessed().unwrap_or(self.now));
            builder.tf(meta.created().map(Self::rr_time), modified, accessed);
        }
    }

    fn dir_links(&self, dir: usize) -> u32 {
        let d = &self.dirs[dir];
        2 + (d.physical.len() + d.placeholders.len()) as u32
    }

    /// The parent the tree `ti` shows for `dir`.
    fn parent_in(&self, dir: usize, ti: usize) -> usize {
        match (self.trees[ti].0, self.dirs[dir].moved_to) {
            (TreeKind::Primary, Some(moved)) => moved,
            _ => self.dirs[dir].parent,
        }
    }

    /// The directories the tree `ti` holds in `dir`.
    fn subdirs_in(&self, dir: usize, ti: usize) -> &[usize] {
        match self.trees[ti].0 {
            TreeKind::Primary => &self.dirs[dir].physical,
            _ => &self.dirs[dir].dirs,
        }
    }

    fn has_dir(&self, dir: usize, ti: usize) -> bool {
        !matches!(self.trees[ti].0, TreeKind::Enhanced | TreeKind::Joliet(_))
            || Some(dir) != self.rr_moved
    }

    fn dir_ref(&self, dir: usize, ti: usize) -> (u32, u32) {
        self.dir_refs.get(&(dir, ti)).copied().unwrap_or((0, 0))
    }

    /// The extents of a file, as `(block, length)` pairs.
    fn file_extents(&self, file: &PFile) -> Vec<(u32, u64)> {
        let key = match file.kind {
            FileKind::Data { node, .. } => node,
            FileKind::Catalog { .. } => CATALOG,
            _ => return vec![(0, 0)],
        };
        match self.extents.get(&key) {
            Some(extents) if !extents.is_empty() => extents.clone(),
            _ => vec![(0, 0)],
        }
    }

    /// The records of `dir` in tree `ti`, sorted by identifier.
    fn records(&self, dir: usize, ti: usize) -> PlanResult<Vec<PendingRecord>> {
        let (tree, rules) = self.trees[ti];
        let rr = self.rock_ridge && tree == TreeKind::Primary;
        let d = &self.dirs[dir];
        let mut records = Vec::new();
        let now = self.record_time(&d.meta);

        let dot = if rr {
            let mut b = SuBuilder::default();
            if dir == 0 {
                b.sp();
            }
            self.posix(
                &mut b,
                &d.meta,
                S_IFDIR,
                0o755,
                self.dir_links(dir),
                d.serial,
            );
            b.nm_current();
            if dir == 0 {
                b.er();
            }
            b.split(inline_space(1))
        } else {
            SplitSu::default()
        };
        records.push(PendingRecord {
            name: vec![0],
            split: dot,
            extent: self.dir_ref(dir, ti),
            flags: FileFlags::DIRECTORY,
            time: now,
        });

        let parent = self.parent_in(dir, ti);
        let dotdot = if rr {
            let p = &self.dirs[parent];
            let mut b = SuBuilder::default();
            self.posix(
                &mut b,
                &p.meta,
                S_IFDIR,
                0o755,
                self.dir_links(parent),
                p.serial,
            );
            b.nm_parent();
            if d.moved_to.is_some() {
                b.pl(self.dir_ref(d.parent, ti).0);
            }
            b.split(inline_space(1))
        } else {
            SplitSu::default()
        };
        records.push(PendingRecord {
            name: vec![1],
            split: dotdot,
            extent: self.dir_ref(parent, ti),
            flags: FileFlags::DIRECTORY,
            time: self.record_time(&self.dirs[parent].meta),
        });

        let mut children: Vec<(usize, bool)> = self
            .subdirs_in(dir, ti)
            .iter()
            .map(|&child| (child, false))
            .collect();
        if rr {
            children.extend(d.placeholders.iter().map(|&child| (child, true)));
        }
        for (child, placeholder) in children {
            let c = &self.dirs[child];
            let iso_name = if placeholder || tree != TreeKind::Primary {
                &c.name
            } else {
                &c.iso_name
            };
            let name = rules.directory(iso_name);
            let split = if rr {
                let mut b = SuBuilder::default();
                self.posix(
                    &mut b,
                    &c.meta,
                    S_IFDIR,
                    0o755,
                    self.dir_links(child),
                    c.serial,
                );
                b.nm(c.name.as_bytes());
                if placeholder {
                    b.cl(self.dir_ref(child, ti).0);
                } else if c.moved_to.is_some() {
                    b.re();
                }
                b.split(inline_space(name.len()))
            } else {
                SplitSu::default()
            };
            let flags = if placeholder {
                FileFlags::empty()
            } else {
                FileFlags::DIRECTORY
            };
            records.push(PendingRecord {
                name,
                split,
                extent: self.dir_ref(child, ti),
                flags,
                time: self.record_time(&c.meta),
            });
        }

        for &file in &d.files {
            let f = &self.files[file];
            if !rr && matches!(f.kind, FileKind::Symlink { .. } | FileKind::Device { .. }) {
                continue;
            }
            let name = rules.file(&f.name);
            let split = if rr {
                let mut b = SuBuilder::default();
                let (type_mode, default) = match f.kind {
                    FileKind::Symlink { .. } => (S_IFLNK, 0o777),
                    FileKind::Device {
                        kind: FileType::BlockDevice,
                        ..
                    } => (S_IFBLK, 0o600),
                    FileKind::Device { .. } => (S_IFCHR, 0o600),
                    _ => (S_IFREG, 0o644),
                };
                self.posix(
                    &mut b,
                    &f.meta,
                    type_mode,
                    default,
                    f.links,
                    self.file_serial(f),
                );
                b.nm(f.name.as_bytes());
                match f.kind {
                    FileKind::Symlink { .. } => {
                        if let Some(target) = self.tree.get(&f.path).and_then(Node::target) {
                            b.sl(target);
                        }
                    }
                    FileKind::Device { number, .. } => b.pn(number.major(), number.minor()),
                    _ => {}
                }
                b.split(inline_space(name.len()))
            } else {
                SplitSu::default()
            };
            let extents = self.file_extents(f);
            let time = self.record_time(&f.meta);
            let last = extents.len() - 1;
            for (index, (block, len)) in extents.into_iter().enumerate() {
                let flags = if index == last {
                    FileFlags::empty()
                } else {
                    FileFlags::NOT_FINAL
                };
                let len = u32::try_from(len).map_err(|_| too_large())?;
                records.push(PendingRecord {
                    name: name.clone(),
                    split: split.clone(),
                    extent: (block, len),
                    flags,
                    time,
                });
            }
        }

        dedup(&mut records, rules);
        records.sort_by(|a, b| {
            let rank = |name: &[u8]| match name {
                [0] => 0,
                [1] => 1,
                _ => 2,
            };
            rank(&a.name)
                .cmp(&rank(&b.name))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(records)
    }

    // -----------------------------------------------------------------------
    // Layout.

    fn descriptor_count(&self) -> u64 {
        let boot = self.opts.el_torito().is_some() || self.base.keep_catalog.is_some();
        1 + self.trees.len() as u64 + u64::from(boot)
    }

    fn layout(mut self) -> PlanResult<Plan> {
        let order = self.preorder();
        let files = self.file_order(&order);
        let desc_end = self.base.descriptors + self.descriptor_count();
        let mut cursor = desc_end
            .max(self.opts.min_blocks())
            .max(self.base.first_block)
            * SECTOR;

        let mut sizes = BTreeMap::new();
        for &dir in &order {
            for ti in 0..self.trees.len() {
                if !self.has_dir(dir, ti) {
                    continue;
                }
                let records = self.records(dir, ti)?;
                let (start, sectors) = layout_records(cursor, &records);
                let size = u32::try_from(sectors * SECTOR).map_err(|_| too_large())?;
                self.dir_refs
                    .insert((dir, ti), (block_of(start * SECTOR)?, size));
                sizes.insert((dir, ti), overflow_len(&records));
                cursor = (start + sectors) * SECTOR + overflow_len(&records);
            }
        }

        let mut file_regions = Vec::new();
        for &file in &files {
            let f = &self.files[file];
            let (key, len) = match f.kind {
                FileKind::Data { node, len } => (node, len),
                FileKind::Catalog { len } => (CATALOG, len),
                _ => continue,
            };
            if len == 0 || self.extents.contains_key(&key) {
                continue;
            }
            if let Some(stored) = self
                .contents
                .get(&key)
                .and_then(|info| info.stored.as_ref())
            {
                let mut extents = Vec::new();
                for extent in stored {
                    extents.push((block_of(extent.offset())?, extent.len()));
                }
                self.extents.insert(key, extents);
                continue;
            }
            let mut extents = Vec::new();
            let mut remaining = len;
            let mut at = cursor.div_ceil(SECTOR) * SECTOR;
            let first = at;
            while remaining > 0 {
                let chunk = remaining.min(MAX_EXTENT);
                extents.push((block_of(at)?, chunk));
                at += chunk;
                remaining -= chunk;
            }
            cursor = at;
            self.extents.insert(key, extents);
            if key != CATALOG {
                file_regions.push((first / SECTOR, key, f.path.clone(), len));
            }
        }

        let mut regions = Vec::new();
        for partition in self.opts.hybrid().map_or(&[][..], Hybrid::appended) {
            let content = partition.content().clone();
            let block = cursor.div_ceil(SECTOR);
            self.appended
                .push((block_of(block * SECTOR)?, content.len()));
            cursor = block * SECTOR + content.len();
            regions.push(Region::Content {
                block,
                content,
                info: None,
            });
        }
        for &dir in &order {
            for ti in 0..self.trees.len() {
                if !self.has_dir(dir, ti) {
                    continue;
                }
                let (start, size) = self.dir_ref(dir, ti);
                let mut records = self.records(dir, ti)?;
                let data = emit_records(start, size, &mut records)?;
                regions.push(Region::Bytes {
                    block: u64::from(start),
                    data,
                });
            }
        }

        let (infos, appended_infos) = self.info_tables()?;
        for region in &mut regions {
            if let Region::Content { block, info, .. } = region {
                let index = self
                    .appended
                    .iter()
                    .position(|&(start, _)| u64::from(start) == *block);
                *info = index.and_then(|index| appended_infos.get(&index).copied());
            }
        }
        for (block, node, path, len) in file_regions {
            regions.push(Region::File {
                block,
                path,
                len,
                info: infos.get(&node).copied(),
            });
        }

        let mut tables = Vec::new();
        for ti in 0..self.trees.len() {
            let l = self.path_table(ti, false)?;
            let m = self.path_table(ti, true)?;
            let size = u32::try_from(l.len()).map_err(|_| too_large())?;
            let l_block = cursor.div_ceil(SECTOR);
            let m_block = l_block + (l.len() as u64).div_ceil(SECTOR);
            cursor = (m_block + (m.len() as u64).div_ceil(SECTOR)) * SECTOR;
            tables.push((
                block_of(l_block * SECTOR)?,
                block_of(m_block * SECTOR)?,
                size,
            ));
            regions.push(Region::Bytes {
                block: l_block,
                data: l,
            });
            regions.push(Region::Bytes {
                block: m_block,
                data: m,
            });
        }

        let catalog = self.boot_catalog()?;
        let catalog_block = match (&catalog, self.extents.get(&CATALOG)) {
            (Some(bytes), Some(extent)) => {
                let block = extent[0].0;
                let mut data = bytes.clone();
                data.resize(extent[0].1 as usize, 0);
                regions.push(Region::Bytes {
                    block: u64::from(block),
                    data,
                });
                Some(block)
            }
            (Some(bytes), None) => {
                let block = cursor.div_ceil(SECTOR);
                cursor = block * SECTOR + bytes.len() as u64;
                regions.push(Region::Bytes {
                    block,
                    data: bytes.clone(),
                });
                Some(block_of(block * SECTOR)?)
            }
            _ => self.base.keep_catalog,
        };

        let data_end = cursor.div_ceil(SECTOR);
        regions.push(Region::Bytes {
            block: data_end,
            data: vec![0; PADDING_BLOCKS as usize * SECTOR_SIZE],
        });
        let end = data_end + PADDING_BLOCKS;
        let hybrid = if self.base.system_area {
            self.opts.hybrid()
        } else {
            None
        };
        let gpt = matches!(
            hybrid.map(Hybrid::scheme),
            Some(PartitionScheme::Gpt | PartitionScheme::GptHybridMbr)
        );
        let total = if gpt || self.base.gpt_backup {
            (end * 4 + BACKUP_GPT_SECTORS).div_ceil(4)
        } else {
            end
        };
        let volume_blocks = u32::try_from(total).map_err(|_| too_large())?;

        let descriptors = self.descriptors(volume_blocks, &tables, catalog_block)?;
        regions.push(Region::Bytes {
            block: self.base.descriptors,
            data: descriptors,
        });

        if self.base.system_area {
            let (system, tail) = self.system_area(end, total)?;
            regions.push(Region::Bytes {
                block: 0,
                data: system,
            });
            if let Some((block, data)) = tail {
                regions.push(Region::Bytes { block, data });
            }
        }

        regions.sort_by_key(Region::block);
        let report = self.report(total);
        Ok(Plan {
            regions,
            end_blocks: end,
            total_blocks: total,
            fill_gaps: self.base.fill_gaps,
            report,
        })
    }

    // -----------------------------------------------------------------------
    // Path tables.

    fn path_table(&self, ti: usize, big_endian: bool) -> PlanResult<Vec<u8>> {
        let rules = self.trees[ti].1;
        let mut out = Vec::new();
        let push = |out: &mut Vec<u8>, name: &[u8], extent: u32, parent: u16| {
            let len = name.len() as u8;
            let header = if big_endian {
                PathTableHeader::big(len, extent, parent)
            } else {
                PathTableHeader::little(len, extent, parent)
            };
            out.extend_from_slice(bytemuck::bytes_of(&header));
            out.extend_from_slice(name);
            if name.len() % 2 == 1 {
                out.push(0);
            }
        };
        push(&mut out, &[0], self.dir_ref(0, ti).0, 1);
        let mut queue = VecDeque::from([(0usize, 1u16)]);
        let mut number = 1u16;
        while let Some((dir, own)) = queue.pop_front() {
            let mut children: Vec<(Vec<u8>, usize)> = self
                .subdirs_in(dir, ti)
                .iter()
                .map(|&child| {
                    let source = match self.trees[ti].0 {
                        TreeKind::Primary => &self.dirs[child].iso_name,
                        _ => &self.dirs[child].name,
                    };
                    (rules.directory(source), child)
                })
                .collect();
            let records = self.records(dir, ti)?;
            for (name, child) in &mut children {
                let extent = self.dir_ref(*child, ti).0;
                if let Some(record) = records.iter().find(|r| {
                    r.flags.contains(FileFlags::DIRECTORY)
                        && r.extent.0 == extent
                        && r.name.len() > 1
                }) {
                    *name = record.name.clone();
                }
            }
            children.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, child) in children {
                number = number.checked_add(1).ok_or(too_large())?;
                push(&mut out, &name, self.dir_ref(child, ti).0, own);
                queue.push_back((child, number));
            }
        }
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // El Torito.

    fn boot_image(&self, path: &str) -> PlanResult<(u32, u64)> {
        let node = self.file_entry(path).ok_or(invalid(Detail::BootImage))?;
        let len = self.contents.get(&node.id()).map_or(0, |info| info.len);
        let block = self
            .extents
            .get(&node.id())
            .and_then(|extents| extents.first())
            .map_or(0, |extent| extent.0);
        Ok((block, len))
    }

    /// The first block and length of an entry's image.
    fn entry_image(&self, entry: &BootEntry) -> PlanResult<(u32, u64)> {
        match (entry.image(), entry.appended()) {
            (Some(path), _) => self.boot_image(path),
            (None, Some(index)) => self
                .appended
                .get(index)
                .copied()
                .ok_or(invalid(Detail::BootImage)),
            (None, None) => Err(invalid(Detail::BootImage)),
        }
    }

    /// The boot information tables, by the tree node of the image and by
    /// appended partition.
    #[allow(clippy::type_complexity)]
    fn info_tables(&self) -> PlanResult<(BTreeMap<usize, InfoTable>, BTreeMap<usize, InfoTable>)> {
        let mut nodes = BTreeMap::new();
        let mut appended = BTreeMap::new();
        let Some(el_torito) = self.opts.el_torito() else {
            return Ok((nodes, appended));
        };
        for entry in el_torito.entries() {
            if entry.boot_info() == BootInfo::None {
                continue;
            }
            let (block, len) = self.entry_image(entry)?;
            if len < 64 {
                return Err(invalid(Detail::BootInfoTable));
            }
            let table = InfoTable {
                kind: entry.boot_info(),
                block,
                len: u32::try_from(len).map_err(|_| invalid(Detail::BootInfoTable))?,
            };
            match (entry.image(), entry.appended()) {
                (Some(path), _) => {
                    let node = self
                        .tree
                        .entry(path)
                        .map(|node| node.id())
                        .ok_or(invalid(Detail::BootImage))?;
                    nodes.insert(node, table);
                }
                (None, Some(index)) => {
                    appended.insert(index, table);
                }
                (None, None) => return Err(invalid(Detail::BootImage)),
            }
        }
        Ok((nodes, appended))
    }

    fn boot_catalog(&self) -> PlanResult<Option<Vec<u8>>> {
        let Some(el_torito) = self.opts.el_torito() else {
            return Ok(None);
        };
        let entries = el_torito.entries();
        let mut out = Vec::new();
        let platform = entries
            .first()
            .map_or(Platform::X86, |entry| entry.platform());
        out.extend_from_slice(bytemuck::bytes_of(&raw::BootValidationEntry::new(
            platform.id(),
        )));
        for (index, entry) in entries.iter().enumerate() {
            let (block, len) = self.entry_image(entry)?;
            let load = entry.load_size().unwrap_or_else(|| {
                if entry.emulation() != crate::Emulation::NoEmulation {
                    1
                } else {
                    u16::try_from(len.div_ceil(512)).unwrap_or(u16::MAX)
                }
            });
            let section = raw::BootSectionEntry::new(
                entry.emulation().media_type(),
                entry.load_segment(),
                load,
                block,
            );
            if index > 0 {
                let header = raw::BootCatalogHeader {
                    header_type: if index + 1 == entries.len() {
                        raw::HEADER_FINAL
                    } else {
                        raw::HEADER_MORE
                    },
                    platform_id: entry.platform().id(),
                    section_count: raw::U16Le::new(1),
                    id_string: [0; 28],
                };
                out.extend_from_slice(bytemuck::bytes_of(&header));
            }
            out.extend_from_slice(bytemuck::bytes_of(&section));
        }
        out.extend_from_slice(&[0; 32]);
        Ok(Some(out))
    }

    // -----------------------------------------------------------------------
    // Descriptors.

    fn identifier<const N: usize>(&self, id: IsoId) -> PlanResult<IsoStr<N>> {
        let text = self.opts.id(id).unwrap_or("");
        if text.len() > N {
            return Err(invalid(Detail::Identifier));
        }
        IsoStr::padded(text.as_bytes()).ok_or(invalid(Detail::Identifier))
    }

    fn date(&self, date: IsoDate) -> DecDateTime {
        self.opts
            .date(date)
            .map_or(DecDateTime::UNSPECIFIED, DecDateTime::from_datetime)
    }

    fn ucs2<const N: usize>(text: &str) -> IsoStr<N> {
        let mut bytes = [0u8; N];
        let paired = N & !1;
        for pair in bytes[..paired].chunks_exact_mut(2) {
            pair.copy_from_slice(&[0x00, 0x20]);
        }
        for (pair, unit) in bytes[..paired].chunks_exact_mut(2).zip(text.encode_utf16()) {
            pair.copy_from_slice(&unit.to_be_bytes());
        }
        IsoStr::from_bytes(bytes)
    }

    fn root_record(&self, ti: usize) -> RootDirectoryRecord {
        let (extent, size) = self.dir_ref(0, ti);
        RootDirectoryRecord::new(extent, size, self.record_time(&self.dirs[0].meta))
    }

    fn descriptors(
        &self,
        volume_blocks: u32,
        tables: &[(u32, u32, u32)],
        catalog: Option<u32>,
    ) -> PlanResult<Vec<u8>> {
        let mut out = Vec::new();
        for (ti, (tree, _)) in self.trees.iter().enumerate() {
            let (l, m, size) = tables[ti];
            let mut d: PrimaryVolumeDescriptor = bytemuck::Zeroable::zeroed();
            d.header = VolumeDescriptorHeader::new(raw::DescriptorType::Primary);
            d.volume_space_size = U32Both::new(volume_blocks);
            d.volume_set_size = U16Both::new(1);
            d.volume_sequence_number = U16Both::new(1);
            d.logical_block_size = U16Both::new(SECTOR_SIZE as u16);
            d.path_table_size = U32Both::new(size);
            d.type_l_path_table = U32Le::new(l);
            d.type_m_path_table = U32Be::new(m);
            d.root = self.root_record(ti);
            d.creation_date = self.date(IsoDate::Created);
            d.modification_date = self.date(IsoDate::Modified);
            d.expiration_date = self.date(IsoDate::Expires);
            d.effective_date = self.date(IsoDate::Effective);
            d.file_structure_version = 1;
            match tree {
                TreeKind::Joliet(level) => {
                    let mut s: SupplementaryVolumeDescriptor = bytemuck::cast(d);
                    s.header = VolumeDescriptorHeader::new(raw::DescriptorType::Supplementary);
                    s.escape_sequences = level.escape_sequences();
                    s.system_identifier = Self::ucs2(self.opts.id(IsoId::System).unwrap_or(""));
                    s.volume_identifier = Self::ucs2(self.opts.id(IsoId::Volume).unwrap_or(""));
                    s.volume_set_identifier =
                        Self::ucs2(self.opts.id(IsoId::VolumeSet).unwrap_or(""));
                    s.publisher_identifier =
                        Self::ucs2(self.opts.id(IsoId::Publisher).unwrap_or(""));
                    s.preparer_identifier = Self::ucs2(self.opts.id(IsoId::Preparer).unwrap_or(""));
                    s.application_identifier =
                        Self::ucs2(self.opts.id(IsoId::Application).unwrap_or(""));
                    s.copyright_file_identifier =
                        Self::ucs2(self.opts.id(IsoId::CopyrightFile).unwrap_or(""));
                    s.abstract_file_identifier =
                        Self::ucs2(self.opts.id(IsoId::AbstractFile).unwrap_or(""));
                    s.bibliographic_file_identifier =
                        Self::ucs2(self.opts.id(IsoId::BibliographicFile).unwrap_or(""));
                    out.extend_from_slice(bytemuck::bytes_of(&s));
                }
                _ => {
                    d.system_identifier = self.identifier(IsoId::System)?;
                    d.volume_identifier = self.identifier(IsoId::Volume)?;
                    d.volume_set_identifier = self.identifier(IsoId::VolumeSet)?;
                    d.publisher_identifier = self.identifier(IsoId::Publisher)?;
                    d.preparer_identifier = self.identifier(IsoId::Preparer)?;
                    d.application_identifier = self.identifier(IsoId::Application)?;
                    d.copyright_file_identifier = self.identifier(IsoId::CopyrightFile)?;
                    d.abstract_file_identifier = self.identifier(IsoId::AbstractFile)?;
                    d.bibliographic_file_identifier = self.identifier(IsoId::BibliographicFile)?;
                    if *tree == TreeKind::Enhanced {
                        let mut s: SupplementaryVolumeDescriptor = bytemuck::cast(d);
                        s.header = VolumeDescriptorHeader::new(raw::DescriptorType::Supplementary);
                        s.header.version = 2;
                        s.escape_sequences = [0; 32];
                        s.file_structure_version = 2;
                        out.extend_from_slice(bytemuck::bytes_of(&s));
                    } else {
                        out.extend_from_slice(bytemuck::bytes_of(&d));
                    }
                }
            }
            if ti == 0
                && let Some(block) = catalog
            {
                out.extend_from_slice(bytemuck::bytes_of(&BootRecordVolumeDescriptor::el_torito(
                    block,
                )));
            }
        }
        out.extend_from_slice(bytemuck::bytes_of(&VolumeDescriptorSetTerminator::new()));
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // Hybrid boot.

    /// The first block and length of the EFI system partition.
    fn efi_image(&self) -> PlanResult<Option<(u32, u64)>> {
        if let Some(&first) = self.appended.first() {
            return Ok(Some(first));
        }
        let Some(el_torito) = self.opts.el_torito() else {
            return Ok(None);
        };
        let mut uefi = el_torito
            .entries()
            .iter()
            .skip(1)
            .filter(|entry| entry.platform() == Platform::Efi);
        let Some(first) = uefi.next() else {
            return Ok(None);
        };
        if uefi.next().is_some() {
            return Ok(None);
        }
        self.entry_image(first)
            .map(Some)
            .map_err(|_| invalid(Detail::HybridBoot))
    }

    fn gpt(&self, end: u64, total: u64) -> PlanResult<(Gpt, Option<usize>, Option<usize>)> {
        let iso_512 = end * 4;
        let total_512 = total * 4;
        let volume = self.opts.id(IsoId::Volume).unwrap_or("");
        let mut gpt = Gpt::new(
            self.guid(&alloc::format!("disk-{volume}")),
            total_512,
            PART_BLOCK,
        )
        .map_err(part_error)?;
        let start = gpt.first_usable().max(ISO_DATA_START_512);
        let iso_end = iso_512.saturating_sub(1);
        if iso_end <= start {
            return Err(invalid(Detail::HybridBoot));
        }
        let esp = match self.efi_image()? {
            Some((block, len)) => {
                let first = u64::from(block) * 4;
                let last = first + len.div_ceil(512).max(1) - 1;
                if first < start || last > iso_end {
                    return Err(invalid(Detail::HybridBoot));
                }
                Some((first, last))
            }
            None => None,
        };
        let entry =
            |type_guid, key: &str, first: u64, last: u64, name: &str| -> PlanResult<GptEntry> {
                let name = PartitionName::new(name).map_err(part_error)?;
                Ok(
                    GptEntry::new(type_guid, self.guid(key), first, last - first + 1)
                        .with_name(name),
                )
            };
        let mut iso_index = None;
        let mut esp_index = None;
        match esp {
            Some((esp_start, esp_end)) => {
                if esp_start > start {
                    let data = entry(
                        part_types::BASIC_DATA,
                        volume,
                        start,
                        esp_start - 1,
                        "ISO9660",
                    )?;
                    iso_index = Some(gpt.add(data).map_err(part_error)?);
                }
                let esp = entry(
                    part_types::EFI_SYSTEM,
                    &alloc::format!("esp-{volume}"),
                    esp_start,
                    esp_end,
                    "EFI System Partition",
                )?;
                esp_index = Some(gpt.add(esp).map_err(part_error)?);
                if esp_end < iso_end {
                    let tail = entry(
                        part_types::BASIC_DATA,
                        &alloc::format!("data-{volume}"),
                        esp_end + 1,
                        iso_end,
                        "ISO9660",
                    )?;
                    gpt.add(tail).map_err(part_error)?;
                }
            }
            None => {
                let data = entry(part_types::BASIC_DATA, volume, start, iso_end, "ISO9660")?;
                iso_index = Some(gpt.add(data).map_err(part_error)?);
            }
        }
        Ok((gpt, iso_index, esp_index))
    }

    fn partition_disk(&self, hybrid: &Hybrid, end: u64, total: u64) -> PlanResult<Disk> {
        let mut disk = match hybrid.scheme() {
            PartitionScheme::Mbr => {
                let sectors = u32::try_from(end * 4).map_err(|_| too_large())?;
                let mut mbr = Mbr::new(u64::from(sectors), PART_BLOCK).map_err(part_error)?;
                mbr.add(
                    MbrEntry::new(MbrType::ISO9660, 0, u64::from(sectors))
                        .with_flags(hybrid.flags()),
                )
                .map_err(part_error)?;
                Disk::new(mbr)
            }
            PartitionScheme::Gpt => Disk::new(self.gpt(end, total)?.0),
            PartitionScheme::GptHybridMbr => {
                let (gpt, iso, esp) = self.gpt(end, total)?;
                let mut config = HybridMbr::new();
                if let Some(iso) = iso {
                    config
                        .add_mirrored(iso, MbrType::ISO9660, hybrid.flags())
                        .map_err(part_error)?;
                }
                if let Some(esp) = esp {
                    config
                        .add_mirrored(esp, MbrType::EFI_SYSTEM, PartitionFlags::empty())
                        .map_err(part_error)?;
                }
                Disk::new(HybridTable::new(gpt, &config).map_err(part_error)?)
            }
        };
        if hybrid.scheme() != PartitionScheme::Gpt
            && let Some(code) = hybrid.bootstrap()
        {
            disk.set_bootstrap(code).map_err(part_error)?;
        }
        Ok(disk)
    }

    /// The system area, and the blocks after the ISO data a backup GPT
    /// takes.
    fn system_area(&self, end: u64, total: u64) -> PlanResult<SystemArea> {
        let mut system = vec![0u8; 16 * SECTOR_SIZE];
        let Some(hybrid) = self.opts.hybrid() else {
            return Ok((system, None));
        };
        let disk = self.partition_disk(hybrid, end, total)?;
        let mut tail = vec![0u8; ((total - end) * SECTOR) as usize];
        for run in disk.runs() {
            let offset = run.lba() * 512;
            let bytes = run.bytes();
            if offset + bytes.len() as u64 <= system.len() as u64 {
                system[offset as usize..offset as usize + bytes.len()].copy_from_slice(bytes);
            } else if offset >= end * SECTOR {
                let at = (offset - end * SECTOR) as usize;
                tail.get_mut(at..at + bytes.len())
                    .ok_or(invalid(Detail::HybridBoot))?
                    .copy_from_slice(bytes);
            } else {
                return Err(invalid(Detail::HybridBoot));
            }
        }
        let tail = (!tail.is_empty()).then_some((end, tail));
        Ok((system, tail))
    }

    // -----------------------------------------------------------------------
    // Report.

    fn report(&self, total: u64) -> Report {
        let mut report = Report::new();
        report.set_size(total * SECTOR);
        for f in &self.files {
            let key = match f.kind {
                FileKind::Data { node, .. } => node,
                FileKind::Catalog { .. } => CATALOG,
                _ => continue,
            };
            if let Some(list) = self.extents.get(&key) {
                for &(block, len) in list {
                    report.push_extent(&f.path, Extent::new(u64::from(block) * SECTOR, len));
                }
            }
        }
        for warning in &self.warnings {
            report.push_warning(warning.clone());
        }
        let preserve = if self.opts.rock_ridge() {
            self.opts.preserve()
        } else {
            Preserve::empty()
        };
        let metas = || {
            self.dirs
                .iter()
                .map(|d| &d.meta)
                .chain(self.files.iter().map(|f| &f.meta))
        };
        type Loss = (Field, bool, fn(&SetAttr) -> bool);
        let losses: [Loss; 5] = [
            (Field::Created, !preserve.contains(Preserve::TIMES), |m| {
                m.created().is_some()
            }),
            (Field::Accessed, !preserve.contains(Preserve::TIMES), |m| {
                m.accessed().is_some()
            }),
            (
                Field::Permissions,
                !preserve.contains(Preserve::PERMISSIONS),
                |m| m.permissions().is_some(),
            ),
            (Field::Owner, !preserve.contains(Preserve::OWNERS), |m| {
                m.owner().is_some()
            }),
            (Field::Attributes, true, |m| {
                m.attributes().is_some_and(|a| !a.is_empty())
            }),
        ];
        for (field, lost, set) in losses {
            let count = if lost {
                metas().filter(|meta| set(meta)).count()
            } else {
                0
            };
            if count > 0 {
                let message = match (field, self.rock_ridge) {
                    (Field::Attributes, _) => "iso 9660 stores no attributes",
                    (_, false) => "needs rock ridge",
                    (_, true) => "not in the rock ridge preserve set",
                };
                report.push_warning(
                    Warning::new(WarningKind::Dropped(field), message).with_count(count as u64),
                );
            }
        }
        let keeps_names = self.rock_ridge
            || self
                .trees
                .iter()
                .any(|(tree, _)| matches!(tree, TreeKind::Joliet(_) | TreeKind::Enhanced));
        if !keeps_names {
            let rules = self.trees[0].1;
            let changed = self
                .files
                .iter()
                .filter(|f| !matches!(f.kind, FileKind::Catalog { .. }))
                .map(|f| (&f.path, &f.name, rules.file(&f.name)))
                .chain(
                    self.dirs
                        .iter()
                        .skip(1)
                        .map(|d| (&d.path, &d.name, rules.directory(&d.name))),
                )
                .filter(|(_, name, stored)| !names::primary_keeps(name, stored))
                .map(|(path, _, stored)| (path.clone(), stored))
                .collect::<BTreeMap<_, _>>();
            for (path, stored) in changed {
                report.push_warning(
                    Warning::new(WarningKind::Renamed, "iso 9660 name")
                        .with_path(path)
                        .with_stored_as(stored),
                );
            }
        }
        if self
            .trees
            .iter()
            .any(|(tree, _)| matches!(tree, TreeKind::Joliet(_)))
        {
            self.joliet_warnings(&mut report);
        }
        report
    }

    /// Warns about names the Joliet tree changes, and about names that
    /// differ from a sibling's only in case, which case-insensitive readers
    /// cannot tell apart.
    fn joliet_warnings(&self, report: &mut Report) {
        for (index, dir) in self.dirs.iter().enumerate() {
            if Some(index) == self.rr_moved {
                continue;
            }
            let children = dir
                .dirs
                .iter()
                .map(|&child| (&self.dirs[child].name, &self.dirs[child].path))
                .chain(
                    dir.files
                        .iter()
                        .map(|&file| &self.files[file])
                        .filter(|f| !matches!(f.kind, FileKind::Catalog { .. }))
                        .map(|f| (&f.name, &f.path)),
                );
            let mut folded = BTreeMap::new();
            for (name, path) in children {
                let joliet = String::from_utf16_lossy(
                    &names::convert_joliet(name)
                        .chunks_exact(2)
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>(),
                );
                if let Some(reason) = names::joliet_change(name) {
                    report.push_warning(
                        Warning::new(WarningKind::Renamed, reason)
                            .with_path(path)
                            .with_stored_as(&joliet),
                    );
                }
                if folded.insert(joliet.to_lowercase(), path).is_some() {
                    report.push_warning(
                        Warning::new(
                            WarningKind::Renamed,
                            "the Joliet name differs from a sibling's only in case; case-insensitive readers see one of them",
                        )
                        .with_path(path),
                    );
                }
            }
        }
    }

    /// A deterministic GUID from `key`, the tree and the seed, or the time
    /// without one, so the same inputs always give the same GPT and
    /// different trees different GUIDs.
    fn guid(&self, key: &str) -> Guid {
        let seed = self.opts.seed().unwrap_or(self.now.unix_seconds() as u64);
        guid(key, seed, self.tree.fingerprint())
    }
}

/// A deterministic GUID from `key`, the seed and the tree's fingerprint.
fn guid(key: &str, seed: u64, tree: u64) -> Guid {
    let mut hash1: u64 = 0xcbf2_9ce4_8422_2325;
    let mut hash2: u64 = 0x0000_0100_0000_01b3;
    for byte in key
        .bytes()
        .chain(seed.to_le_bytes())
        .chain(tree.to_le_bytes())
    {
        hash1 ^= u64::from(byte);
        hash1 = hash1.wrapping_mul(0x0000_0100_0000_01b3);
        hash2 ^= u64::from(byte);
        hash2 = hash2.wrapping_mul(0xcbf2_9ce4_8422_2325);
    }
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hash1.to_le_bytes());
    bytes[8..].copy_from_slice(&hash2.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Guid::from_bytes(bytes)
}

/// A directory record before it is written.
#[derive(Debug, Clone)]
struct PendingRecord {
    name: Vec<u8>,
    split: SplitSu,
    extent: (u32, u32),
    flags: FileFlags,
    time: DirDateTime,
}

impl PendingRecord {
    fn build(&self) -> PlanResult<DirectoryRecord> {
        let mut record = DirectoryRecord::new(&self.name, &self.split.inline)
            .ok_or(invalid(Detail::DirectoryRecord))?;
        let header = record.header_mut();
        header.extent = U32Both::new(self.extent.0);
        header.data_len = U32Both::new(self.extent.1);
        header.date_time = self.time;
        header.flags = self.flags.bits();
        header.volume_sequence_number = U16Both::new(1);
        Ok(record)
    }

    fn len(&self) -> u64 {
        let su_start = (33 + self.name.len() + 1) & !1;
        ((su_start + self.split.inline.len() + 1) & !1) as u64
    }
}

/// Gives names that repeat in a directory a `_n` suffix; the records of one
/// multi-extent file keep one name.
fn dedup(records: &mut [PendingRecord], rules: Rules) {
    let mut seen = BTreeSet::new();
    let mut start = 0;
    while start < records.len() {
        if matches!(records[start].name[..], [0] | [1]) {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < records.len() && records[end - 1].flags.contains(FileFlags::NOT_FINAL) {
            end += 1;
        }
        let original = records[start].name.clone();
        let unique = if seen.contains(&original) {
            let mut n = 1;
            loop {
                let candidate = rules.dedup(&original, n);
                n += 1;
                if !seen.contains(&candidate) {
                    break candidate;
                }
            }
        } else {
            original
        };
        seen.insert(unique.clone());
        for record in &mut records[start..end] {
            record.name = unique.clone();
        }
        start = end;
    }
}

fn overflow_len(records: &[PendingRecord]) -> u64 {
    records
        .iter()
        .filter(|r| r.split.has_overflow())
        .map(|r| r.split.overflow.len() as u64)
        .sum()
}

/// The first sector and the sector count of `records` written from byte
/// `pos`: a record that does not fit a sector starts the next one.
fn layout_records(pos: u64, records: &[PendingRecord]) -> (u64, u64) {
    let start = pos.div_ceil(SECTOR) * SECTOR;
    let mut at = start;
    for record in records {
        let len = record.len();
        let remaining = SECTOR - at % SECTOR;
        if len > remaining {
            at += remaining;
        }
        at += len;
    }
    let end = at.div_ceil(SECTOR) * SECTOR;
    (start / SECTOR, (end - start) / SECTOR)
}

/// The bytes of a directory at `block` of `size` bytes, followed by the
/// continuation area its `CE` entries point at.
fn emit_records(block: u32, size: u32, records: &mut [PendingRecord]) -> PlanResult<Vec<u8>> {
    let ca_block = block + size / SECTOR_SIZE as u32;
    let mut offset = 0u32;
    for record in records.iter_mut() {
        if record.split.has_overflow() {
            record.split.patch_ce(ca_block, offset);
            offset += record.split.overflow.len() as u32;
        }
    }
    let mut out = Vec::with_capacity(size as usize);
    for record in records.iter() {
        let bytes = record.build()?;
        let remaining = SECTOR_SIZE - out.len() % SECTOR_SIZE;
        if bytes.len() > remaining {
            out.resize(out.len() + remaining, 0);
        }
        out.extend_from_slice(bytes.as_bytes());
    }
    out.resize(size as usize, 0);
    for record in records.iter() {
        out.extend_from_slice(&record.split.overflow);
    }
    Ok(out)
}
