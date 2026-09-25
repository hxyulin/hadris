//! The layout of a volume, planned without I/O: every block the writer
//! emits, in ascending order.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::convert::Infallible;

use hadris_fs::{
    ErrorKind, Extent, Field, FileType, Name, PathError, Report, SetAttr, Tree, TreeEntry, Warning,
    WarningKind,
};

use crate::error::{Detail, Error};
use crate::name::{encode_cs0, encode_symlink, write_dstring};
use crate::options::UdfOptions;
use crate::raw::{
    self, CharSpec, EntityId, FileCharacteristics, IcbFlags, ShortAd, Tag, Timestamp, U16Le, U32Le,
    U64Le, file_type, tag,
};
use crate::time::from_datetime;
use crate::volume::UdfId;
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
    /// The block after the last one the writer allocates in the partition
    /// for its structures and the data it writes. In a bridge volume the
    /// ISO 9660 directories and files go at or after it.
    pub(crate) allocated_end: u64,
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
    node: TreeEntry<'a>,
    path: String,
    icb: u32,
    unique: u64,
    file_type: u8,
    data: Data,
}

struct Dir<'a> {
    node: TreeEntry<'a>,
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
    Detail::ImageTooLarge.invalid()
}

fn block_of(value: u64) -> PlanResult<u32> {
    u32::try_from(value).map_err(|_| too_large())
}

struct Planner<'a> {
    opts: &'a UdfOptions,
    /// The identifiers, by `UdfId`, defaults resolved.
    ids: [String; 4],
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
    /// Nodes that set a creation time, and nodes that set attributes.
    dropped: [u64; 2],
}

/// A tree name as text; names that are not UTF-8 get U+FFFD and a
/// warning.
fn text(name: &Name, path: &[u8], warnings: &mut Vec<Warning>) -> String {
    match core::str::from_utf8(name.as_bytes()) {
        Ok(text) => String::from(text),
        Err(_) => {
            let text = String::from_utf8_lossy(name.as_bytes()).into_owned();
            warnings.push(
                Warning::new(WarningKind::Renamed, "udf names are unicode")
                    .with_path(path)
                    .with_stored_as(&text),
            );
            text
        }
    }
}

fn join(parent: &str, name: &Name) -> String {
    alloc::format!("{parent}/{}", String::from_utf8_lossy(name.as_bytes()))
}

impl<'a> Planner<'a> {
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

    fn warn(&mut self, path: &str, kind: WarningKind, message: &'static str) {
        self.warnings
            .push(Warning::new(kind, message).with_path(path));
    }

    fn check_metadata(&mut self, meta: &SetAttr) {
        self.dropped[0] += u64::from(meta.created().is_some());
        self.dropped[1] += u64::from(meta.attributes().is_some_and(|attrs| !attrs.is_empty()));
    }

    fn add_dir(
        &mut self,
        node: TreeEntry<'a>,
        path: &str,
        parent: Option<usize>,
    ) -> PlanResult<usize> {
        let icb = self.alloc(1)?;
        let unique = self.unique();
        let index = self.dirs.len();
        self.check_metadata(node.node().attrs());
        let mut entries = Vec::new();
        let mut fid_bytes = 40usize;
        for (name, child) in node.children() {
            let child_path = join(path, name);
            let symlink = match child.node().file_type() {
                FileType::Dir | FileType::File => None,
                FileType::Symlink => match child.node().target().and_then(encode_symlink) {
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
                _ => {
                    self.warn(
                        &child_path,
                        WarningKind::Skipped,
                        "the udf writer stores no device nodes, fifos or sockets",
                    );
                    continue;
                }
            };
            let encoded = encode_cs0(&text(name, child_path.as_bytes(), &mut self.warnings));
            fid_bytes = fid_bytes
                .checked_add(fid_len(&encoded)?)
                .ok_or_else(too_large)?;
            entries.push((encoded, child, child_path, symlink));
        }
        let fid_block = self.alloc(fid_bytes.div_ceil(SECTOR) as u64)?;
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for (encoded, child, child_path, symlink) in entries {
            if child.node().file_type() == FileType::Dir {
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
        node: TreeEntry<'a>,
        path: &str,
        symlink: Option<Vec<u8>>,
    ) -> PlanResult<usize> {
        if let Some(&index) = self.by_id.get(&node.id()) {
            return Ok(index);
        }
        let icb = self.alloc(1)?;
        let unique = self.unique();
        self.check_metadata(node.node().attrs());
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

    fn file_data(&mut self, node: TreeEntry<'a>) -> PlanResult<Data> {
        let Some(info) = self.contents.get(&node.id()) else {
            return match self.measured {
                true => Err(Detail::Content.corrupt()),
                false => Ok(Data::None),
            };
        };
        if let Some(stored) = &info.stored {
            let mut extents = Vec::new();
            for extent in stored.iter().filter(|extent| !extent.is_empty()) {
                if extent.offset() % SECTOR as u64 != 0 {
                    return Err(Detail::StoredContent.invalid());
                }
                let block = (extent.offset() / SECTOR as u64)
                    .checked_sub(u64::from(PARTITION_START))
                    .ok_or(Detail::StoredContent.invalid())?;
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

impl Planner<'_> {
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

    fn id(&self, id: UdfId, field: &mut [u8]) {
        write_dstring(field, &self.ids[id as usize]);
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
        self.id(UdfId::Volume, &mut buf[24..56]);
        put(&mut buf, 56, &1u16.to_le_bytes());
        put(&mut buf, 58, &1u16.to_le_bytes());
        put(&mut buf, 60, &2u16.to_le_bytes());
        put(&mut buf, 62, &3u16.to_le_bytes());
        put(&mut buf, 64, &1u32.to_le_bytes());
        put(&mut buf, 68, &1u32.to_le_bytes());
        self.id(UdfId::VolumeSet, &mut buf[72..200]);
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
        self.id(UdfId::LogicalVolume, &mut buf[116..244]);
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
        self.id(UdfId::LogicalVolume, &mut buf[84..212]);
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
        self.id(UdfId::LogicalVolume, &mut buf[112..240]);
        charspec(&mut buf[240..304]);
        self.id(UdfId::FileSet, &mut buf[304..336]);
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
            Some(time) => from_datetime(time).ok_or(Detail::Timestamp.invalid()),
            None => Ok(self.now),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn file_entry(
        &self,
        location: u32,
        kind: u8,
        meta: &SetAttr,
        links: usize,
        len: u64,
        ads: &[ShortAd],
        unique: u64,
    ) -> PlanResult<Vec<u8>> {
        if ads.len() > MAX_ADS {
            return Err(ErrorKind::FileTooLarge.into());
        }
        let (permissions, flags) = match meta.permissions() {
            Some(mode) => permissions_of(mode.bits()),
            None => (0x7FFF, IcbFlags::empty()),
        };
        let fe = raw::FileEntry {
            tag: Tag::default(),
            icb_tag: raw::IcbTag {
                strategy: U16Le::new(4),
                max_entries: U16Le::new(1),
                file_type: kind,
                flags: U16Le::new(flags.bits()),
                ..Default::default()
            },
            uid: U32Le::new(meta.owner().map_or(u32::MAX, |owner| owner.uid())),
            gid: U32Le::new(meta.owner().map_or(u32::MAX, |owner| owner.gid())),
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
            accessed: self.timestamp(meta.accessed())?,
            modified: self.timestamp(meta.modified())?,
            attributes_changed: self.timestamp(None)?,
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
            dir.node.node().attrs(),
            1 + dir.dirs.len(),
            dir.fid_bytes as u64,
            &ads,
            dir.unique,
        )?;
        fids.resize(fids.len().div_ceil(SECTOR) * SECTOR, 0);
        Ok((entry, fids))
    }
}

/// Measures every file of `tree` without I/O: the length of content to
/// write, or the extents of stored content in a bridge volume. With
/// `strict`, a bridge file whose content is neither stored nor empty
/// fails; without, it is left out.
pub(crate) fn measure(
    tree: &Tree,
    bridge: bool,
    strict: bool,
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
                Some(extents) if bridge => ContentInfo {
                    len: content.len(),
                    stored: Some(extents.to_vec()),
                },
                None if bridge && content.is_empty() => ContentInfo {
                    len: 0,
                    stored: None,
                },
                None if bridge && !strict => continue,
                None if !bridge => ContentInfo {
                    len: content.len(),
                    stored: None,
                },
                _ => {
                    return Err(PathError::from(
                        Detail::StoredContent.error::<Infallible>(ErrorKind::Unsupported),
                    )
                    .with_path(path));
                }
            };
            out.insert(child.id(), info);
        }
    }
    Ok(out)
}

/// Plans writing `tree` as a UDF volume, without I/O, and returns the
/// report `write` returns: the volume size, where each file's data goes,
/// and what the volume cannot store as the tree asks.
///
/// Size an output device with [`Report::size`]; the volume includes the
/// trailing anchor and the 256 blocks after it. Fails as `write` does
/// before it writes anything: [`ErrorKind::InvalidInput`] for a volume
/// identifier over 126 bytes or a time outside the years 1 to 9999,
/// [`ErrorKind::NameTooLong`] for a name over 254 bytes of OSTA Compressed
/// Unicode, [`ErrorKind::FileTooLarge`] for a file of more than 234 GiB,
/// and [`ErrorKind::Unsupported`] for content stored on another image.
pub fn plan(tree: &Tree, opts: &UdfOptions) -> Result<Report, PathError> {
    let contents = measure(tree, false, false)?;
    Ok(lay_out(tree, opts, &contents, true, None)?.report)
}

/// The identifiers of `opts` with their defaults, checked against their
/// fields.
fn identifiers(tree: &Tree, opts: &UdfOptions) -> PlanResult<[String; 4]> {
    let volume = String::from(opts.id(UdfId::Volume).unwrap_or(""));
    let serial = {
        let time = opts.time();
        let base = opts
            .seed()
            .unwrap_or((time.unix_seconds() as u64) ^ (u64::from(time.nanoseconds()) << 32));
        base.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ tree.fingerprint()
    };
    let or = |id, default: &str| String::from(opts.id(id).unwrap_or(default));
    let ids = [
        volume.clone(),
        or(UdfId::VolumeSet, &alloc::format!("{serial:016X}{volume}")),
        or(UdfId::LogicalVolume, &volume),
        or(UdfId::FileSet, &volume),
    ];
    let mut field = [0u8; 128];
    for (id, len) in [
        (UdfId::Volume, 32),
        (UdfId::VolumeSet, 128),
        (UdfId::LogicalVolume, 128),
        (UdfId::FileSet, 32),
    ] {
        if write_dstring(&mut field[..len], &ids[id as usize]) {
            return Err(Detail::Identifier.invalid());
        }
    }
    Ok(ids)
}

/// Plans the volume `opts` describes for `tree`. `contents` holds the
/// length of every file, or, for a bridge volume, the stored extents;
/// without `measured`, files missing from it get no data. `bridge` is the
/// number of ISO 9660 volume descriptors a bridge volume follows.
pub(crate) fn lay_out(
    tree: &Tree,
    opts: &UdfOptions,
    contents: &BTreeMap<usize, ContentInfo>,
    measured: bool,
    bridge: Option<u32>,
) -> PlanResult<Plan> {
    if opts.revision() >= crate::UdfRevision::V2_50 {
        return Err(Detail::PartitionMap.error(ErrorKind::Unsupported));
    }
    let ids = identifiers(tree, opts)?;
    let now = from_datetime(opts.time()).ok_or(Detail::Timestamp.invalid())?;
    let mut planner = Planner {
        opts,
        ids,
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
        dropped: [0; 2],
    };

    let mut stack = vec![(tree.root(), String::new(), None::<(usize, usize)>)];
    while let Some((node, path, parent)) = stack.pop() {
        let index = planner.add_dir(node, &path, parent.map(|(dir, _)| dir))?;
        if let Some((dir, slot)) = parent {
            planner.dirs[dir].dirs[slot].1 = index;
        }
        let children: Vec<_> = node
            .children()
            .filter(|(_, child)| child.node().file_type() == FileType::Dir)
            .enumerate()
            .map(|(slot, (name, child))| (child, join(&path, name), Some((index, slot))))
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
                    return Err(Detail::StoredContent.invalid());
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
    let recognition = u64::from(bridge.unwrap_or(0));
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
    let mut extents: BTreeMap<usize, Vec<Extent>> = BTreeMap::new();
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
                    vec![Extent::new(
                        (base + u64::from(*block)) * SECTOR as u64,
                        *len,
                    )],
                );
                *len
            }
            Data::Stored(stored) => {
                let mut list = Vec::new();
                for &(block, len) in stored {
                    short_ads(block, len, &mut ads)?;
                    list.push(Extent::new((base + u64::from(block)) * SECTOR as u64, len));
                }
                extents.insert(node.node.id(), list);
                stored.iter().map(|(_, len)| len).sum()
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
            node.node.node().attrs(),
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
    let fill_gaps = bridge.is_none();
    if !fill_gaps {
        regions.push(Region::Bytes {
            block: total - 1,
            data: planner.sector(),
        });
    }
    regions.sort_by_key(Region::block);

    let mut report = Report::new();
    report.set_size(total * SECTOR as u64);
    for warning in planner.warnings {
        report.push_warning(warning);
    }
    let losses = [
        (Field::Created, "file entries have no creation time"),
        (Field::Attributes, "udf stores no dos attributes"),
    ];
    for ((field, message), count) in losses.into_iter().zip(planner.dropped) {
        if count > 0 {
            report
                .push_warning(Warning::new(WarningKind::Dropped(field), message).with_count(count));
        }
    }
    collect_paths(tree.root(), &extents, &mut report);
    Ok(Plan {
        regions,
        total_blocks: total,
        allocated_end,
        fill_gaps,
        report,
    })
}

/// The extents of every name of every file with some.
fn collect_paths(dir: TreeEntry<'_>, extents: &BTreeMap<usize, Vec<Extent>>, report: &mut Report) {
    let mut pending = vec![(dir, Vec::new())];
    while let Some((dir, prefix)) = pending.pop() {
        for (name, child) in dir.children() {
            let mut path = prefix.clone();
            path.push(b'/');
            path.extend_from_slice(name.as_bytes());
            if child.node().file_type() == FileType::Dir {
                pending.push((child, path));
            } else if let Some(list) = extents.get(&child.id()) {
                for extent in list {
                    report.push_extent(&path, *extent);
                }
            }
        }
    }
}
