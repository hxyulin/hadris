//! The layout of a volume, planned without I/O: every block the writer
//! emits, in ascending order.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;

use hadris_fs::tree::{NodeKind, Tree, TreeNode, Warning, WarningKind};
use hadris_fs::{Clock, ErrorKind, Extent, SetMetadata};

use crate::error::{Detail, Error};
use crate::name::{encode_cs0, encode_symlink, write_dstring};
use crate::options::UdfOptions;
use crate::raw::{
    self, CharSpec, EntityId, FileCharacteristics, IcbFlags, ShortAd, Tag, Timestamp, U16Le, U32Le,
    U64Le, file_type, tag,
};
use crate::report::{Report, normalize};
use crate::time::from_datetime;
use crate::volume::permissions_of;

pub(crate) type PlanResult<T> = Result<T, Error<Infallible>>;

/// A logical block, and the size of every descriptor sector.
pub(crate) const SECTOR: usize = 2048;
const MAIN_SEQUENCE: u32 = 257;
const SEQUENCE_BLOCKS: u32 = 16;
const RESERVE_SEQUENCE: u32 = MAIN_SEQUENCE + SEQUENCE_BLOCKS;
const INTEGRITY: u32 = RESERVE_SEQUENCE + SEQUENCE_BLOCKS;
const PARTITION_START: u32 = INTEGRITY + 1;
/// The blocks after the partition: the trailing anchor and 256 more.
const TAIL: u64 = 257;
/// The largest extent a short allocation descriptor holds, in whole
/// blocks so that every extent but the last stays block aligned.
const MAX_AD: u64 = (1 << 30) - SECTOR as u64;
/// The short allocation descriptors a file entry block holds.
const MAX_ADS: usize = (SECTOR - 176) / 8;
const IMPLEMENTATION: &[u8] = b"*hadris-udf";

/// The length of a file's content and, for stored content, its extents.
#[derive(Debug, Clone)]
pub(crate) struct ContentInfo {
    pub(crate) len: u64,
    pub(crate) stored: Option<Vec<Extent>>,
}

/// A run of blocks the emitter writes.
#[derive(Debug)]
pub(crate) enum Region {
    Bytes { block: u64, data: Vec<u8> },
    File { block: u64, path: String, len: u64 },
}

impl Region {
    pub(crate) fn block(&self) -> u64 {
        match self {
            Region::Bytes { block, .. } | Region::File { block, .. } => *block,
        }
    }

    pub(crate) fn blocks(&self) -> u64 {
        match self {
            Region::Bytes { data, .. } => (data.len() as u64).div_ceil(SECTOR as u64),
            Region::File { len, .. } => len.div_ceil(SECTOR as u64),
        }
    }
}

pub(crate) struct Plan {
    pub(crate) regions: Vec<Region>,
    pub(crate) total_blocks: u64,
    pub(crate) fill_gaps: bool,
    pub(crate) report: Report,
}

/// Where a file's data is.
enum Data {
    None,
    Written { block: u32, len: u64 },
    Stored(Vec<(u32, u64)>),
    Symlink { block: u32, bytes: Vec<u8> },
}

struct Node<'a> {
    node: TreeNode<'a>,
    path: String,
    icb: u32,
    unique: u64,
    file_type: u8,
    data: Data,
}

struct Dir<'a> {
    node: TreeNode<'a>,
    icb: u32,
    unique: u64,
    parent: usize,
    fid_block: u32,
    fid_bytes: usize,
    /// Non-directories as node indexes, then directories as dir indexes.
    files: Vec<(Vec<u8>, usize)>,
    dirs: Vec<(Vec<u8>, usize)>,
}

fn too_large() -> Error<Infallible> {
    Error::invalid(Detail::ImageTooLarge)
}

fn block_of(value: u64) -> PlanResult<u32> {
    u32::try_from(value).map_err(|_| too_large())
}

struct Planner<'a, C> {
    opts: &'a UdfOptions<C>,
    contents: &'a BTreeMap<usize, ContentInfo>,
    measured: bool,
    now: Timestamp,
    version: u16,
    next: u64,
    unique: u64,
    dirs: Vec<Dir<'a>>,
    nodes: Vec<Node<'a>>,
    by_id: BTreeMap<usize, usize>,
    warnings: Vec<Warning>,
}

impl<'a, C: Clock> Planner<'a, C> {
    fn alloc(&mut self, blocks: u64) -> PlanResult<u32> {
        let block = block_of(self.next)?;
        self.next = self.next.checked_add(blocks).ok_or_else(too_large)?;
        block_of(self.next)?;
        Ok(block)
    }

    fn unique(&mut self) -> u64 {
        let id = self.unique;
        self.unique += 1;
        id
    }

    fn warn(&mut self, path: &str, kind: WarningKind, message: &str) {
        self.warnings
            .push(Warning::new(normalize(path), kind, message));
    }

    fn check_metadata(&mut self, path: &str, meta: &SetMetadata) {
        if meta.times().created().is_some() {
            self.warn(
                path,
                WarningKind::IgnoredMetadata,
                "file entries have no creation time",
            );
        }
        if meta.attributes().is_some_and(|attrs| !attrs.is_empty()) {
            self.warn(
                path,
                WarningKind::IgnoredMetadata,
                "UDF stores no DOS attributes",
            );
        }
    }

    fn add_dir(
        &mut self,
        node: TreeNode<'a>,
        path: &str,
        parent: Option<usize>,
    ) -> PlanResult<usize> {
        let icb = self.alloc(1)?;
        let unique = self.unique();
        let index = self.dirs.len();
        self.check_metadata(path, node.metadata());
        let mut entries = Vec::new();
        let mut fid_bytes = 40usize;
        for (name, child) in node.children() {
            let child_path = alloc::format!("{path}/{name}");
            let symlink = match child.kind() {
                NodeKind::Device(..) => {
                    self.warn(
                        &child_path,
                        WarningKind::Skipped,
                        "UDF volumes store no device nodes",
                    );
                    continue;
                }
                NodeKind::Symlink(target) => match encode_symlink(target) {
                    Some(bytes) => Some(bytes),
                    None => {
                        self.warn(
                            &child_path,
                            WarningKind::Skipped,
                            "symlink target is not UTF-8 or has a component over 255 bytes",
                        );
                        continue;
                    }
                },
                _ => None,
            };
            let encoded = encode_cs0(name);
            fid_bytes = fid_bytes
                .checked_add(fid_len(&encoded)?)
                .ok_or_else(too_large)?;
            entries.push((encoded, child, child_path, symlink));
        }
        let fid_block = self.alloc(fid_bytes.div_ceil(SECTOR) as u64)?;
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for (encoded, child, child_path, symlink) in entries {
            if matches!(child.kind(), NodeKind::Dir) {
                dirs.push((encoded, usize::MAX));
            } else {
                files.push((encoded, self.add_node(child, &child_path, symlink)?));
            }
        }
        self.dirs.push(Dir {
            node,
            icb,
            unique,
            parent: parent.unwrap_or(index),
            fid_block,
            fid_bytes,
            files,
            dirs,
        });
        Ok(index)
    }

    fn add_node(
        &mut self,
        node: TreeNode<'a>,
        path: &str,
        symlink: Option<Vec<u8>>,
    ) -> PlanResult<usize> {
        if let Some(&index) = self.by_id.get(&node.id()) {
            return Ok(index);
        }
        let icb = self.alloc(1)?;
        let unique = self.unique();
        self.check_metadata(path, node.metadata());
        let (file_type, data) = match symlink {
            Some(bytes) => {
                let blocks = (bytes.len() as u64).div_ceil(SECTOR as u64);
                let block = self.alloc(blocks)?;
                (file_type::SYMLINK, Data::Symlink { block, bytes })
            }
            None => (file_type::FILE, self.file_data(node)?),
        };
        let index = self.nodes.len();
        self.nodes.push(Node {
            node,
            path: path.to_string(),
            icb,
            unique,
            file_type,
            data,
        });
        self.by_id.insert(node.id(), index);
        Ok(index)
    }

    fn file_data(&mut self, node: TreeNode<'a>) -> PlanResult<Data> {
        let Some(info) = self.contents.get(&node.id()) else {
            return match self.measured {
                true => Err(Error::corrupt(Detail::Content)),
                false => Ok(Data::None),
            };
        };
        if let Some(stored) = &info.stored {
            let mut extents = Vec::new();
            for extent in stored.iter().filter(|extent| !extent.is_empty()) {
                if extent.offset() % SECTOR as u64 != 0 {
                    return Err(Error::invalid(Detail::StoredContent));
                }
                let block = (extent.offset() / SECTOR as u64)
                    .checked_sub(u64::from(PARTITION_START))
                    .ok_or(Error::invalid(Detail::StoredContent))?;
                extents.push((block_of(block)?, extent.len()));
            }
            return Ok(if extents.is_empty() {
                Data::None
            } else {
                Data::Stored(extents)
            });
        }
        if info.len == 0 {
            return Ok(Data::None);
        }
        let block = self.alloc(info.len.div_ceil(SECTOR as u64))?;
        Ok(Data::Written {
            block,
            len: info.len,
        })
    }
}

/// The on-disk length of a file identifier with the encoded name.
fn fid_len(encoded: &[u8]) -> PlanResult<usize> {
    if encoded.len() > 255 {
        return Err(ErrorKind::NameTooLong.into());
    }
    Ok((38 + encoded.len() + 3) & !3)
}

/// Short allocation descriptors of `len` bytes from `block`, split at the
/// largest extent one descriptor holds.
fn short_ads(block: u32, len: u64, out: &mut Vec<ShortAd>) -> PlanResult<()> {
    let mut position = u64::from(block);
    let mut remaining = len;
    while remaining > 0 {
        let chunk = remaining.min(MAX_AD);
        out.push(ShortAd {
            length: U32Le::new(chunk as u32),
            position: U32Le::new(block_of(position)?),
        });
        position += MAX_AD / SECTOR as u64;
        remaining -= chunk;
    }
    Ok(())
}

fn charspec(buf: &mut [u8]) {
    buf.copy_from_slice(bytemuck::bytes_of(&CharSpec::OSTA));
}

fn entity(buf: &mut [u8], id: &[u8]) {
    buf.copy_from_slice(bytemuck::bytes_of(&EntityId::new(id)));
}

fn put(buf: &mut [u8], at: usize, bytes: &[u8]) {
    buf[at..at + bytes.len()].copy_from_slice(bytes);
}

impl<C: Clock> Planner<'_, C> {
    fn revision(&self) -> [u8; 2] {
        self.opts.revision().to_raw().to_le_bytes()
    }

    fn domain(&self, buf: &mut [u8]) {
        entity(buf, EntityId::OSTA_DOMAIN);
        put(buf, 24, &self.revision());
    }

    fn sector(&self) -> Vec<u8> {
        vec![0u8; SECTOR]
    }

    fn seal(&self, buf: &mut [u8], id: u16, location: u32, crc_length: usize) {
        Tag::seal(buf, id, self.version, location, crc_length.min(496));
    }

    fn volume_id(&self, field: &mut [u8]) {
        write_dstring(field, self.opts.volume_id());
    }

    fn anchor(&self, location: u32) -> Vec<u8> {
        let mut buf = self.sector();
        let extent = |start: u32| raw::ExtentAd {
            length: U32Le::new(SEQUENCE_BLOCKS * SECTOR as u32),
            location: U32Le::new(start),
        };
        put(&mut buf, 16, bytemuck::bytes_of(&extent(MAIN_SEQUENCE)));
        put(&mut buf, 24, bytemuck::bytes_of(&extent(RESERVE_SEQUENCE)));
        self.seal(&mut buf, tag::ANCHOR, location, SECTOR - 16);
        buf
    }

    fn primary(&self, location: u32) -> Vec<u8> {
        let mut buf = self.sector();
        self.volume_id(&mut buf[24..56]);
        put(&mut buf, 56, &1u16.to_le_bytes());
        put(&mut buf, 58, &1u16.to_le_bytes());
        put(&mut buf, 60, &2u16.to_le_bytes());
        put(&mut buf, 62, &3u16.to_le_bytes());
        put(&mut buf, 64, &1u32.to_le_bytes());
        put(&mut buf, 68, &1u32.to_le_bytes());
        self.volume_id(&mut buf[72..200]);
        charspec(&mut buf[200..264]);
        charspec(&mut buf[264..328]);
        entity(&mut buf[344..376], IMPLEMENTATION);
        put(&mut buf, 376, bytemuck::bytes_of(&self.now));
        entity(&mut buf[388..420], IMPLEMENTATION);
        self.seal(&mut buf, tag::PRIMARY_VOLUME, location, 496);
        buf
    }

    fn implementation_use(&self, location: u32) -> Vec<u8> {
        let mut buf = self.sector();
        put(&mut buf, 16, &1u32.to_le_bytes());
        entity(&mut buf[20..52], b"*UDF LV Info");
        charspec(&mut buf[52..116]);
        self.volume_id(&mut buf[116..244]);
        self.seal(&mut buf, tag::IMPLEMENTATION_USE, location, 496);
        buf
    }

    fn partition(&self, location: u32, length: u32) -> Vec<u8> {
        let mut buf = self.sector();
        put(&mut buf, 16, &2u32.to_le_bytes());
        put(&mut buf, 20, &1u16.to_le_bytes());
        let contents: &[u8] = if self.opts.revision().is_nsr03() {
            b"+NSR03"
        } else {
            b"+NSR02"
        };
        entity(&mut buf[24..56], contents);
        put(&mut buf, 184, &1u32.to_le_bytes());
        put(&mut buf, 188, &PARTITION_START.to_le_bytes());
        put(&mut buf, 192, &length.to_le_bytes());
        entity(&mut buf[196..228], IMPLEMENTATION);
        self.seal(&mut buf, tag::PARTITION, location, 496);
        buf
    }

    fn logical_volume(&self, location: u32) -> Vec<u8> {
        let mut buf = self.sector();
        put(&mut buf, 16, &3u32.to_le_bytes());
        charspec(&mut buf[20..84]);
        self.volume_id(&mut buf[84..212]);
        put(&mut buf, 212, &(SECTOR as u32).to_le_bytes());
        self.domain(&mut buf[216..248]);
        let fsd = raw::LongAd {
            length: U32Le::new(SECTOR as u32),
            ..Default::default()
        };
        put(&mut buf, 248, bytemuck::bytes_of(&fsd));
        put(&mut buf, 264, &6u32.to_le_bytes());
        put(&mut buf, 268, &1u32.to_le_bytes());
        entity(&mut buf[272..304], IMPLEMENTATION);
        put(&mut buf, 432, &(SECTOR as u32).to_le_bytes());
        put(&mut buf, 436, &INTEGRITY.to_le_bytes());
        put(&mut buf, 440, &[1, 6, 1, 0, 0, 0]);
        self.seal(&mut buf, tag::LOGICAL_VOLUME, location, 496);
        buf
    }

    fn unallocated(&self, location: u32) -> Vec<u8> {
        let mut buf = self.sector();
        put(&mut buf, 16, &4u32.to_le_bytes());
        self.seal(&mut buf, tag::UNALLOCATED_SPACE, location, 496);
        buf
    }

    fn terminating(&self, location: u32) -> Vec<u8> {
        let mut buf = self.sector();
        self.seal(&mut buf, tag::TERMINATING, location, 0);
        buf
    }

    fn sequence(&self, start: u32, length: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(6 * SECTOR);
        out.extend(self.primary(start));
        out.extend(self.implementation_use(start + 1));
        out.extend(self.partition(start + 2, length));
        out.extend(self.logical_volume(start + 3));
        out.extend(self.unallocated(start + 4));
        out.extend(self.terminating(start + 5));
        out
    }

    fn integrity(&self, length: u32) -> Vec<u8> {
        let mut buf = self.sector();
        put(&mut buf, 16, bytemuck::bytes_of(&self.now));
        put(&mut buf, 28, &1u32.to_le_bytes());
        put(&mut buf, 40, &self.unique.to_le_bytes());
        put(&mut buf, 72, &1u32.to_le_bytes());
        put(&mut buf, 76, &46u32.to_le_bytes());
        put(&mut buf, 84, &length.to_le_bytes());
        entity(&mut buf[88..120], IMPLEMENTATION);
        put(&mut buf, 120, &(self.nodes.len() as u32).to_le_bytes());
        put(&mut buf, 124, &(self.dirs.len() as u32).to_le_bytes());
        for at in [128, 130, 132] {
            put(&mut buf, at, &self.revision());
        }
        self.seal(&mut buf, tag::INTEGRITY, INTEGRITY, 496);
        buf
    }

    fn file_set(&self, root: u32) -> Vec<u8> {
        let mut buf = self.sector();
        put(&mut buf, 16, bytemuck::bytes_of(&self.now));
        put(&mut buf, 28, &3u16.to_le_bytes());
        put(&mut buf, 30, &3u16.to_le_bytes());
        put(&mut buf, 32, &1u32.to_le_bytes());
        put(&mut buf, 36, &1u32.to_le_bytes());
        charspec(&mut buf[48..112]);
        self.volume_id(&mut buf[112..240]);
        charspec(&mut buf[240..304]);
        self.volume_id(&mut buf[304..336]);
        let root = raw::LongAd {
            length: U32Le::new(SECTOR as u32),
            location: raw::LbAddr {
                block: U32Le::new(root),
                partition: U16Le::new(0),
            },
            implementation_use: [0; 6],
        };
        put(&mut buf, 400, bytemuck::bytes_of(&root));
        self.domain(&mut buf[416..448]);
        self.seal(&mut buf, tag::FILE_SET, 0, 496);
        buf
    }

    fn timestamp(&self, time: Option<hadris_fs::DateTime>) -> PlanResult<Timestamp> {
        match time {
            Some(time) => from_datetime(time).ok_or(Error::invalid(Detail::Timestamp)),
            None => Ok(self.now),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn file_entry(
        &self,
        location: u32,
        kind: u8,
        meta: &SetMetadata,
        links: usize,
        len: u64,
        ads: &[ShortAd],
        unique: u64,
    ) -> PlanResult<Vec<u8>> {
        if ads.len() > MAX_ADS {
            return Err(ErrorKind::FileTooLarge.into());
        }
        let (permissions, flags) = match meta.mode() {
            Some(mode) => permissions_of(mode.bits()),
            None => (0x7FFF, IcbFlags::empty()),
        };
        let times = meta.times();
        let fe = raw::FileEntry {
            tag: Tag::default(),
            icb_tag: raw::IcbTag {
                strategy: U16Le::new(4),
                max_entries: U16Le::new(1),
                file_type: kind,
                flags: U16Le::new(flags.bits()),
                ..Default::default()
            },
            uid: U32Le::new(meta.uid().unwrap_or(u32::MAX)),
            gid: U32Le::new(meta.gid().unwrap_or(u32::MAX)),
            permissions: U32Le::new(permissions),
            link_count: U16Le::new(links.min(usize::from(u16::MAX)) as u16),
            record_format: 0,
            record_display_attributes: 0,
            record_length: U32Le::new(0),
            information_length: U64Le::new(len),
            blocks_recorded: U64Le::new(
                ads.iter()
                    .map(|ad| u64::from(ad.len()).div_ceil(SECTOR as u64))
                    .sum(),
            ),
            accessed: self.timestamp(times.accessed())?,
            modified: self.timestamp(times.modified())?,
            attributes_changed: self.timestamp(times.changed())?,
            checkpoint: U32Le::new(1),
            extended_attribute_icb: raw::LongAd::default(),
            implementation: EntityId::new(IMPLEMENTATION),
            unique_id: U64Le::new(unique),
            extended_attributes_length: U32Le::new(0),
            allocation_descriptors_length: U32Le::new((ads.len() * 8) as u32),
        };
        let mut buf = self.sector();
        put(&mut buf, 0, bytemuck::bytes_of(&fe));
        put(&mut buf, 176, bytemuck::cast_slice(ads));
        self.seal(&mut buf, tag::FILE_ENTRY, location, 160 + ads.len() * 8);
        Ok(buf)
    }

    fn identifier(
        &self,
        location: u32,
        icb: u32,
        unique: u64,
        characteristics: FileCharacteristics,
        name: &[u8],
    ) -> Vec<u8> {
        let mut implementation_use = [0u8; 6];
        if self.opts.revision().is_nsr03() {
            implementation_use[2..].copy_from_slice(&(unique as u32).to_le_bytes());
        }
        let fid = raw::FileIdentifierDescriptor {
            tag: Tag::default(),
            version: U16Le::new(1),
            characteristics: characteristics.bits(),
            identifier_length: name.len() as u8,
            icb: raw::LongAd {
                length: U32Le::new(SECTOR as u32),
                location: raw::LbAddr {
                    block: U32Le::new(icb),
                    partition: U16Le::new(0),
                },
                implementation_use,
            },
            implementation_use_length: U16Le::new(0),
        };
        let total = fid.total_len();
        let mut buf = vec![0u8; total];
        put(&mut buf, 0, bytemuck::bytes_of(&fid));
        put(&mut buf, 38, name);
        Tag::seal(
            &mut buf,
            tag::FILE_IDENTIFIER,
            self.version,
            location,
            total - 16,
        );
        buf
    }

    fn directory(&self, dir: &Dir<'_>) -> PlanResult<(Vec<u8>, Vec<u8>)> {
        let parent = &self.dirs[dir.parent];
        let mut fids = Vec::with_capacity(dir.fid_bytes);
        let location = |len: usize| dir.fid_block + (len / SECTOR) as u32;
        fids.extend(self.identifier(
            location(0),
            parent.icb,
            parent.unique,
            FileCharacteristics::PARENT | FileCharacteristics::DIRECTORY,
            &[],
        ));
        for (name, index) in &dir.files {
            let node = &self.nodes[*index];
            fids.extend(self.identifier(
                location(fids.len()),
                node.icb,
                node.unique,
                FileCharacteristics::empty(),
                name,
            ));
        }
        for (name, index) in &dir.dirs {
            let child = &self.dirs[*index];
            fids.extend(self.identifier(
                location(fids.len()),
                child.icb,
                child.unique,
                FileCharacteristics::DIRECTORY,
                name,
            ));
        }
        let ads = [ShortAd {
            length: U32Le::new(dir.fid_bytes as u32),
            position: U32Le::new(dir.fid_block),
        }];
        let entry = self.file_entry(
            dir.icb,
            file_type::DIRECTORY,
            dir.node.metadata(),
            1 + dir.dirs.len(),
            dir.fid_bytes as u64,
            &ads,
            dir.unique,
        )?;
        fids.resize(fids.len().div_ceil(SECTOR) * SECTOR, 0);
        Ok((entry, fids))
    }
}

/// Plans the volume `opts` describes for `tree`. `contents` holds the
/// length of every file, or, for a bridge volume, the stored extents;
/// without `measured`, files missing from it get no data.
pub(crate) fn plan<C: Clock>(
    tree: &Tree,
    opts: &UdfOptions<C>,
    contents: &BTreeMap<usize, ContentInfo>,
    measured: bool,
) -> PlanResult<Plan> {
    let mut encoded = [0u8; 256];
    if write_dstring(&mut encoded[..128], opts.volume_id()) {
        return Err(Error::invalid(Detail::Identifier));
    }
    let now = from_datetime(opts.clock().now()).ok_or(Error::invalid(Detail::Timestamp))?;
    let mut planner = Planner {
        opts,
        contents,
        measured,
        now,
        version: if opts.revision().is_nsr03() { 3 } else { 2 },
        next: 1,
        unique: 16,
        dirs: Vec::new(),
        nodes: Vec::new(),
        by_id: BTreeMap::new(),
        warnings: Vec::new(),
    };

    let mut stack = vec![(tree.root(), String::new(), None::<(usize, usize)>)];
    while let Some((node, path, parent)) = stack.pop() {
        let index = planner.add_dir(node, &path, parent.map(|(dir, _)| dir))?;
        if let Some((dir, slot)) = parent {
            planner.dirs[dir].dirs[slot].1 = index;
        }
        let children: Vec<_> = node
            .children()
            .filter(|(_, child)| matches!(child.kind(), NodeKind::Dir))
            .enumerate()
            .map(|(slot, (name, child))| {
                (child, alloc::format!("{path}/{name}"), Some((index, slot)))
            })
            .collect();
        stack.extend(children.into_iter().rev());
    }

    let allocated_end = u64::from(PARTITION_START) + planner.next;
    let mut data_end = allocated_end;
    for node in &planner.nodes {
        if let Data::Stored(extents) = &node.data {
            for &(block, len) in extents {
                let start = u64::from(PARTITION_START) + u64::from(block);
                if start < allocated_end {
                    return Err(Error::invalid(Detail::StoredContent));
                }
                data_end = data_end.max(start + len.div_ceil(SECTOR as u64));
            }
        }
    }
    let total = opts.min_blocks().max(data_end + TAIL);
    let anchor = block_of(total - TAIL)?;
    block_of(total)?;
    let partition_len = anchor - PARTITION_START;

    let mut regions = Vec::new();
    let recognition = u64::from(opts.bridge().map_or(0, |bridge| bridge.iso_descriptors()));
    let nsr = if opts.revision().is_nsr03() {
        raw::vsd::NSR03
    } else {
        raw::vsd::NSR02
    };
    let mut vrs = Vec::with_capacity(3 * SECTOR);
    for id in [raw::vsd::BEA01, nsr, raw::vsd::TEA01] {
        vrs.extend_from_slice(bytemuck::bytes_of(&raw::VolumeStructureDescriptor::new(id)));
    }
    regions.push(Region::Bytes {
        block: raw::VRS_START + recognition,
        data: vrs,
    });
    regions.push(Region::Bytes {
        block: u64::from(raw::ANCHOR_BLOCK),
        data: planner.anchor(raw::ANCHOR_BLOCK),
    });
    regions.push(Region::Bytes {
        block: u64::from(MAIN_SEQUENCE),
        data: planner.sequence(MAIN_SEQUENCE, partition_len),
    });
    regions.push(Region::Bytes {
        block: u64::from(RESERVE_SEQUENCE),
        data: planner.sequence(RESERVE_SEQUENCE, partition_len),
    });
    regions.push(Region::Bytes {
        block: u64::from(INTEGRITY),
        data: planner.integrity(partition_len),
    });
    let root_icb = planner.dirs[0].icb;
    regions.push(Region::Bytes {
        block: u64::from(PARTITION_START),
        data: planner.file_set(root_icb),
    });

    let base = u64::from(PARTITION_START);
    for dir in &planner.dirs {
        let (entry, fids) = planner.directory(dir)?;
        regions.push(Region::Bytes {
            block: base + u64::from(dir.icb),
            data: entry,
        });
        regions.push(Region::Bytes {
            block: base + u64::from(dir.fid_block),
            data: fids,
        });
    }
    let mut extents = BTreeMap::new();
    for node in &planner.nodes {
        let mut ads = Vec::new();
        let len = match &node.data {
            Data::None => 0,
            Data::Written { block, len } => {
                short_ads(*block, *len, &mut ads)?;
                regions.push(Region::File {
                    block: base + u64::from(*block),
                    path: node.path.clone(),
                    len: *len,
                });
                extents.insert(
                    node.node.id(),
                    Some(Extent::new(
                        (base + u64::from(*block)) * SECTOR as u64,
                        *len,
                    )),
                );
                *len
            }
            Data::Stored(stored) => {
                let mut contiguous = true;
                let mut expect = None;
                for &(block, len) in stored {
                    if expect.is_some_and(|expect| expect != block) {
                        contiguous = false;
                    }
                    expect = Some(block + (len / SECTOR as u64) as u32);
                    short_ads(block, len, &mut ads)?;
                }
                let total: u64 = stored.iter().map(|(_, len)| len).sum();
                let extent = Extent::new((base + u64::from(stored[0].0)) * SECTOR as u64, total);
                extents.insert(node.node.id(), contiguous.then_some(extent));
                total
            }
            Data::Symlink { block, bytes } => {
                short_ads(*block, bytes.len() as u64, &mut ads)?;
                regions.push(Region::Bytes {
                    block: base + u64::from(*block),
                    data: bytes.clone(),
                });
                bytes.len() as u64
            }
        };
        let entry = planner.file_entry(
            node.icb,
            node.file_type,
            node.node.metadata(),
            node.node.links(),
            len,
            &ads,
            node.unique,
        )?;
        regions.push(Region::Bytes {
            block: base + u64::from(node.icb),
            data: entry,
        });
    }
    regions.push(Region::Bytes {
        block: u64::from(anchor),
        data: planner.anchor(anchor),
    });
    let fill_gaps = opts.bridge().is_none();
    if !fill_gaps {
        regions.push(Region::Bytes {
            block: total - 1,
            data: planner.sector(),
        });
    }
    regions.sort_by_key(Region::block);

    let mut by_path = BTreeMap::new();
    collect_paths(tree.root(), "", &extents, &mut by_path);
    let report = Report::new(total, allocated_end, by_path, planner.warnings);
    Ok(Plan {
        regions,
        total_blocks: total,
        fill_gaps,
        report,
    })
}

/// The extent of every name of every file with one.
fn collect_paths(
    dir: TreeNode<'_>,
    prefix: &str,
    extents: &BTreeMap<usize, Option<Extent>>,
    out: &mut BTreeMap<String, Extent>,
) {
    let mut pending = vec![(dir, prefix.to_string())];
    while let Some((dir, prefix)) = pending.pop() {
        for (name, child) in dir.children() {
            let path = alloc::format!("{prefix}/{name}");
            match child.kind() {
                NodeKind::Dir => pending.push((child, path)),
                _ => {
                    if let Some(Some(extent)) = extents.get(&child.id()) {
                        out.insert(path, *extent);
                    }
                }
            }
        }
    }
}
