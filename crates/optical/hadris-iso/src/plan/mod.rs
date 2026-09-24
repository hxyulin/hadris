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

use hadris_fs::tree::{NodeKind, Tree, TreeNode, Warning, WarningKind};
use hadris_fs::{Clock, DateTime, DeviceKind, DeviceNumber, ErrorKind, Extent, SetMetadata};
use hadris_part::gpt::types as part_types;
use hadris_part::{
    Disk, Gpt, GptEntry, Guid, Hybrid, HybridMbr, Mbr, MbrEntry, MbrType, PartitionFlags,
    PartitionName, TableError,
};
use hadris_storage::BlockSize;

use crate::boot::Platform;
use crate::error::{Detail, Error};
use crate::namespace::JolietLevel;
use crate::options::{
    BootInfo, Charset, HybridBoot, IsoLevel, IsoOptions, PartitionScheme, Preserve, Relocation,
};
use crate::raw::{
    self, BootRecordVolumeDescriptor, DecDateTime, DirDateTime, DirectoryRecord, FileFlags, IsoStr,
    PathTableHeader, PrimaryVolumeDescriptor, RootDirectoryRecord, SECTOR_SIZE,
    SupplementaryVolumeDescriptor, U16Both, U32Be, U32Both, U32Le, VolumeDescriptorHeader,
    VolumeDescriptorSetTerminator,
};
use crate::report::{Report, normalize};
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
const APPLICATION: &str = "HADRIS-ISO";
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
}

impl Region {
    pub(crate) fn block(&self) -> u64 {
        match self {
            Self::Bytes { block, .. } | Self::File { block, .. } => *block,
        }
    }

    pub(crate) fn blocks(&self) -> u64 {
        let len = match self {
            Self::Bytes { data, .. } => data.len() as u64,
            Self::File { len, .. } => *len,
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
        kind: DeviceKind,
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
    meta: SetMetadata,
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

    fn len(&self) -> u64 {
        match self.kind {
            FileKind::Data { len, .. } | FileKind::Catalog { len } => len,
            _ => 0,
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
    meta: SetMetadata,
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

struct Planner<'a, C> {
    tree: &'a Tree,
    opts: &'a IsoOptions<C>,
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

fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        alloc::format!("/{name}")
    } else {
        alloc::format!("{parent}/{name}")
    }
}

/// Lays `tree` out as `opts` says, from `base`.
pub(crate) fn plan<C: Clock>(
    tree: &Tree,
    opts: &IsoOptions<C>,
    contents: &BTreeMap<usize, ContentInfo>,
    base: Base,
) -> PlanResult<Plan> {
    let rock_ridge = opts.rock_ridge().is_some();
    let case = opts.name_case();
    let level = opts.level();
    let mut trees = vec![(TreeKind::Primary, Rules::Primary { level, case })];
    if opts.has_enhanced_tree() {
        trees.push((TreeKind::Enhanced, Rules::Enhanced));
    }
    if let Some(level) = opts.joliet() {
        trees.push((TreeKind::Joliet(level), Rules::Joliet));
    }
    let mut planner = Planner {
        tree,
        opts,
        contents,
        base,
        now: opts.clock().now(),
        dirs: Vec::new(),
        files: Vec::new(),
        trees,
        rock_ridge,
        rr_moved: None,
        serials: BTreeMap::new(),
        warnings: Vec::new(),
        extents: BTreeMap::new(),
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

impl<C: Clock> Planner<'_, C> {
    // -----------------------------------------------------------------------
    // Gathering the tree.

    fn gather(&mut self) -> PlanResult<()> {
        let root = self.tree.root();
        self.dirs.push(PDir {
            name: String::new(),
            iso_name: String::new(),
            path: String::from("/"),
            meta: *root.metadata(),
            parent: 0,
            dirs: Vec::new(),
            files: Vec::new(),
            moved_to: None,
            placeholders: Vec::new(),
            physical: Vec::new(),
            serial: 0,
        });
        let mut pending = vec![(0usize, root)];
        let mut skipped = 0usize;
        let mut first_skipped = None;
        while let Some((dir, node)) = pending.pop() {
            let path = self.dirs[dir].path.clone();
            let mut children = Vec::new();
            for (name, child) in node.children() {
                let child_path = join(&path, name);
                let kind = match child.kind() {
                    NodeKind::Dir => {
                        let id = self.dirs.len();
                        self.dirs.push(PDir {
                            name: name.to_string(),
                            iso_name: name.to_string(),
                            path: child_path,
                            meta: *child.metadata(),
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
                    NodeKind::File(content) => {
                        let len = match self.contents.get(&child.id()) {
                            Some(info) => info.len,
                            None => content.len().unwrap_or(0),
                        };
                        if len >= MAX_EXTENT && !matches!(self.opts.level(), IsoLevel::L3) {
                            return Err(Detail::ImageTooLarge.error(ErrorKind::FileTooLarge));
                        }
                        FileKind::Data {
                            node: child.id(),
                            len,
                        }
                    }
                    NodeKind::Symlink(_) if self.rock_ridge => {
                        FileKind::Symlink { node: child.id() }
                    }
                    NodeKind::Device(kind, number) if self.rock_ridge => FileKind::Device {
                        node: child.id(),
                        kind,
                        number,
                    },
                    _ => {
                        skipped += 1;
                        first_skipped.get_or_insert(child_path);
                        continue;
                    }
                };
                let id = self.files.len();
                self.files.push(PFile {
                    name: name.to_string(),
                    path: child_path,
                    kind,
                    meta: *child.metadata(),
                    links: u32::try_from(child.links()).unwrap_or(u32::MAX),
                });
                self.dirs[dir].files.push(id);
            }
            for child in children.into_iter().rev() {
                pending.push(child);
            }
        }
        if let Some(path) = first_skipped {
            self.warnings.push(Warning::new(
                path,
                WarningKind::Skipped,
                alloc::format!(
                    "{skipped} symlink or device entries need Rock Ridge and were left out"
                ),
            ));
        }
        self.dirs
            .iter_mut()
            .for_each(|dir| dir.physical = dir.dirs.clone());
        Ok(())
    }

    /// Checks the boot options against the tree before anything is laid
    /// out: boot images exist and fit their emulation, a load size is not
    /// zero, and the MBR boot code fits. Warns about load sizes past the
    /// image and a hybrid table that gets no EFI system partition.
    fn check_boot(&mut self) -> PlanResult<()> {
        if let Some(code) = self.opts.hybrid().and_then(HybridBoot::bootstrap)
            && code.len() > 446
        {
            return Err(Detail::HybridBoot.error(ErrorKind::LimitExceeded));
        }
        let Some(el_torito) = self.opts.el_torito() else {
            return Ok(());
        };
        for entry in el_torito.entries() {
            let node = self
                .tree
                .get(entry.image())
                .filter(|node| matches!(node.kind(), NodeKind::File(_)))
                .ok_or(invalid(Detail::BootImage))?;
            let len = self.contents.get(&node.id()).map_or(0, |info| info.len);
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
                self.warnings.push(Warning::new(
                    normalize(entry.image()),
                    WarningKind::IgnoredMetadata,
                    alloc::format!(
                        "the load size of {load} sectors runs past the {len}-byte boot image; firmware loads the bytes after it too"
                    ),
                ));
            }
        }
        if let Some(hybrid) = self.opts.hybrid()
            && hybrid.scheme() != PartitionScheme::Mbr
            && hybrid.efi_partition().is_none()
        {
            let mut uefi = el_torito
                .entries()
                .iter()
                .skip(1)
                .filter(|entry| entry.platform() == Platform::Efi);
            if let (Some(first), Some(_)) = (uefi.next(), uefi.next()) {
                self.warnings.push(Warning::new(
                    normalize(first.image()),
                    WarningKind::Skipped,
                    "several UEFI boot entries and no HybridBoot::with_efi_partition: the partition table has no EFI system partition",
                ));
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
            .get(&self.dirs[parent].path)
            .ok_or(invalid(Detail::CatalogPath))?;
        if node.child(name).is_some() {
            return Err(invalid(Detail::CatalogPath));
        }
        let first_dir = node
            .children()
            .position(|(_, child)| matches!(child.kind(), NodeKind::Dir));
        let insert_at = first_dir.unwrap_or(0).saturating_sub(1);
        let files_before = node
            .children()
            .take(insert_at)
            .filter(|(child_name, child)| {
                !matches!(child.kind(), NodeKind::Dir)
                    && self.dirs[parent]
                        .files
                        .iter()
                        .any(|&f| self.files[f].name == *child_name)
            })
            .count();
        let entries = el_torito.entries().len().saturating_sub(1);
        let len = ((96 + entries * 64) as u64).div_ceil(SECTOR) * SECTOR;
        let id = self.files.len();
        self.files.push(PFile {
            name: name.to_string(),
            path: path.clone(),
            kind: FileKind::Catalog { len },
            meta: SetMetadata::new(),
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
        self.opts.rock_ridge().map(|rr| *rr.relocation())
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
                    meta: SetMetadata::new(),
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

    fn record_time(&self, meta: &SetMetadata) -> DirDateTime {
        DirDateTime::from_datetime(meta.times().modified().unwrap_or(self.now))
    }

    fn rr_time(time: DateTime) -> [u8; 7] {
        DirDateTime::from_datetime(time).to_bytes()
    }

    /// `PX` and `TF` for a node.
    fn posix(
        &self,
        builder: &mut SuBuilder,
        meta: &SetMetadata,
        type_mode: u32,
        default: u32,
        links: u32,
        serial: u32,
    ) {
        let preserve = self
            .opts
            .rock_ridge()
            .map_or(Preserve::empty(), |rr| rr.preserve());
        let mode = if preserve.contains(Preserve::PERMISSIONS) {
            meta.mode().map_or(default, |mode| mode.bits())
        } else {
            default
        };
        let (uid, gid) = if preserve.contains(Preserve::OWNERS) {
            (meta.uid().unwrap_or(0), meta.gid().unwrap_or(0))
        } else {
            (0, 0)
        };
        builder.px(type_mode | mode, links, uid, gid, serial);
        if preserve.contains(Preserve::TIMES) {
            let times = meta.times();
            let modified = Self::rr_time(times.modified().unwrap_or(self.now));
            let accessed = Self::rr_time(times.accessed().unwrap_or(self.now));
            builder.tf(times.created().map(Self::rr_time), modified, accessed);
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
                        kind: DeviceKind::Block,
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
                    FileKind::Symlink { node } => {
                        if let Some(NodeKind::Symlink(target)) =
                            self.tree.get(&f.path).map(|n: TreeNode<'_>| n.kind())
                        {
                            let _ = node;
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

        let infos = self.info_tables()?;
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
            hybrid.map(HybridBoot::scheme),
            Some(PartitionScheme::Gpt | PartitionScheme::Hybrid)
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
        let node = self
            .tree
            .get(path)
            .filter(|node| matches!(node.kind(), NodeKind::File(_)))
            .ok_or(invalid(Detail::BootImage))?;
        let len = self.contents.get(&node.id()).map_or(0, |info| info.len);
        let block = self
            .extents
            .get(&node.id())
            .and_then(|extents| extents.first())
            .map_or(0, |extent| extent.0);
        Ok((block, len))
    }

    fn info_tables(&self) -> PlanResult<BTreeMap<usize, InfoTable>> {
        let mut out = BTreeMap::new();
        let Some(el_torito) = self.opts.el_torito() else {
            return Ok(out);
        };
        for entry in el_torito.entries() {
            if entry.boot_info_table() == BootInfo::None {
                continue;
            }
            let (block, len) = self.boot_image(entry.image())?;
            if len < 64 {
                return Err(invalid(Detail::BootInfoTable));
            }
            let node = self
                .tree
                .get(entry.image())
                .map(|node| node.id())
                .ok_or(invalid(Detail::BootImage))?;
            out.insert(
                node,
                InfoTable {
                    kind: entry.boot_info_table(),
                    block,
                    len: u32::try_from(len).map_err(|_| invalid(Detail::BootInfoTable))?,
                },
            );
        }
        Ok(out)
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
            let (block, len) = self.boot_image(entry.image())?;
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

    fn identifier<const N: usize>(&self, text: &str, d_chars: bool) -> PlanResult<IsoStr<N>> {
        if text.len() > N {
            return Err(invalid(Detail::Identifier));
        }
        let mut bytes = text.as_bytes().to_vec();
        if self.opts.charset() == Charset::Strict {
            for byte in &mut bytes {
                if byte.is_ascii_lowercase() {
                    *byte = byte.to_ascii_uppercase();
                } else if !(byte.is_ascii_uppercase()
                    || byte.is_ascii_digit()
                    || *byte == b'_'
                    || (!d_chars && b" !\"%$'()*+,-./:;<=>?".contains(byte)))
                {
                    *byte = b'_';
                }
            }
        }
        IsoStr::padded(&bytes).ok_or(invalid(Detail::Identifier))
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
        let ids = self.opts.volume();
        let now = DecDateTime::from_datetime(self.now);
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
            d.creation_date = now;
            d.modification_date = now;
            d.expiration_date = DecDateTime::UNSPECIFIED;
            d.effective_date = DecDateTime::UNSPECIFIED;
            d.file_structure_version = 1;
            match tree {
                TreeKind::Joliet(level) => {
                    let mut s: SupplementaryVolumeDescriptor = bytemuck::cast(d);
                    s.header = VolumeDescriptorHeader::new(raw::DescriptorType::Supplementary);
                    s.escape_sequences = level.escape_sequences();
                    s.system_identifier = Self::ucs2(ids.system().unwrap_or(""));
                    s.volume_identifier = Self::ucs2(ids.volume());
                    s.volume_set_identifier = Self::ucs2(ids.volume_set().unwrap_or(""));
                    s.publisher_identifier = Self::ucs2(ids.publisher().unwrap_or(""));
                    s.preparer_identifier = Self::ucs2(ids.preparer().unwrap_or(""));
                    s.application_identifier = Self::ucs2(ids.application().unwrap_or(APPLICATION));
                    s.copyright_file_identifier = Self::ucs2("");
                    s.abstract_file_identifier = Self::ucs2("");
                    s.bibliographic_file_identifier = Self::ucs2("");
                    out.extend_from_slice(bytemuck::bytes_of(&s));
                }
                _ => {
                    d.system_identifier = self.identifier(ids.system().unwrap_or(""), false)?;
                    d.volume_identifier = self.identifier(ids.volume(), true)?;
                    d.volume_set_identifier =
                        self.identifier(ids.volume_set().unwrap_or(""), true)?;
                    d.publisher_identifier =
                        self.identifier(ids.publisher().unwrap_or(""), false)?;
                    d.preparer_identifier = self.identifier(ids.preparer().unwrap_or(""), false)?;
                    d.application_identifier =
                        self.identifier(ids.application().unwrap_or(APPLICATION), false)?;
                    d.copyright_file_identifier = IsoStr::empty();
                    d.abstract_file_identifier = IsoStr::empty();
                    d.bibliographic_file_identifier = IsoStr::empty();
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

    fn efi_image(&self, hybrid: &HybridBoot) -> Option<String> {
        if let Some(path) = hybrid.efi_partition() {
            return Some(path.to_string());
        }
        let el_torito = self.opts.el_torito()?;
        let mut uefi = el_torito
            .entries()
            .iter()
            .skip(1)
            .filter(|entry| entry.platform() == Platform::Efi);
        let first = uefi.next()?;
        uefi.next().is_none().then(|| first.image().to_string())
    }

    fn gpt(
        &self,
        hybrid: &HybridBoot,
        end: u64,
        total: u64,
    ) -> PlanResult<(Gpt, Option<usize>, Option<usize>)> {
        let iso_512 = end * 4;
        let total_512 = total * 4;
        let volume = self.opts.volume().volume();
        let mut gpt = Gpt::new(
            guid(&alloc::format!("disk-{volume}")),
            total_512,
            PART_BLOCK,
        )
        .map_err(part_error)?;
        let start = gpt.first_usable().max(ISO_DATA_START_512);
        let iso_end = iso_512.saturating_sub(1);
        if iso_end <= start {
            return Err(invalid(Detail::HybridBoot));
        }
        let esp = match self.efi_image(hybrid) {
            Some(path) => {
                let (block, len) = self
                    .boot_image(&path)
                    .map_err(|_| invalid(Detail::HybridBoot))?;
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
                Ok(GptEntry::new(type_guid, guid(key), first, last - first + 1).with_name(name))
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

    fn partition_disk(&self, hybrid: &HybridBoot, end: u64, total: u64) -> PlanResult<Disk> {
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
            PartitionScheme::Gpt => Disk::new(self.gpt(hybrid, end, total)?.0),
            PartitionScheme::Hybrid => {
                let (gpt, iso, esp) = self.gpt(hybrid, end, total)?;
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
                Disk::new(Hybrid::new(gpt, &config).map_err(part_error)?)
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
        let mut extents = BTreeMap::new();
        for f in &self.files {
            let key = match f.kind {
                FileKind::Data { node, .. } => node,
                FileKind::Catalog { .. } => CATALOG,
                _ => continue,
            };
            if let Some(list) = self.extents.get(&key)
                && let Some(&(block, _)) = list.first()
            {
                extents.insert(
                    f.path.clone(),
                    Extent::new(u64::from(block) * SECTOR, f.len()),
                );
            }
        }
        let mut warnings = self.warnings.clone();
        if !self.rock_ridge {
            let dropped = self
                .dirs
                .iter()
                .map(|d| &d.meta)
                .chain(self.files.iter().map(|f| &f.meta))
                .filter(|meta| {
                    meta.mode().is_some() || meta.uid().is_some() || meta.gid().is_some()
                })
                .count();
            if dropped > 0 {
                warnings.push(Warning::new(
                    "/",
                    WarningKind::IgnoredMetadata,
                    alloc::format!(
                        "permissions and owners of {dropped} entries need Rock Ridge and were not stored"
                    ),
                ));
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
                .map(|f| (&f.path, names::primary_keeps(&f.name, &rules.file(&f.name))))
                .chain(self.dirs.iter().skip(1).map(|d| {
                    (
                        &d.path,
                        names::primary_keeps(&d.name, &rules.directory(&d.name)),
                    )
                }))
                .filter(|(_, kept)| !kept)
                .map(|(path, _)| path.clone())
                .collect::<BTreeSet<_>>();
            for path in changed {
                warnings.push(Warning::new(
                    path,
                    WarningKind::Renamed,
                    "the name does not fit the primary tree and was shortened",
                ));
            }
        }
        if self
            .trees
            .iter()
            .any(|(tree, _)| matches!(tree, TreeKind::Joliet(_)))
        {
            self.joliet_warnings(&mut warnings);
        }
        Report::new(total, extents, warnings)
    }

    /// Warns about names the Joliet tree changes, and about names that
    /// differ from a sibling's only in case, which case-insensitive readers
    /// cannot tell apart.
    fn joliet_warnings(&self, warnings: &mut Vec<Warning>) {
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
                if let Some(reason) = names::joliet_change(name) {
                    warnings.push(Warning::new(path.clone(), WarningKind::Renamed, reason));
                }
                let key = String::from_utf16_lossy(
                    &names::convert_joliet(name)
                        .chunks_exact(2)
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>(),
                )
                .to_lowercase();
                if let Some(first) = folded.insert(key, path) {
                    warnings.push(Warning::new(
                        path.clone(),
                        WarningKind::Renamed,
                        alloc::format!(
                            "the Joliet name differs from {first} only in case; case-insensitive readers see one of them"
                        ),
                    ));
                }
            }
        }
    }
}

/// A deterministic GUID from `key`, so the same volume always gets the same
/// GPT.
fn guid(key: &str) -> Guid {
    let mut hash1: u64 = 0xcbf2_9ce4_8422_2325;
    let mut hash2: u64 = 0x0000_0100_0000_01b3;
    for byte in key.bytes() {
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
