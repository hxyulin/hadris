use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use hadris_fs::tree::{Content, NodeKind, Tree, Warning, WarningKind};
use hadris_fs::{
    Clock, DirCursor, ErrorKind, Extent, FileType, MountError, NameBuf, NodeId, SetMetadata,
};
use hadris_io::ErrorType;
use hadris_part::{Disk, Gpt, GptEntry, Hybrid, HybridMbr, PartitionKind, PartitionTable};
use hadris_storage::{BlockIndex, BlockSize, StorageError};

use super::image::IsoImage;
use super::storage::BlockDevice;
use super::write::{check_block_size, emit, measure};
use crate::error::{Detail, Error};
use crate::namespace::Namespace;
use crate::options::{IsoLevel, IsoOptions, RockRidge, SessionMode, VolumeIdentifiers};
use crate::plan::{self, Base, Region};
use crate::raw::{self, SECTOR_SIZE};
use crate::report::Report;

const SECTOR: u64 = SECTOR_SIZE as u64;
/// The system area, the backup GPT region after the data, and the first
/// sectors of partitions that kept their size.
type Tables512 = (Vec<u8>, Option<Vec<u8>>, Vec<u64>);
/// Sectors of a kept boot catalog read to patch its entries.
const CATALOG_SECTORS: u64 = 8;
/// The largest extent a stored file keeps: a multiple of the block size.
const MAX_EXTENT: u64 = (u32::MAX as u64 / SECTOR) * SECTOR;

fn never<E>(err: Error<core::convert::Infallible>) -> Error<E> {
    err.map_device(|never| match never {})
}

fn text<const N: usize>(field: &raw::IsoStr<N>) -> Option<String> {
    let bytes = field.trimmed();
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(bytes).into_owned())
}

fn set_metadata(meta: &hadris_fs::Metadata) -> SetMetadata {
    let set = SetMetadata::new()
        .with_times(meta.times())
        .with_mode(meta.permissions());
    match meta.owner() {
        Some((uid, gid)) => set.with_uid(uid).with_gid(gid),
        None => set,
    }
}

/// A 512-byte-block window on a device, for reading partition tables.
struct Sectors<'a, D> {
    dev: &'a mut D,
    len: u64,
}

impl<D: BlockDevice> ErrorType for Sectors<'_, D> {
    type Error = StorageError<D::Error>;
}

io_transform! {

impl<D: BlockDevice> BlockDevice for Sectors<'_, D> {
    fn block_size(&self) -> BlockSize {
        const { BlockSize::new(512).unwrap() }
    }

    fn block_count(&self) -> u64 {
        self.len / 512
    }

    async fn read_blocks(&mut self, first: BlockIndex, buf: &mut [u8]) -> Result<(), Self::Error> {
        let offset = first.get().checked_mul(512).ok_or(StorageError::OutOfRange)?;
        if offset.saturating_add(buf.len() as u64) > self.len {
            return Err(StorageError::OutOfRange);
        }
        let len = self.len;
        super::image::read_bytes(self.dev, len, offset, buf).await.map_err(|err| {
            match err.into_device_error() {
                Some(err) => StorageError::Device(err),
                None => StorageError::OutOfRange,
            }
        })
    }
}

/// An existing image read into a [`Tree`] to be written back.
///
/// Every file of the image becomes a tree entry whose content is its
/// extents on the device ([`Content::stored`]), with Rock Ridge names,
/// modes, owners, times, symlinks, device nodes and hard links when the
/// image has them, Joliet or enhanced names otherwise. Change the tree, then
/// [`write`](Self::write) it back: unchanged files keep their extents, new
/// content goes after the old data.
///
/// ```rust,ignore
/// let mut session = Session::open(&mut dev)?;
/// session.tree_mut().add_file("new.txt", Content::bytes("hi"))?;
/// session.tree_mut().remove("old.txt")?;
/// let report = session.write(&session.options(), SessionMode::Append)?;
/// ```
#[derive(Debug)]
pub struct Session<D> {
    dev: D,
    tree: Tree,
    volume_blocks: u64,
    catalog: Option<u32>,
    /// The byte offset in the kept catalog of each entry whose boot image
    /// is a file of the tree, and that file's path.
    boot: Vec<(usize, String)>,
    options: IsoOptions,
    warnings: Vec<Warning>,
}

impl<D: BlockDevice> Session<D> {
    /// Reads the image on `dev` into a tree.
    ///
    /// Reads the newest descriptor set, at logical sector 16, and walks the
    /// most capable tree. Fails like `IsoImage::open`, and with
    /// [`ErrorKind::Unsupported`] for logical blocks other than 2048 bytes.
    pub async fn open(dev: D) -> Result<Self, MountError<D, D::Error>> {
        let mut iso = IsoImage::open(dev).await?;
        if iso.block_size() != SECTOR_SIZE as u32 {
            let err: hadris_fs::Error<D::Error> = ErrorKind::Unsupported.into();
            return Err(MountError::new(err, iso.into_inner()));
        }
        match read_session(&mut iso).await {
            Ok((tree, options, warnings)) => {
                let volume_blocks = u64::from(iso.volume_blocks());
                let catalog = iso.boot_catalog_block();
                let mut session = Self {
                    dev: iso.into_inner(),
                    tree,
                    volume_blocks,
                    catalog,
                    boot: Vec::new(),
                    options,
                    warnings,
                };
                match session.map_boot_images().await {
                    Ok(()) => Ok(session),
                    Err(err) => Err(MountError::new(err.into(), session.dev)),
                }
            }
            Err(err) => Err(MountError::new(err.into(), iso.into_inner())),
        }
    }

    /// The tree the image holds.
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The tree, for changes.
    pub fn tree_mut(&mut self) -> &mut Tree {
        &mut self.tree
    }

    /// Options that write the tree back as the image has it: its volume
    /// identifiers, Joliet level, Rock Ridge and enhanced tree, at Level 3.
    /// El Torito is left out: without it in the options, the image's boot
    /// catalog is kept, and its entries follow their boot images (see
    /// [`write`](Self::write)).
    pub fn options(&self) -> IsoOptions {
        self.options.clone()
    }

    /// Entries of the image the tree could not hold, such as FIFOs.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// The number of blocks the image's volume descriptors declare.
    pub fn volume_blocks(&self) -> u64 {
        self.volume_blocks
    }

    /// Writes the tree back as `mode` says and returns the [`Report`].
    ///
    /// Without El Torito in `opts`, the image's boot catalog is kept, and
    /// its entries follow the tree paths of their boot images: an entry
    /// whose image was replaced points at the new content, in place in the
    /// old catalog, and one whose image was removed fails with
    /// [`ErrorKind::InvalidInput`] and [`Detail::BootImage`]. Boot
    /// information tables in replaced images are not written. With El
    /// Torito in `opts`, a new catalog is written.
    ///
    /// New data goes after the old volume and after every partition of
    /// the image's partition table, so partitions appended after the ISO
    /// data (`xorriso -append_partition`) are kept. The system area is
    /// written only by [`SessionMode::Rewrite`]: with hybrid boot in `opts`,
    /// new partition tables replace the old ones; without, the old tables
    /// are kept and their partitions that ended with the old volume grow
    /// with it, unless that would overlap another partition.
    /// [`SessionMode::Append`] never writes the system area; it warns when
    /// `opts` has hybrid boot or the image has partition tables, which
    /// then still describe the previous session.
    ///
    /// Afterwards the tree's new files point at their new extents, so the
    /// session can be written again.
    pub async fn write<C: Clock>(&mut self, opts: &IsoOptions<C>, mode: SessionMode) -> Result<Report, Error<D::Error>> {
        check_block_size(&self.dev)?;
        let contents = measure(&self.tree, true).await.map_err(never)?;
        let old = self.volume_blocks;
        let existing = self.partition_tables(old).await?;
        let has_tables = existing.is_some();
        let mut floor = existing.as_ref().map_or(old, |tables| tables.floor.max(old));
        if mode == SessionMode::Append {
            floor = floor.max(existing.as_ref().map_or(0, |tables| tables.backup_end.div_ceil(4)));
        }
        let tables = match (mode, opts.hybrid()) {
            (SessionMode::Rewrite, None) => existing,
            _ => None,
        };
        let keep_catalog = match opts.el_torito() {
            Some(_) => None,
            None => self.catalog,
        };
        if keep_catalog.is_some() {
            self.check_boot_images()?;
        }
        let base = match mode {
            SessionMode::Append => Base {
                descriptors: floor + u64::from(raw::DESCRIPTOR_START),
                first_block: floor,
                fill_gaps: false,
                system_area: false,
                keep_catalog,
                gpt_backup: false,
            },
            _ => Base {
                descriptors: u64::from(raw::DESCRIPTOR_START),
                first_block: floor,
                fill_gaps: false,
                system_area: opts.hybrid().is_some(),
                keep_catalog,
                gpt_backup: tables.as_ref().is_some_and(|tables| tables.gpt),
            },
        };
        let descriptors = base.descriptors;
        let mut plan = plan::plan(&self.tree, opts, &contents, base).map_err(never)?;
        if mode == SessionMode::Append {
            let copy = plan.regions.iter().find_map(|region| match region {
                Region::Bytes { block, data } if *block == descriptors => Some(data.clone()),
                _ => None,
            });
            if let Some(data) = copy {
                plan.regions.push(Region::Bytes {
                    block: u64::from(raw::DESCRIPTOR_START),
                    data,
                });
            }
            if opts.hybrid().is_some() {
                plan.report.warn(Warning::new(
                    "/",
                    WarningKind::IgnoredMetadata,
                    "an appended session does not write the system area; the hybrid boot options were not applied",
                ));
            }
            if has_tables {
                plan.report.warn(Warning::new(
                    "/",
                    WarningKind::IgnoredMetadata,
                    "the partition tables still describe the previous session",
                ));
            }
        }
        if let Some(block) = keep_catalog
            && let Some(region) = self.patched_catalog(block, &plan.report).await?
        {
            plan.regions.push(region);
        }
        if let Some(tables) = tables {
            let (system, tail, kept) = self.updated_tables(tables, plan.end_blocks, plan.total_blocks).await?;
            plan.regions.push(Region::Bytes { block: 0, data: system });
            if let Some(tail) = tail {
                plan.regions.push(Region::Bytes {
                    block: plan.end_blocks,
                    data: tail,
                });
            }
            for start in kept {
                plan.report.warn(Warning::new(
                    "/",
                    WarningKind::IgnoredMetadata,
                    alloc::format!(
                        "the partition at 512-byte sector {start} keeps its size: growing it over the new data would overlap another partition"
                    ),
                ));
            }
        }
        plan.regions.sort_by_key(Region::block);
        emit(&mut self.dev, &self.tree, &plan).await?;
        for warning in &self.warnings {
            plan.report.warn(warning.clone());
        }
        self.volume_blocks = plan.total_blocks;
        if keep_catalog.is_none() {
            self.catalog = None;
            self.boot.clear();
        }
        self.store_new_files(&plan.report, &contents);
        Ok(plan.report)
    }

    /// Returns the device.
    pub fn into_inner(self) -> D {
        self.dev
    }

    fn store_new_files(&mut self, report: &Report, contents: &BTreeMap<usize, plan::ContentInfo>) {
        let mut paths = Vec::new();
        let mut pending = vec![(String::new(), self.tree.root())];
        while let Some((prefix, dir)) = pending.pop() {
            for (name, child) in dir.children() {
                let path = alloc::format!("{prefix}/{name}");
                if child.file_type() == FileType::Dir {
                    pending.push((path, child));
                } else if contents.get(&child.id()).is_some_and(|info| info.stored.is_none() && info.len > 0) {
                    paths.push(path);
                }
            }
        }
        for path in paths {
            let Some(extent) = report.extent_of(&path) else {
                continue;
            };
            let mut extents = Vec::new();
            let mut offset = extent.offset();
            let mut left = extent.len();
            while left > 0 {
                let len = left.min(MAX_EXTENT);
                extents.push(Extent::new(offset, len));
                offset += len;
                left -= len;
            }
            if let Some(content) = self.tree.content_mut(&path) {
                *content = Content::stored(extents);
            }
        }
    }

    /// Records, for each entry of the kept boot catalog, the tree path of
    /// its boot image: the file whose first extent it loads.
    async fn map_boot_images(&mut self) -> Result<(), Error<D::Error>> {
        self.boot.clear();
        let Some(block) = self.catalog else {
            return Ok(());
        };
        let bytes = self.read_catalog(block).await?;
        let entries = catalog_entries(&bytes);
        let mut firsts = BTreeMap::new();
        let mut pending = vec![(String::new(), self.tree.root())];
        while let Some((prefix, dir)) = pending.pop() {
            for (name, child) in dir.children() {
                let path = alloc::format!("{prefix}/{name}");
                if child.file_type() == FileType::Dir {
                    pending.push((path, child));
                } else if let Some(NodeKind::File(content)) = Some(child.kind())
                    && let Some(first) = content.stored_extents().and_then(|extents| extents.first())
                {
                    firsts.entry(first.offset()).or_insert(path);
                }
            }
        }
        for (at, rba) in entries {
            if let Some(path) = firsts.get(&(u64::from(rba) * SECTOR)) {
                self.boot.push((at, path.clone()));
            }
        }
        Ok(())
    }

    /// Fails when a boot image the kept catalog loads is gone from the tree.
    fn check_boot_images<E>(&self) -> Result<(), Error<E>> {
        for (_, path) in &self.boot {
            match self.tree.get(path).map(|node| node.kind()) {
                Some(NodeKind::File(content)) if content.len() != Some(0) => {}
                _ => return Err(Error::invalid(Detail::BootImage)),
            }
        }
        Ok(())
    }

    /// The sectors of the catalog at `block` that hold its entries.
    async fn read_catalog(&mut self, block: u32) -> Result<Vec<u8>, Error<D::Error>> {
        let len = self.dev.block_count().saturating_mul(u64::from(self.dev.block_size().get()));
        let offset = u64::from(block) * SECTOR;
        let size = (CATALOG_SECTORS * SECTOR).min(len.saturating_sub(offset));
        let mut bytes = vec![0u8; size as usize];
        super::image::read_bytes(&mut self.dev, len, offset, &mut bytes).await?;
        Ok(bytes)
    }

    /// The kept catalog at `block` with each entry pointing at where its
    /// boot image is now, or `None` when none moved.
    async fn patched_catalog(&mut self, block: u32, report: &Report) -> Result<Option<Region>, Error<D::Error>> {
        if self.boot.is_empty() {
            return Ok(None);
        }
        let mut bytes = self.read_catalog(block).await?;
        let mut changed = false;
        for (at, path) in &self.boot {
            let extent = report.extent_of(path).ok_or(Error::invalid(Detail::BootImage))?;
            let rba = u32::try_from(extent.offset() / SECTOR).map_err(|_| Error::invalid(Detail::ImageTooLarge))?;
            let field = &mut bytes[at + 8..at + 12];
            if field != rba.to_le_bytes() {
                field.copy_from_slice(&rba.to_le_bytes());
                changed = true;
            }
        }
        if !changed {
            return Ok(None);
        }
        let used = self.boot.iter().map(|(at, _)| at + 32).max().unwrap_or(0);
        bytes.truncate(used.div_ceil(SECTOR_SIZE) * SECTOR_SIZE);
        Ok(Some(Region::Bytes {
            block: u64::from(block),
            data: bytes,
        }))
    }

    /// The partition table of the image, when it has one.
    async fn partition_tables(&mut self, old: u64) -> Result<Option<Tables>, Error<D::Error>> {
        let len = self.dev.block_count().saturating_mul(u64::from(self.dev.block_size().get()));
        let mut sectors = Sectors { dev: &mut self.dev, len };
        let Ok(disk) = super::part::read(&mut sectors).await else {
            return Ok(None);
        };
        let floor = disk
            .partitions()
            .map(|partition| partition.end().div_ceil(4))
            .max()
            .unwrap_or(0);
        let gpt = !matches!(disk.table(), PartitionTable::Mbr(_));
        let (iso_end, backup_end) = match disk.table() {
            PartitionTable::Gpt(gpt) => (backup_array(gpt).min(old * 4), gpt.backup_lba() + 1),
            PartitionTable::Hybrid(hybrid) => (
                backup_array(hybrid.gpt()).min(old * 4),
                hybrid.gpt().backup_lba() + 1,
            ),
            _ => (old * 4, 0),
        };
        Ok(Some(Tables { disk, floor, gpt, iso_end, backup_end }))
    }

    /// The system area with the tables moved to the new size, the backup
    /// GPT region after the data, and the first sectors of partitions that
    /// ended with the old volume but cannot grow with it.
    async fn updated_tables(
        &mut self,
        tables: Tables,
        end: u64,
        total: u64,
    ) -> Result<Tables512, Error<D::Error>> {
        let bad = |_| Error::invalid(Detail::HybridBoot);
        let new_iso_end = end * 4;
        let spans: Vec<(u64, u64)> = tables
            .disk
            .partitions()
            .map(|partition| (partition.start(), partition.end()))
            .collect();
        let ends_with_volume = |start: u64, len: u64| {
            let last = start + len;
            last <= tables.iso_end && last + 4 >= tables.iso_end
        };
        let fits = |start: u64| {
            spans
                .iter()
                .all(|&(other, other_end)| other == start || other_end <= start || other >= new_iso_end)
        };
        let kept: Vec<u64> = spans
            .iter()
            .filter(|&&(start, end)| ends_with_volume(start, end - start) && !fits(start))
            .map(|&(start, _)| start)
            .collect();
        let extends = |start: u64, len: u64| ends_with_volume(start, len) && fits(start);
        let bootstrap = *tables.disk.bootstrap();
        let mut disk = match tables.disk.into_table() {
            PartitionTable::Mbr(mut mbr) => {
                let resize: Vec<(usize, u64)> = mbr
                    .partitions()
                    .filter(|p| extends(p.start(), p.len()))
                    .map(|p| (p.index(), new_iso_end - p.start()))
                    .collect();
                for (index, len) in resize {
                    mbr.resize(index, len).map_err(bad)?;
                }
                Disk::new(mbr)
            }
            PartitionTable::Gpt(gpt) => Disk::new(regrow(&gpt, total * 4, new_iso_end, &extends).map_err(bad)?.0),
            PartitionTable::Hybrid(hybrid) => {
                let (gpt, slots) = regrow(hybrid.gpt(), total * 4, new_iso_end, &extends).map_err(bad)?;
                let mut config = HybridMbr::new();
                for mirror in hybrid.mbr_partitions() {
                    let PartitionKind::Mbr(kind) = mirror.kind() else {
                        continue;
                    };
                    if let Some(&(_, index)) = slots.iter().find(|(start, _)| *start == mirror.start()) {
                        config.add_mirrored(index, kind, mirror.flags()).map_err(bad)?;
                    }
                }
                Disk::new(Hybrid::new(gpt, &config).map_err(bad)?)
            }
            _ => return Err(Error::invalid(Detail::HybridBoot)),
        };
        disk.set_bootstrap(&bootstrap).map_err(bad)?;
        let mut system = vec![0u8; 16 * SECTOR_SIZE];
        let len = self.dev.block_count().saturating_mul(u64::from(self.dev.block_size().get()));
        super::image::read_bytes(&mut self.dev, len, 0, &mut system).await?;
        let mut tail = vec![0u8; ((total - end) * SECTOR) as usize];
        for run in disk.runs() {
            let offset = run.lba() * 512;
            let bytes = run.bytes();
            if offset + bytes.len() as u64 <= system.len() as u64 {
                system[offset as usize..offset as usize + bytes.len()].copy_from_slice(bytes);
            } else if offset >= end * SECTOR {
                let at = (offset - end * SECTOR) as usize;
                tail.get_mut(at..at + bytes.len())
                    .ok_or(Error::invalid(Detail::HybridBoot))?
                    .copy_from_slice(bytes);
            } else {
                return Err(Error::invalid(Detail::HybridBoot));
            }
        }
        Ok((system, (!tail.is_empty()).then_some(tail), kept))
    }
}

/// Reads the image's tree, the options that write it back, and what the
/// tree could not hold.
async fn read_session<D: BlockDevice>(iso: &mut IsoImage<D>) -> Result<(Tree, IsoOptions, Vec<Warning>), Error<D::Error>> {
    let pvd = iso.primary_descriptor().await?;
    let namespaces = iso.namespaces();
    let mut ids = VolumeIdentifiers::new(text(&pvd.volume_identifier).unwrap_or_default());
    if let Some(value) = text(&pvd.system_identifier) {
        ids = ids.with_system(value);
    }
    if let Some(value) = text(&pvd.volume_set_identifier) {
        ids = ids.with_volume_set(value);
    }
    if let Some(value) = text(&pvd.publisher_identifier) {
        ids = ids.with_publisher(value);
    }
    if let Some(value) = text(&pvd.preparer_identifier) {
        ids = ids.with_preparer(value);
    }
    if let Some(value) = text(&pvd.application_identifier) {
        ids = ids.with_application(value);
    }
    let mut options = IsoOptions::default().with_volume(ids).with_level(IsoLevel::L3);
    if let Some(level) = namespaces.joliet_level() {
        options = options.with_joliet(level);
    }
    if namespaces.contains(Namespace::RockRidge) {
        options = options.with_rock_ridge(RockRidge::default());
    }
    if namespaces.contains(Namespace::Enhanced) {
        options = options.with_enhanced_tree();
    }
    let mut view = iso.view(Namespace::Preferred)?;
    let mut tree = Tree::new();
    let mut warnings = Vec::new();
    let root = view.root();
    let meta = view.node_metadata(root).await?;
    tree.set_metadata("/", set_metadata(&meta)).map_err(Error::from)?;
    let mut links: BTreeMap<u64, String> = BTreeMap::new();
    let mut pending: Vec<(NodeId, String)> = vec![(root, String::new())];
    let mut name = NameBuf::new();
    while let Some((dir, prefix)) = pending.pop() {
        let mut cursor = DirCursor::start();
        while let Some(entry) = view.read_dir_entry(dir, &mut cursor, &mut name).await? {
            let path = alloc::format!("{prefix}/{}", String::from_utf8_lossy(name.as_bytes()));
            let node = entry.node();
            let meta = view.node_metadata(node).await?;
            match meta.file_type() {
                FileType::Dir => {
                    if dir == root && view.is_relocation_dir(node).await? {
                        continue;
                    }
                    tree.add_dir(&path).map_err(Error::from)?;
                    pending.push((node, path.clone()));
                }
                FileType::File => {
                    let mut extents = Vec::new();
                    view.extents(node, |extent| extents.push(extent)).await?;
                    let first = extents.first().map(Extent::offset).filter(|_| meta.len() > 0);
                    match first.and_then(|first| links.get(&first).filter(|_| meta.nlink() > 1)) {
                        Some(target) => tree.add_hard_link(&path, &target.clone()).map_err(Error::from)?,
                        None => {
                            let content = if meta.len() == 0 {
                                Content::empty()
                            } else {
                                Content::stored(extents)
                            };
                            tree.add_file(&path, content).map_err(Error::from)?;
                            if let Some(first) = first {
                                links.insert(first, path.clone());
                            }
                        }
                    }
                }
                FileType::Symlink => {
                    let mut target = vec![0u8; 4096];
                    let len = view.read_link(node, &mut target).await?;
                    tree.add_symlink(&path, &target[..len]).map_err(Error::from)?;
                }
                FileType::CharDevice | FileType::BlockDevice => {
                    let number = view
                        .rock_ridge(node)
                        .await?
                        .and_then(|rr| rr.device())
                        .unwrap_or(hadris_fs::DeviceNumber::new(0, 0));
                    let kind = match meta.file_type() {
                        FileType::BlockDevice => hadris_fs::DeviceKind::Block,
                        _ => hadris_fs::DeviceKind::Char,
                    };
                    tree.add_device(&path, kind, number).map_err(Error::from)?;
                }
                _ => {
                    warnings.push(Warning::new(path, WarningKind::Skipped, "FIFOs and sockets cannot be kept"));
                    continue;
                }
            }
            tree.set_metadata(&path, set_metadata(&meta)).map_err(Error::from)?;
        }
    }
    Ok((tree, options, warnings))
}

}

/// The partition table an image has, and what `Rewrite` needs to extend it.
struct Tables {
    disk: Disk,
    /// The first block after every partition.
    floor: u64,
    gpt: bool,
    /// The 512-byte sector after the old ISO data.
    iso_end: u64,
    /// The 512-byte sector after the backup GPT, or 0 for an MBR.
    backup_end: u64,
}

/// The byte offset and load block of each boot entry in the El Torito
/// catalog `bytes`: the default entry and the entries of each section.
fn catalog_entries(bytes: &[u8]) -> Vec<(usize, u32)> {
    let rba = |at: usize| {
        u32::from_le_bytes([bytes[at + 8], bytes[at + 9], bytes[at + 10], bytes[at + 11]])
    };
    let mut out = Vec::new();
    if bytes.len() < 64 || bytes[0] != 1 {
        return out;
    }
    out.push((32, rba(32)));
    let mut pos = 64;
    while let Some(&kind) = bytes.get(pos) {
        if kind != raw::HEADER_MORE && kind != raw::HEADER_FINAL || pos + 32 > bytes.len() {
            break;
        }
        let count = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]);
        pos += 32;
        for _ in 0..count {
            if pos + 32 > bytes.len() {
                return out;
            }
            out.push((pos, rba(pos)));
            pos += 32;
            while bytes.get(pos) == Some(&0x44) {
                pos += 32;
            }
        }
        if kind == raw::HEADER_FINAL {
            break;
        }
    }
    out
}

fn backup_array(gpt: &Gpt) -> u64 {
    let array = (u64::from(hadris_part::raw::GPT_DEFAULT_ENTRIES) * 128).div_ceil(512);
    gpt.backup_lba().saturating_sub(array)
}

/// A GPT for a disk of `blocks` 512-byte blocks with the partitions of
/// `old`, those `extends` names grown to end at `iso_end`. Returns the new
/// table and each partition's start and new index.
fn regrow(
    old: &Gpt,
    blocks: u64,
    iso_end: u64,
    extends: &dyn Fn(u64, u64) -> bool,
) -> Result<(Gpt, Vec<(u64, usize)>), hadris_part::TableError> {
    let block = BlockSize::new(512).unwrap_or(old.block_size());
    let mut gpt = Gpt::new(old.disk_guid(), blocks, block)?;
    let mut slots = Vec::new();
    for partition in old.partitions() {
        let PartitionKind::Gpt(kind) = partition.kind() else {
            continue;
        };
        let len = if extends(partition.start(), partition.len()) {
            iso_end - partition.start()
        } else {
            partition.len()
        };
        let mut entry = GptEntry::new(
            kind,
            partition
                .unique_guid()
                .unwrap_or(hadris_part::Guid::from_bytes([0; 16])),
            partition.start(),
            len,
        )
        .with_attributes(partition.attributes());
        if let Some(name) = partition.name() {
            entry = entry.with_name(*name);
        }
        let index = gpt.add(entry)?;
        slots.push((partition.start(), index));
    }
    Ok((gpt, slots))
}
