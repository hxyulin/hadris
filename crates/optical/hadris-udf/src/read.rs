use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, Extent, FileType, FsResult, FsStats, Metadata,
    MountError, MountOptions, Name, NodeId, OpenMode, Permissions,
};
use hadris_storage::BlockIndex;

use super::FileSystem;
use super::storage::BlockDevice;
use crate::UdfRevision;
use crate::error::{Detail, Error};
use crate::volume::{
    EntityId as Entity, Icb, Identifier, Info, Location, MAX_BLOCK, MAX_PARTITIONS, PartitionInfo,
    PartitionKind, VolumeInfo, check_tag,
};
#[cfg(feature = "alloc")]
use crate::volume::{PartitionMap, PartitionSpan};
use hadris_udf_raw::{
    self as raw, AllocationExtentDescriptor, AnchorVolumeDescriptorPointer, EntityId, ExtAd,
    FileCharacteristics, FileIdentifierDescriptor, FileSetDescriptor, IcbFlags,
    LogicalVolumeDescriptor, LogicalVolumeIntegrityDescriptor, LongAd, PathComponent, ShortAd, Tag,
    allocation, extent, tag, vsd,
};

/// Allocation descriptors walked for one file.
const MAX_DESCRIPTORS: u32 = 1 << 20;
/// Allocation extent descriptors followed for one file.
const MAX_CONTINUATIONS: u32 = 1 << 12;
/// Blocks read from one volume descriptor sequence, pointers included.
const MAX_SEQUENCE: u32 = 512;
/// Volume structure descriptors read from the recognition sequence.
const MAX_RECOGNITION: u64 = 64;
/// Blocks read from the integrity sequence.
const MAX_INTEGRITY: u32 = 1024;
/// The buffer a directory identifier is read into when it fits.
const FID_BUFFER: usize = 512;

#[derive(Debug)]
struct DirectoryWindow {
    bytes: [u8; MAX_BLOCK],
    offset: Option<u64>,
}

/// One stretch of a file's data.
#[derive(Clone, Copy)]
enum Piece {
    Disk {
        location: Location,
        len: u64,
    },
    /// Reads as zeros; `location` is where an allocated but unrecorded
    /// extent lies.
    Zero {
        len: u64,
        location: Option<Location>,
    },
    Embedded {
        start: usize,
        len: usize,
    },
}

impl Piece {
    fn len(&self) -> u64 {
        match *self {
            Piece::Disk { len, .. } | Piece::Zero { len, .. } => len,
            Piece::Embedded { len, .. } => len as u64,
        }
    }
}

/// Walks the allocation descriptors of an entry, following continuation
/// extents.
struct Walk {
    aed: [u8; MAX_BLOCK],
    /// The allocation extent descriptor in `aed`, once one is followed.
    aed_at: Option<Location>,
    pos: usize,
    end: usize,
    steps: u32,
    hops: u32,
    done: bool,
}

/// Where a walk of an entry's allocation descriptors stood before the
/// piece holding the start of the last read, so that reads at or after it
/// resume there instead of walking from the first descriptor.
#[derive(Debug, Clone, Copy, Default)]
struct Resume {
    icb: Option<Location>,
    aed_at: Option<Location>,
    pos: usize,
    end: usize,
    steps: u32,
    hops: u32,
    /// The byte of the data the next piece starts at.
    offset: u64,
    location: Option<u32>,
}

/// A File Identifier Descriptor found in a directory.
struct Fid {
    characteristics: FileCharacteristics,
    icb: Location,
    name: [u8; 255],
    name_len: usize,
    next: u64,
}

/// The id of the entry `fid` names, when its ICB lies inside a partition.
fn fid_node<E>(info: &Info, fid: &Fid) -> Result<NodeId, Error<E>> {
    info.check_range::<E>(fid.icb, 1)
        .map_err(|_| Detail::FileIdentifier.corrupt())?;
    Ok(fid.icb.id())
}

/// The metadata listed for an entry whose file entry cannot be read: the
/// type its identifier records and nothing else.
fn damaged(fid: &Fid) -> Metadata {
    let file_type = match fid.characteristics.contains(FileCharacteristics::DIRECTORY) {
        true => FileType::Dir,
        false => FileType::File,
    };
    Metadata::new(file_type, Permissions::new(0))
}

/// The revision a logical volume's domain records, when it agrees with the
/// recognition sequence.
fn domain_revision(domain: &EntityId, nsr03: bool) -> Option<UdfRevision> {
    if domain.name() != EntityId::OSTA_DOMAIN {
        return None;
    }
    let revision = UdfRevision::from_raw(domain.udf_revision());
    match nsr03 {
        false if revision.to_raw() != 0 && revision < UdfRevision::V2_00 => Some(revision),
        true if revision >= UdfRevision::V2_00 => Some(revision),
        _ => None,
    }
}

/// The prevailing descriptors of one volume descriptor sequence.
struct Sequence {
    primary: Option<(u32, raw::PrimaryVolumeDescriptor)>,
    logical: Option<(u32, [u8; MAX_BLOCK])>,
    partitions: [(u16, u32, PartitionInfo); MAX_PARTITIONS],
    partition_count: usize,
}

#[cfg(feature = "alloc")]
#[derive(Clone, Copy)]
enum MapSpec {
    Physical,
    Sparable {
        packet: u32,
        size: u32,
        tables: [u32; 4],
        count: u8,
    },
    Metadata {
        file: u32,
        mirror: u32,
    },
    Virtual {
        v2: bool,
    },
}

#[cfg(feature = "alloc")]
impl MapSpec {
    fn kind(self) -> PartitionKind {
        match self {
            Self::Physical => PartitionKind::Physical,
            Self::Sparable { .. } => PartitionKind::Sparable,
            Self::Metadata { .. } => PartitionKind::Metadata,
            Self::Virtual { .. } => PartitionKind::Virtual,
        }
    }
}

#[cfg(feature = "alloc")]
fn map_spec<E>(map: &[u8], revision: UdfRevision) -> FsResult<MapSpec, E> {
    let id: EntityId = bytemuck::pod_read_unaligned(&map[4..36]);
    let word = |at| u32::from_le_bytes([map[at], map[at + 1], map[at + 2], map[at + 3]]);
    match id.name() {
        b"*UDF Sparable Partition" => {
            let packet = u32::from(u16::from_le_bytes([map[40], map[41]]));
            let count = map[42];
            if packet == 0 || !(1..=4).contains(&count) {
                return Err(Detail::PartitionMap.corrupt());
            }
            Ok(MapSpec::Sparable {
                packet,
                count,
                size: word(44),
                tables: core::array::from_fn(|index| word(48 + index * 4)),
            })
        }
        b"*UDF Metadata Partition" => Ok(MapSpec::Metadata {
            file: word(40),
            mirror: word(44),
        }),
        b"*UDF Virtual Partition" => {
            let map_revision = UdfRevision::from_raw(id.udf_revision());
            let v2 = if map_revision.to_raw() != 0 {
                map_revision
            } else {
                revision
            } >= UdfRevision::V2_00;
            Ok(MapSpec::Virtual { v2 })
        }
        _ => Err(Detail::PartitionMap.error(ErrorKind::Unsupported)),
    }
}

#[cfg(feature = "alloc")]
fn add_span<E>(
    spans: &mut alloc::vec::Vec<PartitionSpan>,
    block: u32,
    physical: u64,
    len: u32,
) -> FsResult<(), E> {
    if len == 0 {
        return Ok(());
    }
    if let Some(last) = spans.last_mut() {
        if last.block.checked_add(last.len) == Some(block)
            && last.physical + u64::from(last.len) == physical
        {
            last.len = last
                .len
                .checked_add(len)
                .ok_or(Detail::PartitionMap.corrupt())?;
            return Ok(());
        }
    }
    if spans.len() >= MAX_DESCRIPTORS as usize {
        return Err(ErrorKind::LimitExceeded.into());
    }
    spans.try_reserve(1).map_err(|_| ErrorKind::LimitExceeded)?;
    spans.push(PartitionSpan {
        block,
        physical,
        len,
    });
    Ok(())
}

io_transform! {
impl DirectoryWindow {
    fn new() -> Self {
        Self {
            bytes: [0; MAX_BLOCK],
            offset: None,
        }
    }

    async fn read<D: BlockDevice>(
        &mut self,
        info: &Info,
        dev: &mut D,
        mut offset: u64,
        mut out: &mut [u8],
    ) -> Result<(), Error<D::Error>> {
        let size = u64::from(info.block_size.max(dev.block_size().get()));
        while !out.is_empty() {
            let base = offset / size * size;
            let len = usize::try_from(info.len.saturating_sub(base).min(size)).unwrap_or(0);
            let within = (offset - base) as usize;
            if within >= len {
                return Err(Detail::FileIdentifier.corrupt());
            }
            if self.offset != Some(base) {
                self.offset = None;
                read_bytes(dev, info.len, base, &mut self.bytes[..len]).await?;
                self.offset = Some(base);
            }
            let take = (len - within).min(out.len());
            out[..take].copy_from_slice(&self.bytes[within..within + take]);
            out = &mut out[take..];
            offset += take as u64;
        }
        Ok(())
    }
}

/// Reads `buf.len()` bytes from byte `offset` of `dev`, which holds `len`
/// bytes.
async fn read_bytes<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<(), Error<D::Error>> {
    let end = offset
        .checked_add(buf.len() as u64)
        .ok_or(Detail::OutsideImage.corrupt())?;
    if end > len {
        return Err(Detail::OutsideImage.corrupt());
    }
    let bs = u64::from(dev.block_size().get());
    let mut done = 0;
    let mut pos = offset;
    while done < buf.len() {
        let block = BlockIndex::new(pos / bs);
        let within = (pos % bs) as usize;
        let left = buf.len() - done;
        if within == 0 && left as u64 >= bs {
            let whole = left - left % bs as usize;
            dev.read_blocks(block, &mut buf[done..done + whole])
                .await?;
            done += whole;
            pos += whole as u64;
        } else {
            let mut scratch = [0u8; MAX_BLOCK];
            let scratch = &mut scratch[..bs as usize];
            dev.read_blocks(block, scratch).await?;
            let take = (bs as usize - within).min(left);
            buf[done..done + take].copy_from_slice(&scratch[within..within + take]);
            done += take;
            pos += take as u64;
        }
    }
    Ok(())
}

async fn read_mapped<D: BlockDevice>(
    info: &Info, dev: &mut D, at: Location, mut within: u64, mut out: &mut [u8],
    mut window: Option<(&mut DirectoryWindow, bool)>,
) -> FsResult<(), D::Error> {
    let bs = u64::from(info.block_size);
    let mirrored = window.as_ref().is_some_and(|(_, mirror)| *mirror);
    while !out.is_empty() {
        let block = u32::try_from(within / bs).ok().and_then(|block| at.block.checked_add(block))
            .ok_or(Detail::Partition.corrupt())?;
        let at = Location { block, ..at };
        let local = within % bs;
        let main = info.run(at, mirrored);
        let alternate = info.run(at, !mirrored);
        let (offset, len) = main.or(alternate).ok_or(Detail::Partition.corrupt())?;
        let len = alternate.filter(|_| main.is_some()).map_or(len, |(_, mirror)| len.min(mirror));
        let take = (len - local).min(out.len() as u64) as usize;
        let chunk = &mut out[..take];
        let read = match window.as_mut() {
            Some((window, _)) => window.read(info, dev, offset + local, chunk).await,
            None => read_bytes(dev, info.len, offset + local, chunk).await,
        };
        if let Err(error) = read {
            let Some((other, _)) = alternate.filter(|&(other, _)| other != offset) else { return Err(error) };
            read_bytes(dev, info.len, other + local, chunk).await.map_err(|_| error)?;
        }
        within += take as u64;
        out = &mut out[take..];
    }
    Ok(())
}

async fn read_tagged<D: BlockDevice>(
    info: &Info, dev: &mut D, at: Location, data: &mut [u8], tags: &[u16], bad: Detail,
) -> FsResult<(u64, u16), D::Error> {
    let mut failure = None;
    for mirror in [false, true] {
        let Some((offset, _)) = info.run(at, mirror) else { continue };
        let read = read_bytes(dev, info.len, offset, data).await.and_then(|()| {
            let tag = check_tag(data, None, at.block).map_err(|_| bad.corrupt())?;
            let id = tag.identifier.get();
            tags.contains(&id).then_some((offset, id)).ok_or_else(|| bad.corrupt())
        });
        match read {
            Ok(value) => return Ok(value),
            Err(error) => { failure.get_or_insert(error); }
        }
    }
    Err(failure.unwrap_or_else(|| bad.corrupt()))
}

/// Finds an anchor at block 256, N-256 or N-1, or with `backup` at N-256
/// or N-1 only, trying logical block sizes from the device's up to 4096
/// bytes.
async fn find_anchor<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    device_block: u32,
    backup: bool,
) -> Result<(u32, AnchorVolumeDescriptorPointer), Error<D::Error>> {
    let mut sizes = [device_block, 2048, 512, 1024, 4096];
    for i in 1..sizes.len() {
        if sizes[..i].contains(&sizes[i]) {
            sizes[i] = 0;
        }
    }
    for size in sizes {
        if !(512..=MAX_BLOCK as u32).contains(&size) {
            continue;
        }
        let blocks = len / u64::from(size);
        let last = blocks.saturating_sub(1);
        let candidates = [u64::from(raw::ANCHOR_BLOCK), last.saturating_sub(256), last];
        for block in candidates {
            if block < u64::from(raw::ANCHOR_BLOCK) || block >= blocks {
                continue;
            }
            if backup && block == u64::from(raw::ANCHOR_BLOCK) {
                continue;
            }
            let Ok(location) = u32::try_from(block) else {
                continue;
            };
            let mut buf = [0u8; 512];
            read_bytes(dev, len, block * u64::from(size), &mut buf).await?;
            if check_tag(&buf, Some(tag::ANCHOR), location).is_ok() {
                return Ok((size, bytemuck::pod_read_unaligned(&buf)));
            }
        }
    }
    Err(Detail::Anchor.corrupt())
}

/// Whether the recognition sequence holds NSR03 rather than NSR02.
async fn recognition<D: BlockDevice>(dev: &mut D, len: u64, block_size: u32) -> Result<bool, Error<D::Error>> {
    let stride = u64::from(block_size.max(2048));
    let mut extended = false;
    let mut nsr = None;
    for i in 0..MAX_RECOGNITION {
        let offset = raw::VRS_START * 2048 + i * stride;
        if offset + 7 > len {
            break;
        }
        let mut head = [0u8; 7];
        read_bytes(dev, len, offset, &mut head).await?;
        let mut id = [0u8; 5];
        id.copy_from_slice(&head[1..6]);
        match id {
            vsd::BEA01 => extended = true,
            vsd::NSR02 if extended => nsr = Some(false),
            vsd::NSR03 if extended => nsr = Some(true),
            vsd::TEA01 => {
                if nsr.is_some() {
                    break;
                }
                extended = false;
            }
            vsd::CD001 | vsd::BOOT2 | vsd::NSR02 | vsd::NSR03 | [b'C', b'D', b'W', b'0', b'2'] => {}
            _ => break,
        }
    }
    nsr.ok_or(Detail::RecognitionSequence.error(ErrorKind::NotRecognized))
}

/// Reads one volume descriptor sequence, keeping the descriptors with the
/// highest sequence numbers.
async fn sequence<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    block_size: u32,
    start: raw::ExtentAd,
) -> Result<Sequence, Error<D::Error>> {
    let bs = block_size as usize;
    let mut found = Sequence {
        primary: None,
        logical: None,
        partitions: [(0, 0, PartitionInfo::default()); MAX_PARTITIONS],
        partition_count: 0,
    };
    let mut block = start.location.get();
    let mut remaining = start.length.get().div_ceil(block_size);
    let mut budget = MAX_SEQUENCE;
    let mut buf = [0u8; MAX_BLOCK];
    while remaining > 0 && budget > 0 {
        budget -= 1;
        remaining -= 1;
        let data = &mut buf[..bs];
        read_bytes(dev, len, u64::from(block) * u64::from(block_size), data).await?;
        if data[..Tag::SIZE].iter().all(|&b| b == 0) {
            break;
        }
        let tag = check_tag(data, None, block).map_err(|()| Detail::Descriptor.corrupt())?;
        let number = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        match tag.identifier.get() {
            tag::PRIMARY_VOLUME => {
                if found.primary.is_none_or(|(seen, _)| number >= seen) {
                    found.primary = Some((number, bytemuck::pod_read_unaligned(&data[..512])));
                }
            }
            tag::LOGICAL_VOLUME => {
                if found.logical.as_ref().is_none_or(|(seen, _)| number >= *seen) {
                    found.logical = Some((number, buf));
                }
            }
            tag::PARTITION => {
                let pd: raw::PartitionDescriptor = bytemuck::pod_read_unaligned(&data[..512]);
                let part = PartitionInfo::new(pd.number.get(), pd.start.get(), pd.length.get());
                let count = found.partition_count;
                match found.partitions[..count].iter_mut().find(|(n, _, _)| *n == part.number()) {
                    Some(slot) if number >= slot.1 => *slot = (part.number(), number, part),
                    Some(_) => {}
                    None if count < MAX_PARTITIONS => {
                        found.partitions[count] = (part.number(), number, part);
                        found.partition_count += 1;
                    }
                    None => return Err(Detail::PartitionMap.error(ErrorKind::Unsupported)),
                }
            }
            tag::VOLUME_POINTER => {
                let next: raw::ExtentAd = bytemuck::pod_read_unaligned(&data[20..28]);
                block = next.location.get();
                remaining = next.length.get().div_ceil(block_size);
                continue;
            }
            tag::TERMINATING => break,
            _ => {}
        }
        block = block.checked_add(1).ok_or(Detail::DescriptorSequence.corrupt())?;
    }
    if found.primary.is_none() || found.logical.is_none() || found.partition_count == 0 {
        return Err(Detail::DescriptorSequence.corrupt());
    }
    Ok(found)
}

/// What the last descriptor of the integrity sequence records: the free
/// blocks when it is closed, whether it is open, and when it was recorded.
#[derive(Default)]
struct Integrity {
    free: Option<u64>,
    open: Option<bool>,
    recorded: Option<hadris_fs::DateTime>,
}

/// Follows the integrity sequence to its last descriptor: each extent's
/// descriptors are recorded one per block until a terminating descriptor,
/// an unrecorded (zeroed) block or the extent's end, and the last one names
/// the next extent (ECMA-167 3/10.10). A damaged sequence, or a descriptor
/// with an unknown integrity type, records nothing.
async fn integrity<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    block_size: u32,
    mut next: raw::ExtentAd,
    partitions: &[PartitionInfo],
) -> Integrity {
    let mut found = Integrity::default();
    let mut buf = [0u8; MAX_BLOCK];
    let mut budget = MAX_INTEGRITY;
    while next.length.get() != 0 {
        let extent = next;
        next = raw::ExtentAd::default();
        let blocks = extent.length.get().div_ceil(block_size).max(1);
        for index in 0..blocks {
            let Some(left) = budget.checked_sub(1) else {
                return Integrity::default();
            };
            budget = left;
            let Some(block) = extent.location.get().checked_add(index) else {
                return Integrity::default();
            };
            let data = &mut buf[..block_size as usize];
            if read_bytes(dev, len, u64::from(block) * u64::from(block_size), data).await.is_err() {
                return Integrity::default();
            }
            let tag = match check_tag(data, None, block) {
                Ok(tag) => tag.identifier.get(),
                Err(()) if index > 0 && data.iter().all(|&byte| byte == 0) => break,
                Err(()) => return Integrity::default(),
            };
            match tag {
                tag::INTEGRITY => {}
                tag::TERMINATING => break,
                _ => return Integrity::default(),
            }
            let lvid: LogicalVolumeIntegrityDescriptor = bytemuck::pod_read_unaligned(&data[..80]);
            let count = (lvid.partition_count.get() as usize).min(partitions.len());
            let Some(table) = data.get(80..80 + 4 * count) else {
                return Integrity::default();
            };
            found.free = (lvid.integrity_type.get() == 1)
                .then(|| {
                    table
                        .chunks_exact(4)
                        .zip(partitions)
                        .filter(|(_, partition)| matches!(partition.kind(), PartitionKind::Physical | PartitionKind::Sparable))
                        .map(|(value, _)| value)
                        .map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
                        .try_fold(0u64, |sum, v| (v != u32::MAX).then_some(sum + u64::from(v)))
                })
                .flatten();
            found.open = match lvid.integrity_type.get() {
                0 => Some(true),
                1 => Some(false),
                _ => return Integrity::default(),
            };
            found.recorded = crate::time::to_datetime(&lvid.recorded);
            next = lvid.next;
        }
    }
    found
}

#[cfg(feature = "alloc")]
async fn load_maps<D: BlockDevice>(info: &mut Info, dev: &mut D, specs: &[MapSpec]) -> FsResult<(), D::Error> {
    for (index, spec) in specs.iter().copied().enumerate() {
        if let MapSpec::Sparable { packet, size, tables, count } = spec {
            let part = info.partitions()[index];
            info.maps[index].main = sparing_spans(info, dev, part, packet, size, &tables[..usize::from(count)]).await?;
        }
    }
    for (index, spec) in specs.iter().copied().enumerate() {
        let part = info.partitions()[index];
        if matches!(spec, MapSpec::Physical | MapSpec::Sparable { .. }) { continue; }
        let backing = info.partitions().iter().enumerate()
            .filter(|(_, p)| p.number() == part.number() && matches!(p.kind(), PartitionKind::Physical | PartitionKind::Sparable))
            .max_by_key(|(_, p)| p.kind() == PartitionKind::Sparable)
            .map(|(index, _)| index as u16)
            .ok_or(Detail::PartitionMap.corrupt())?;
        let (len, map) = match spec {
            MapSpec::Metadata { file, mirror } => metadata_map(info, dev, backing, file, mirror).await?,
            MapSpec::Virtual { v2 } => {
                let (len, main) = virtual_spans(info, dev, backing, v2).await?;
                (len, PartitionMap { main, mirror: alloc::vec::Vec::new() })
            }
            _ => continue,
        };
        info.volume.partitions[index].set_map(spec.kind(), len);
        info.maps[index] = map;
    }
    Ok(())
}

#[cfg(feature = "alloc")]
async fn sparing_spans<D: BlockDevice>(
    info: &Info, dev: &mut D, part: PartitionInfo, packet: u32, size: u32, tables: &[u32],
) -> FsResult<alloc::vec::Vec<PartitionSpan>, D::Error> {
    let bad = || Detail::PartitionMap.corrupt();
    let size = u64::from(size).min(56 + u64::from(u16::MAX) * 8) as usize;
    if size < 56 { return Err(bad()); }
    let mut bytes = alloc::vec::Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| ErrorKind::LimitExceeded)?;
    bytes.resize(size, 0);
    let bs = u64::from(info.block_size);
    let mut best = None;
    let mut failure = None;
    for &block in tables {
        let offset = u64::from(block) * bs;
        let table = read_bytes(dev, info.len, offset, &mut bytes).await
            .and_then(|()| sparing_table(&bytes, block, part, packet, info.len / bs));
        match table {
            Ok((seq, spans)) if best.as_ref().is_none_or(|(seen, _)| seq > *seen) => best = Some((seq, spans)),
            Err(error) => { failure.get_or_insert(error); }
            _ => {}
        }
    }
    best.map(|(_, spans)| spans).ok_or_else(|| failure.unwrap_or_else(bad))
}

#[cfg(feature = "alloc")]
fn sparing_table<E>(
    bytes: &[u8], location: u32, part: PartitionInfo, packet: u32, blocks: u64,
) -> FsResult<(u32, alloc::vec::Vec<PartitionSpan>), E> {
    let bad = || Detail::PartitionMap.corrupt();
    check_tag(bytes, Some(0), location).map_err(|_| bad())?;
    let id: EntityId = bytemuck::pod_read_unaligned(&bytes[16..48]);
    if id.name() != b"*UDF Sparing Table" { return Err(bad()); }
    let count = usize::from(u16::from_le_bytes([bytes[48], bytes[49]]));
    let seq = u32::from_le_bytes([bytes[52], bytes[53], bytes[54], bytes[55]]);
    let entries = bytes.get(56..56 + count * 8).ok_or_else(bad)?;
    let base = u64::from(part.start());
    let mut spans = alloc::vec::Vec::new();
    let mut next = 0;
    for entry in entries.chunks_exact(8) {
        let original = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let physical = u64::from(u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]));
        if original >= 0xFFFF_FFF0 { continue; }
        if original < next || original % packet != 0 || original.checked_add(packet).is_none_or(|end| end > part.len())
            || physical + u64::from(packet) > blocks
        {
            return Err(bad());
        }
        add_span(&mut spans, next, base + u64::from(next), original - next)?;
        add_span(&mut spans, original, physical, packet)?;
        next = original + packet;
    }
    add_span(&mut spans, next, base + u64::from(next), part.len() - next)?;
    Ok((seq, spans))
}

#[cfg(feature = "alloc")]
async fn metadata_map<D: BlockDevice>(
    info: &Info, dev: &mut D, backing: u16, file: u32, mirror: u32,
) -> FsResult<(u32, PartitionMap), D::Error> {
    let main = metadata_spans(info, dev, Location { partition: backing, block: file }, 0xFA).await;
    let backup = metadata_spans(info, dev, Location { partition: backing, block: mirror }, 0xFB).await;
    match (main, backup) {
        (Ok((len, main)), Ok((other, mirror))) if len == other => Ok((len, PartitionMap { main, mirror })),
        (Ok((len, main)), _) => Ok((len, PartitionMap { main, mirror: alloc::vec::Vec::new() })),
        (Err(_), Ok((len, mirror))) => Ok((len, PartitionMap { main: alloc::vec::Vec::new(), mirror })),
        (Err(error), Err(_)) => Err(error),
    }
}

#[cfg(feature = "alloc")]
async fn metadata_spans<D: BlockDevice>(
    info: &Info, dev: &mut D, at: Location, kind: u8,
) -> FsResult<(u32, alloc::vec::Vec<PartitionSpan>), D::Error> {
    let bad = || Detail::PartitionMap.corrupt();
    let icb = icb_at(info, dev, at).await?;
    let bs = u64::from(info.block_size);
    if icb.file_type != kind || icb.allocation() != allocation::SHORT || icb.size == 0 || icb.size % bs != 0 {
        return Err(bad());
    }
    let blocks = u32::try_from(icb.size / bs).map_err(|_| bad())?;
    let mut spans = alloc::vec::Vec::new();
    let mut file = 0;
    let mut walk = Walk::new(&icb);
    while file < blocks {
        let piece = walk.next(info, dev, &icb).await?.ok_or_else(bad)?;
        if piece.len() % bs != 0 { return Err(bad()); }
        let len = ((piece.len() / bs) as u32).min(blocks - file);
        if let Piece::Disk { mut location, .. } = piece {
            let mut done = 0;
            while done < len {
                let (offset, run) = info.run(location, false).ok_or_else(bad)?;
                let take = (run / bs).min(u64::from(len - done)) as u32;
                add_span(&mut spans, file + done, offset / bs, take)?;
                done += take;
                if done < len {
                    location.block = location.block.checked_add(take).ok_or_else(bad)?;
                }
            }
        }
        file += len;
    }
    if spans.is_empty() { return Err(bad()); }
    Ok((blocks, spans))
}

#[cfg(feature = "alloc")]
async fn virtual_spans<D: BlockDevice>(
    info: &Info, dev: &mut D, backing: u16, v2: bool,
) -> FsResult<(u32, alloc::vec::Vec<PartitionSpan>), D::Error> {
    let part = info.partitions()[usize::from(backing)];
    let media_end = (info.len / u64::from(info.block_size)).saturating_sub(u64::from(part.start()));
    let media_end = u32::try_from(media_end).unwrap_or(u32::MAX);
    let end = media_end.min(part.len());
    let bad = || Detail::PartitionMap.corrupt();
    let mut failure = None;
    for block in (end.saturating_sub(4)..end).rev() {
        let at = Location { partition: backing, block };
        let result = match icb_at(info, dev, at).await {
            Ok(icb) if icb.file_type == if v2 { 0xF8 } else { 0 } => read_vat(info, dev, &icb, v2).await,
            Ok(_) => Err(bad()),
            Err(error) => Err(error),
        };
        match result {
            Ok(vat) => return Ok(vat),
            Err(error) if error.kind() == ErrorKind::Io => { failure.get_or_insert(error); }
            Err(_) => {}
        }
    }
    Err(failure.unwrap_or_else(bad))
}

#[cfg(feature = "alloc")]
async fn read_vat<D: BlockDevice>(
    info: &Info, dev: &mut D, icb: &Icb, v2: bool,
) -> FsResult<(u32, alloc::vec::Vec<PartitionSpan>), D::Error> {
    let bad = || Detail::PartitionMap.corrupt();
    let mut resume = Resume::default();
    let mut buf = [0; MAX_BLOCK];
    let (mut pos, end) = if v2 {
        read_exact(info, dev, icb, &mut resume, 0, &mut buf[..4], Detail::PartitionMap).await?;
        let start = u64::from(u16::from_le_bytes([buf[0], buf[1]]));
        let implementation = u64::from(u16::from_le_bytes([buf[2], buf[3]]));
        if start != 152 + implementation || start > icb.size { return Err(bad()); }
        (start, icb.size)
    } else {
        let end = icb.size.checked_sub(36).ok_or_else(bad)?;
        read_exact(info, dev, icb, &mut resume, end, &mut buf[..32], Detail::PartitionMap).await?;
        let id: EntityId = bytemuck::pod_read_unaligned(&buf[..32]);
        if id.name() != b"*UDF Virtual Alloc Tbl" { return Err(bad()); }
        resume = Resume::default();
        (0, end)
    };
    let bytes = end - pos;
    if bytes == 0 || bytes % 4 != 0 { return Err(bad()); }
    let count = u32::try_from(bytes / 4).map_err(|_| bad())?;
    let mut spans = alloc::vec::Vec::new();
    let mut block = 0;
    let bs = u64::from(info.block_size);
    while pos < end {
        let len = (end - pos).min(buf.len() as u64) as usize;
        read_exact(info, dev, icb, &mut resume, pos, &mut buf[..len], Detail::PartitionMap).await?;
        for entry in buf[..len].chunks_exact(4) {
            let physical = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
            if physical != u32::MAX {
                let at = Location { partition: icb.at.partition, block: physical };
                let (offset, _) = info.run(at, false).ok_or_else(bad)?;
                add_span(&mut spans, block, offset / bs, 1)?;
            }
            block += 1;
        }
        pos += len as u64;
    }
    Ok((count, spans))
}

/// Reads the volume structures of `dev`; with `backup` from the anchors at
/// the end of the volume and the reserve descriptor sequence.
async fn mount<D: BlockDevice>(dev: &mut D, backup: bool) -> Result<Info, Error<D::Error>> {
    let device_block = dev.block_size().get();
    if device_block as usize > MAX_BLOCK {
        return Err(Detail::BlockSize.error(ErrorKind::Unsupported));
    }
    let len = dev.block_count().saturating_mul(u64::from(device_block));
    let anchor = find_anchor(dev, len, device_block, backup).await;
    let probe = anchor.as_ref().map_or(2048, |(block_size, _)| *block_size);
    let nsr03 = recognition(dev, len, probe).await?;
    let (block_size, anchor) = anchor?;
    let (first, second) = match backup {
        true => (anchor.reserve, anchor.main),
        false => (anchor.main, anchor.reserve),
    };
    let found = match sequence(dev, len, block_size, first).await {
        Ok(found) => found,
        Err(err) if err.kind() == ErrorKind::Io => return Err(err),
        Err(err) => match sequence(dev, len, block_size, second).await {
            Ok(found) => found,
            Err(fallback) if fallback.kind() == ErrorKind::Io => return Err(fallback),
            Err(_) => return Err(err),
        },
    };

    let Some((_, lvd_block)) = found.logical else {
        return Err(Detail::DescriptorSequence.corrupt());
    };
    let lvd: LogicalVolumeDescriptor = bytemuck::pod_read_unaligned(&lvd_block[..440]);
    if lvd.block_size.get() != block_size {
        return Err(Detail::BlockSize.corrupt());
    }
    let maps = usize::try_from(lvd.map_table_length.get())
        .ok()
        .and_then(|len| len.checked_add(440))
        .filter(|&end| end <= block_size as usize)
        .and_then(|end| lvd_block.get(440..end))
        .ok_or(Detail::Descriptor.corrupt())?;
    let mut partitions = [PartitionInfo::default(); MAX_PARTITIONS];
    #[cfg(feature = "alloc")]
    let mut specs = [MapSpec::Physical; MAX_PARTITIONS];
    let revision = domain_revision(&lvd.domain, nsr03).unwrap_or(if nsr03 {
        UdfRevision::V2_01
    } else {
        UdfRevision::V1_02
    });
    let mut count = 0;
    let mut at = 0;
    for _ in 0..lvd.map_count.get() {
        let (kind, map_len) = match maps.get(at..at + 2) {
            Some(head) => (head[0], usize::from(head[1])),
            None => return Err(Detail::Descriptor.corrupt()),
        };
        let map = maps
            .get(at..at + map_len)
            .filter(|_| map_len >= 2)
            .ok_or(Detail::Descriptor.corrupt())?;
        if count == MAX_PARTITIONS {
            return Err(Detail::PartitionMap.error(ErrorKind::Unsupported));
        }
        let number = match (kind, map_len) {
            (1, 6) => u16::from_le_bytes([map[4], map[5]]),
            #[cfg(feature = "alloc")]
            (2, 64) => {
                specs[count] = map_spec(map, revision)?;
                u16::from_le_bytes([map[38], map[39]])
            }
            #[cfg(not(feature = "alloc"))]
            (2, _) => return Err(Detail::PartitionMap.error(ErrorKind::Unsupported)),
            _ => return Err(Detail::Descriptor.corrupt()),
        };
        let (_, _, part) = found.partitions[..found.partition_count]
            .iter()
            .find(|(n, _, _)| *n == number)
            .ok_or(Detail::Partition.corrupt())?;
        partitions[count] = *part;
        #[cfg(feature = "alloc")]
        partitions[count].set_map(specs[count].kind(), part.len());
        count += 1;
        at += map_len;
    }
    let (_, pvd) = found.primary.ok_or(Detail::DescriptorSequence.corrupt())?;
    let mut info = Info {
        block_size,
        len,
        root: Location { partition: 0, block: 0 },
        free_blocks: None,
        #[cfg(feature = "alloc")]
        maps: core::array::from_fn(|_| Default::default()),
        volume: VolumeInfo {
            revision,
            block_size,
            partitions,
            partition_count: count,
            implementation: Entity::from_raw(lvd.implementation),
            domain: Entity::from_raw(lvd.domain),
            volume: Identifier::decode(&pvd.volume_identifier),
            volume_set: Identifier::decode(&pvd.volume_set_identifier),
            logical_volume: Identifier::decode(&lvd.logical_volume_identifier),
            file_set: Identifier::decode(&[]),
            recorded: crate::time::to_datetime(&pvd.recorded),
            integrity_recorded: None,
            was_dirty: None,
        },
    };
    #[cfg(feature = "alloc")]
    load_maps(&mut info, dev, &specs[..count]).await?;

    let fsd_at: LongAd = bytemuck::pod_read_unaligned(&lvd.contents_use);
    let fsd_at = Location::from_long(&fsd_at);
    let mut buf = [0u8; MAX_BLOCK];
    let data = &mut buf[..block_size as usize];
    read_tagged(&info, dev, fsd_at, data, &[tag::FILE_SET], Detail::FileSet).await?;
    let fsd: FileSetDescriptor = bytemuck::pod_read_unaligned(&data[..512]);
    info.root = Location::from_long(&fsd.root);
    info.volume.file_set = Identifier::decode(&fsd.file_set_identifier);
    let root = icb_at(&info, dev, info.root).await.map_err(|err| match err.kind() {
        ErrorKind::Io => err,
        _ => Detail::FileSet.corrupt(),
    })?;
    if !root.is_dir() {
        return Err(Detail::FileSet.corrupt());
    }
    let integrity = integrity(dev, len, block_size, lvd.integrity_sequence, info.partitions()).await;
    info.free_blocks = integrity.free;
    info.volume.was_dirty = integrity.open;
    info.volume.integrity_recorded = integrity.recorded;
    Ok(info)
}

/// Reads and decodes the entry at `at`.
async fn icb_at<D: BlockDevice>(info: &Info, dev: &mut D, at: Location) -> Result<Icb, Error<D::Error>> {
    let bs = info.block_size as usize;
    info.check_range(at, bs as u64)?;
    let mut block = [0u8; MAX_BLOCK];
    let (offset, id) = read_tagged(info, dev, at, &mut block[..bs], &[tag::FILE_ENTRY, tag::EXTENDED_FILE_ENTRY], Detail::Icb).await?;
    Icb::parse(at, offset, id, block, bs)
}

impl Walk {
    fn new(icb: &Icb) -> Self {
        Self {
            aed: [0; MAX_BLOCK],
            aed_at: None,
            pos: icb.ad_start,
            end: icb.ad_start + icb.ad_len,
            steps: 0,
            hops: 0,
            done: false,
        }
    }

    /// Resumes a newly initialized walk of `icb`, or keeps its first
    /// descriptor when `resume` is of another entry or after `offset`. Returns the byte
    /// of the data the next piece starts at.
    async fn resume<D: BlockDevice>(&mut self, info: &Info, dev: &mut D, icb: &Icb, resume: &Resume, offset: u64) -> Result<u64, Error<D::Error>> {
        if resume.icb != Some(icb.at) || resume.offset > offset {
            return Ok(0);
        }
        if let Some(at) = resume.aed_at {
            self.load(info, dev, at).await?;
        }
        self.pos = resume.pos;
        self.end = resume.end;
        self.steps = resume.steps;
        self.hops = resume.hops;
        Ok(resume.offset)
    }

    /// Where the walk of `icb` stands, before the piece at byte `offset`.
    fn mark(&self, icb: &Icb, offset: u64) -> Resume {
        Resume {
            icb: Some(icb.at),
            aed_at: self.aed_at,
            pos: self.pos,
            end: self.end,
            steps: self.steps,
            hops: self.hops,
            offset,
            location: None,
        }
    }

    /// Reads the allocation extent descriptor at `at` and returns the end
    /// of its descriptors.
    async fn load<D: BlockDevice>(&mut self, info: &Info, dev: &mut D, at: Location) -> Result<usize, Error<D::Error>> {
        let bad = || Detail::AllocationDescriptor.corrupt();
        let bs = info.block_size as usize;
        info.check_range(at, bs as u64)?;
        read_tagged(info, dev, at, &mut self.aed[..bs], &[tag::ALLOCATION_EXTENT], Detail::AllocationDescriptor).await?;
        let aed: AllocationExtentDescriptor = bytemuck::pod_read_unaligned(&self.aed[..24]);
        let ads = aed.allocation_descriptors_length.get() as usize;
        let end = 24usize.checked_add(ads).filter(|&end| end <= bs).ok_or_else(bad)?;
        self.aed_at = Some(at);
        Ok(end)
    }

    /// The next stretch of data, or `None` after the last descriptor.
    async fn next<D: BlockDevice>(&mut self, info: &Info, dev: &mut D, icb: &Icb) -> Result<Option<Piece>, Error<D::Error>> {
        if icb.flags.contains(IcbFlags::TRANSFORMED) {
            return Err(Detail::AllocationDescriptor.error(ErrorKind::Unsupported));
        }
        let bad = || Detail::AllocationDescriptor.corrupt();
        loop {
            if self.done {
                return Ok(None);
            }
            let size = match icb.allocation() {
                allocation::EMBEDDED => {
                    self.done = true;
                    return Ok(Some(Piece::Embedded { start: icb.ad_start, len: icb.ad_len }));
                }
                allocation::SHORT => 8,
                allocation::LONG => 16,
                allocation::EXTENDED => 20,
                _ => return Err(bad()),
            };
            if self.pos + size > self.end {
                self.done = true;
                return Ok(None);
            }
            self.steps += 1;
            if self.steps > MAX_DESCRIPTORS {
                return Err(bad());
            }
            let source = if self.aed_at.is_some() { &self.aed[..] } else { &icb.block[..] };
            let bytes = &source[self.pos..self.pos + size];
            let (length, information, kind, at) = match size {
                8 => {
                    let ad: ShortAd = bytemuck::pod_read_unaligned(bytes);
                    let at = Location { partition: icb.at.partition, block: ad.position.get() };
                    (ad.len(), ad.len(), ad.extent_type(), at)
                }
                16 => {
                    let ad: LongAd = bytemuck::pod_read_unaligned(bytes);
                    (ad.len(), ad.len(), ad.extent_type(), Location::from_long(&ad))
                }
                _ => {
                    let ad: ExtAd = bytemuck::pod_read_unaligned(bytes);
                    let recorded = ad.recorded_length.get();
                    let information = ad.information_length.get();
                    if recorded > ad.len() {
                        return Err(bad());
                    }
                    match ad.extent_type() {
                        extent::RECORDED if recorded != information => {
                            return Err(Detail::AllocationDescriptor.error(ErrorKind::Unsupported));
                        }
                        extent::ALLOCATED | extent::UNALLOCATED if recorded != 0 || information > ad.len() => {
                            return Err(bad());
                        }
                        _ => {}
                    }
                    let at = Location {
                        partition: ad.location.partition.get(),
                        block: ad.location.block.get(),
                    };
                    (ad.len(), information, ad.extent_type(), at)
                }
            };
            self.pos += size;
            if length == 0 {
                if information != 0 {
                    return Err(bad());
                }
                self.done = true;
                return Ok(None);
            }
            let length = u64::from(length);
            match kind {
                extent::CONTINUATION => {
                    self.hops += 1;
                    if self.hops > MAX_CONTINUATIONS {
                        return Err(bad());
                    }
                    let end = self.load(info, dev, at).await?;
                    if end as u64 > length.max(24) {
                        return Err(bad());
                    }
                    self.pos = 24;
                    self.end = end;
                }
                extent::RECORDED => {
                    info.check_range(at, length)?;
                    if information != 0 {
                        return Ok(Some(Piece::Disk { location: at, len: u64::from(information) }));
                    }
                }
                extent::ALLOCATED => {
                    let location = info.check_range::<D::Error>(at, length).is_ok().then_some(at);
                    if information != 0 {
                        return Ok(Some(Piece::Zero { len: u64::from(information), location }));
                    }
                }
                _ if information != 0 => return Ok(Some(Piece::Zero { len: u64::from(information), location: None })),
                _ => {}
            }
        }
    }
}

/// Reads up to `buf.len()` bytes of the data of `icb` from `offset`;
/// fewer only at the end of the data. Resumes the walk `resume` records
/// and records where this read started in it.
async fn read_stream<D: BlockDevice>(
    info: &Info,
    dev: &mut D,
    icb: &Icb,
    resume: &mut Resume,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, Error<D::Error>> {
    read_stream_window(info, dev, icb, resume, offset, buf, None).await
}

async fn read_stream_window<D: BlockDevice>(
    info: &Info,
    dev: &mut D,
    icb: &Icb,
    resume: &mut Resume,
    offset: u64,
    buf: &mut [u8],
    mut window: Option<(&mut DirectoryWindow, bool)>,
) -> Result<usize, Error<D::Error>> {
    if offset >= icb.size {
        return Ok(0);
    }
    let want = usize::try_from(icb.size - offset).unwrap_or(usize::MAX).min(buf.len());
    let mut walk = Walk::new(icb);
    let mut pos = walk.resume(info, dev, icb, resume, offset).await?;
    let mut done = 0usize;
    while done < want {
        let mark = walk.mark(icb, pos);
        let Some(piece) = walk.next(info, dev, icb).await? else {
            return Err(Detail::AllocationDescriptor.corrupt());
        };
        let len = piece.len();
        let at = offset + done as u64;
        if at < pos + len {
            if done == 0 {
                *resume = mark;
                resume.location = match piece {
                    Piece::Disk { location, .. } => location.block.checked_add(((at - pos) / u64::from(info.block_size)) as u32),
                    Piece::Embedded { .. } => Some(icb.at.block),
                    Piece::Zero { .. } => None,
                };
            }
            let within = at - pos;
            let take = usize::try_from(len - within).unwrap_or(usize::MAX).min(want - done);
            let out = &mut buf[done..done + take];
            match piece {
                Piece::Disk { location, .. } => {
                    let window = window.as_mut().map(|(window, mirror)| (&mut **window, *mirror));
                    read_mapped(info, dev, location, within, out, window).await?;
                }
                Piece::Zero { .. } => out.fill(0),
                Piece::Embedded { start, .. } => {
                    let start = start + within as usize;
                    out.copy_from_slice(&icb.block[start..start + take]);
                }
            }
            done += take;
        }
        pos += len;
    }
    Ok(done)
}

/// Fills `buf` from `offset` of the data of `icb`, failing with `bad` when
/// the data ends first.
async fn read_exact<D: BlockDevice>(
    info: &Info,
    dev: &mut D,
    icb: &Icb,
    resume: &mut Resume,
    offset: u64,
    buf: &mut [u8],
    bad: Detail,
) -> Result<(), Error<D::Error>> {
    if read_stream(info, dev, icb, resume, offset, buf).await? != buf.len() {
        return Err(bad.corrupt());
    }
    Ok(())
}

/// The file identifier at `pos` in directory `dir`.
async fn fid_at<D: BlockDevice>(info: &Info, dev: &mut D, dir: &Icb, resume: &mut Resume, window: &mut DirectoryWindow, pos: u64) -> Result<Fid, Error<D::Error>> {
    let first = fid_at_copy(info, dev, dir, resume, window, pos, false).await;
    if first.is_err() && info.mirrored() {
        return fid_at_copy(info, dev, dir, resume, window, pos, true).await.or(first);
    }
    first
}

async fn fid_at_copy<D: BlockDevice>(info: &Info, dev: &mut D, dir: &Icb, resume: &mut Resume, window: &mut DirectoryWindow, pos: u64, mirror: bool) -> Result<Fid, Error<D::Error>> {
    let bad = || Detail::FileIdentifier.corrupt();
    let mut buf = [0u8; FID_BUFFER];
    if read_stream_window(info, dev, dir, resume, pos, &mut buf[..38], Some((window, mirror))).await? != 38 {
        return Err(bad());
    }
    let fid: FileIdentifierDescriptor = bytemuck::pod_read_unaligned(&buf[..38]);
    let tag = fid.tag;
    if !tag.is_checksum_valid()
        || tag.identifier.get() != tag::FILE_IDENTIFIER
        || !matches!(tag.version.get(), 2 | 3)
        || tag.reserved != 0
        || Some(tag.location.get()) != resume.location
        || !(1..=32767).contains(&fid.version.get())
        || fid.characteristics & 0xF0 != 0
        || fid.implementation_use_length.get() % 4 != 0
        || matches!(fid.implementation_use_length.get(), 1..=31)
        || (fid.characteristics & FileCharacteristics::PARENT.bits() != 0
            && (fid.identifier_length != 0 || fid.characteristics & FileCharacteristics::DIRECTORY.bits() == 0))
    {
        return Err(bad());
    }
    let total = fid.total_len() as u64;
    let next = pos.checked_add(total).filter(|next| *next <= dir.size).ok_or_else(bad)?;
    let covered = u64::from(tag.crc_length.get());
    if covered + 16 > total {
        return Err(bad());
    }
    let crc_end = 16 + covered;
    let mut crc = raw::crc16_update(0, &buf[16..crc_end.min(38) as usize]);
    let name_at = 38 + u64::from(fid.implementation_use_length.get());
    let name_len = usize::from(fid.identifier_length);
    let name_end = name_at + name_len as u64;
    let characteristics = FileCharacteristics::from_bits_retain(fid.characteristics);
    let want_name = name_len > 0 && !characteristics.intersects(FileCharacteristics::DELETED | FileCharacteristics::PARENT);
    let tail_at = if want_name { name_at } else { name_end };
    let end = total;
    let mut name = [0u8; 255];
    let mut at = 38u64;
    while at < end {
        if at >= crc_end && at < tail_at {
            at = tail_at;
        }
        if at >= end {
            break;
        }
        let chunk_end = if at < crc_end && tail_at > crc_end { crc_end } else { end };
        let take = (chunk_end - at).min(FID_BUFFER as u64) as usize;
        if read_stream_window(info, dev, dir, resume, pos + at, &mut buf[..take], Some((window, mirror))).await? != take {
            return Err(bad());
        }
        if at < crc_end {
            crc = raw::crc16_update(crc, &buf[..(crc_end - at).min(take as u64) as usize]);
        }
        let start = at.max(name_at);
        let end = (at + take as u64).min(name_end);
        if want_name && start < end {
            name[(start - name_at) as usize..(end - name_at) as usize]
                .copy_from_slice(&buf[(start - at) as usize..(end - at) as usize]);
        }
        let padding_at = at.max(name_end);
        if padding_at < at + take as u64
            && buf[(padding_at - at) as usize..take].iter().any(|&byte| byte != 0)
        {
            return Err(bad());
        }
        at += take as u64;
    }
    if crc != tag.crc.get() {
        return Err(bad());
    }
    Ok(Fid {
        characteristics,
        icb: Location::from_long(&fid.icb),
        name,
        name_len,
        next,
    })
}

/// The name of `fid`, decoded into `out`.
fn fid_name<E>(fid: &Fid, out: &mut [u8]) -> Result<usize, Error<E>> {
    if fid.name_len == 0 {
        return Err(Detail::FileIdentifier.corrupt());
    }
    crate::name::decode_name(&fid.name[..fid.name_len], out).ok_or(Detail::FileIdentifier.corrupt())
}

/// Writes the target of the symlink `icb` into `out`.
async fn link_target<D: BlockDevice>(info: &Info, dev: &mut D, icb: &Icb, out: &mut [u8]) -> FsResult<usize, D::Error> {
    let mut resume = Resume::default();
    let mut len = 0usize;
    let mut pos = 0u64;
    let push = |out: &mut [u8], len: &mut usize, bytes: &[u8]| -> Result<(), ErrorKind> {
        if *len > 0 && out[*len - 1] != b'/' {
            *out.get_mut(*len).ok_or(ErrorKind::LimitExceeded)? = b'/';
            *len += 1;
        }
        out.get_mut(*len..*len + bytes.len())
            .ok_or(ErrorKind::LimitExceeded)?
            .copy_from_slice(bytes);
        *len += bytes.len();
        Ok(())
    };
    while pos < icb.size {
        let mut head = [0u8; 4];
        read_exact(info, dev, icb, &mut resume, pos, &mut head, Detail::PathComponent).await?;
        let component: PathComponent = bytemuck::pod_read_unaligned(&head);
        pos += 4;
        let mut ident = [0u8; 255];
        let ident = &mut ident[..usize::from(component.identifier_length)];
        read_exact(info, dev, icb, &mut resume, pos, ident, Detail::PathComponent).await?;
        pos += ident.len() as u64;
        match component.component_type {
            1 | 2 => {
                *out.get_mut(0).ok_or(ErrorKind::LimitExceeded)? = b'/';
                len = 1;
            }
            3 => push(out, &mut len, b"..")?,
            4 => push(out, &mut len, b".")?,
            5 => {
                let mut name = [0u8; 1024];
                let n = crate::name::decode_cs0(ident, &mut name)
                    .filter(|&n| n > 0)
                    .ok_or(Detail::PathComponent.corrupt())?;
                push(out, &mut len, &name[..n])?;
            }
            _ => return Err(Detail::PathComponent.corrupt()),
        }
    }
    Ok(len)
}

/// An open UDF volume on a block device.
///
/// It reads the volume structures once, when opened, and needs no
/// allocator. A fixed 4 KiB directory-sector buffer reuses identifier bytes;
/// file data and file entries are read directly. It implements the
/// `hadris_fs` `FileSystem` trait read-only:
/// node ids are ICB locations, so they are stable and `forget` does nothing; a directory's id is the id its
/// name in the parent lists, and hard links share one id. An id outside
/// every partition fails with [`ErrorKind::InvalidHandle`]; one inside a
/// partition is read as an entry, and a damaged entry fails with
/// [`ErrorKind::Corrupt`]. Write methods fail with [`ErrorKind::ReadOnly`].
///
/// ```rust,no_run
/// # #[cfg(all(feature = "sync", feature = "std"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use hadris_udf::sync::UdfFs;
/// use hadris_fs::sync::FileSystem;
/// use hadris_fs::{MountOptions, Resolve};
/// # let dev = hadris_storage::MemDevice::new(vec![0; 1024 * 1024], hadris_storage::BlockSize::new(512).unwrap());
/// let mut udf = UdfFs::mount(dev, MountOptions::new())?;
/// let file = udf.resolve(b"/docs/readme.txt", Resolve::Lexical)?;
/// let mut buf = [0u8; 64];
/// let n = udf.read(file, 0, &mut buf)?;
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sync", feature = "std")))]
/// # fn main() {}
/// ```
#[derive(Debug)]
pub struct UdfFs<D> {
    dev: D,
    info: Info,
    /// The walk of the directory read last.
    dir_walk: Resume,
    /// The walk of the file read last.
    file_walk: Resume,
    directory_window: DirectoryWindow,
}

impl<D: BlockDevice> UdfFs<D> {
    /// Mounts the volume on `dev`, read-only in every version: finds an
    /// anchor, checks the recognition sequence, and reads the prevailing
    /// volume descriptors, from the reserve sequence when the main one is
    /// damaged, and the file set. With [`MountOptions::backup_boot`] it
    /// skips the anchor at block 256 for those at the end of the volume
    /// and reads the reserve sequence first. Other options do not apply.
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorKind::NotRecognized`] without a UDF recognition
    /// sequence, with [`ErrorKind::Corrupt`] when the structures are
    /// invalid, and with [`ErrorKind::Unsupported`] for partition maps
    /// other than type 1 or device blocks above 4096 bytes. The device
    /// comes back in the [`MountError`]. Device failures, including reads of
    /// the reserve sequence, retain their [`ErrorKind::Io`] and device error.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all))]
    pub async fn mount(mut dev: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        match mount(&mut dev, options.is_backup_boot()).await {
            Ok(info) => Ok(Self { dev, info, dir_walk: Resume::default(), file_walk: Resume::default(), directory_window: DirectoryWindow::new() }),
            Err(err) => Err(MountError::new(err, dev)),
        }
    }

    /// Gives the device back. The volume is read-only, so there is nothing
    /// to sync and this never fails.
    ///
    /// # Errors
    ///
    /// This read-only unmount always succeeds and performs no I/O.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all))]
    pub async fn unmount(self) -> Result<D, MountError<D, D::Error>> {
        Ok(self.dev)
    }

    /// What the volume descriptors record: the revision, block size,
    /// partitions, identifiers, domain, implementation and times.
    pub fn info(&self) -> &VolumeInfo {
        &self.info.volume
    }

    /// Whether the logical volume integrity descriptor was open at mount:
    /// the volume was not cleanly closed. `None` when the integrity
    /// sequence is empty, damaged, cannot be read or records an unknown
    /// integrity type, so the state is unknown.
    pub fn was_dirty(&self) -> Option<bool> {
        self.info.volume.was_dirty
    }

    /// Reads `buf.len()` bytes from byte `offset` of the device.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Corrupt`] if the requested range is outside the
    /// mounted image, or propagates device failures. An error may leave
    /// `buf` partly modified.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(offset = offset, bytes = buf.len())))]
    pub async fn read_raw(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error<D::Error>> {
        read_bytes(&mut self.dev, self.info.len, offset, buf).await
    }

    /// Borrows the device.
    pub fn device(&self) -> &D {
        &self.dev
    }

    /// Returns the device.
    pub fn into_inner(self) -> D {
        self.dev
    }

    async fn icb(&mut self, node: NodeId) -> FsResult<Icb, D::Error> {
        let at = Location::of(node)
            .filter(|at| self.info.check_range::<D::Error>(*at, 1).is_ok())
            .ok_or(ErrorKind::InvalidHandle)?;
        icb_at(&self.info, &mut self.dev, at).await
    }

    async fn dir(&mut self, node: NodeId) -> FsResult<Icb, D::Error> {
        let icb = self.icb(node).await?;
        if !icb.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok(icb)
    }

    async fn icb_metadata(&mut self, icb: &Icb) -> FsResult<Metadata, D::Error> {
        let file_type = icb.fs_type().ok_or(Detail::Icb.corrupt())?;
        let mut meta = icb.metadata(file_type);
        if file_type == FileType::Symlink {
            let mut target = [0u8; 4096];
            let len = link_target(&self.info, &mut self.dev, icb, &mut target).await?;
            meta = meta.with_len(len as u64);
        }
        Ok(meta)
    }

    /// Maps a node's data to the device: fills `out` with its extents
    /// that end after byte `from` of the data, in order, and returns how
    /// many it filled. Call again from the end of the last one for more; 0
    /// means there are none. Allocated but unrecorded extents are marked
    /// unwritten, holes are left out, and data embedded in the file entry
    /// is one extent inside it. Transformed content returns [`ErrorKind::Unsupported`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidHandle`] for a node outside the volume,
    /// [`ErrorKind::Corrupt`] for damaged entries or allocation descriptors,
    /// or a device error. `out` may contain extents found before failure.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(node = ?node)))]
    pub async fn extents(&mut self, node: NodeId, from: u64, out: &mut [Extent]) -> FsResult<usize, D::Error> {
        let icb = self.icb(node).await?;
        let mut walk = Walk::new(&icb);
        let mut pos = 0u64;
        let mut count = 0;
        while pos < icb.size {
            let Some(piece) = walk.next(&self.info, &mut self.dev, &icb).await? else {
                return Err(Detail::AllocationDescriptor.corrupt());
            };
            let len = piece.len().min(icb.size - pos);
            let disk = match piece {
                Piece::Disk { location, .. } => Some((location, false)),
                Piece::Zero { location, .. } => location.map(|location| (location, true)),
                Piece::Embedded { start, .. } => {
                    if pos + len > from && len != 0 {
                        let Some(slot) = out.get_mut(count) else { return Ok(count) };
                        *slot = Extent::new(icb.disk_offset + start as u64, len).with_file_offset(pos);
                        count += 1;
                    }
                    None
                }
            };
            if let Some((mut at, zero)) = disk {
                let bs = u64::from(self.info.block_size);
                let mut done = 0;
                while done < len {
                    let run = self.info.run(at, false).or_else(|| self.info.run(at, true));
                    let Some((offset, bytes)) = run else {
                        if zero { break; }
                        return Err(Detail::Partition.corrupt());
                    };
                    let size = (len - done).min(bytes);
                    if pos + done + size > from {
                        let Some(slot) = out.get_mut(count) else { return Ok(count) };
                        let extent = Extent::new(offset, size).with_file_offset(pos + done);
                        *slot = if zero { extent.with_unwritten() } else { extent };
                        count += 1;
                    }
                    done += size;
                    if done < len {
                        at.block = at.block.checked_add((size / bs) as u32).ok_or(Detail::Partition.corrupt())?;
                    }
                }
            }
            pos += len;
        }
        Ok(count)
    }

    /// Locates a node's on-disk record: its file entry, one logical block.
    /// Returns how many it filled; fails with [`ErrorKind::LimitExceeded`]
    /// when `out` is empty.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if `out` is empty, or
    /// [`ErrorKind::InvalidHandle`] if the node lies outside the volume.
    /// Entry corruption and device failures are propagated.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(node = ?node)))]
    pub async fn records(&mut self, node: NodeId, out: &mut [Extent]) -> FsResult<usize, D::Error> {
        let block_size = u64::from(self.info.block_size);
        let icb = self.icb(node).await?;
        *out.first_mut().ok_or(ErrorKind::LimitExceeded)? = Extent::new(icb.disk_offset, block_size);
        Ok(1)
    }
}

impl<D: BlockDevice> FileSystem for UdfFs<D> {
    type DeviceError = D::Error;

    /// Read-only, with symlinks, hard links, permissions and owners;
    /// names are UTF-16 and compared exactly.
    fn capabilities(&self) -> Capabilities {
        self.info.capabilities()
    }

    fn root(&self) -> NodeId {
        self.info.root.id()
    }

    /// The partitions' size, and the free space a closed integrity
    /// descriptor records.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all))]
    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        let total = self.info.partitions().iter()
            .filter(|p| matches!(p.kind(), PartitionKind::Physical | PartitionKind::Sparable))
            .map(|p| u64::from(p.len())).sum();
        Ok(FsStats::new(total, self.info.free_blocks.unwrap_or(0), self.info.block_size))
    }

    /// The logical volume identifier.
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let id = self.info.volume.logical_volume.as_str();
        if id.is_empty() {
            return Ok(None);
        }
        let out = buf.get_mut(..id.len()).ok_or(ErrorKind::LimitExceeded)?;
        out.copy_from_slice(id.as_bytes());
        Ok(core::str::from_utf8(out).ok())
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(dir = ?dir)))]
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        let icb = self.dir(dir).await?;
        let mut pos = 0;
        let mut buf = [0u8; 1024];
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, &mut self.dir_walk, &mut self.directory_window, pos).await?;
            pos = fid.next;
            if fid.characteristics.intersects(FileCharacteristics::DELETED | FileCharacteristics::PARENT) {
                continue;
            }
            let len = fid_name(&fid, &mut buf)?;
            if &buf[..len] == name.as_bytes() {
                return fid_node(&self.info, &fid);
            }
        }
        Err(ErrorKind::NotFound.into())
    }

    /// Does nothing: UDF node ids are stable.
    fn forget(&mut self, node: NodeId, count: u64) {
        let _ = (node, count);
    }

    /// The directory containing `dir`, from its parent identifier. The
    /// root is its own parent.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(dir = ?dir)))]
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        let icb = self.dir(dir).await?;
        let mut pos = 0;
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, &mut self.dir_walk, &mut self.directory_window, pos).await?;
            if fid.characteristics.contains(FileCharacteristics::PARENT) {
                return fid_node(&self.info, &fid);
            }
            pos = fid.next;
        }
        Err(Detail::FileIdentifier.corrupt())
    }

    /// Type, size, times (creation only from extended file entries),
    /// permissions, owner and link count. A symlink's size is the length of
    /// its target. Invalid target components fail with [`ErrorKind::Corrupt`],
    /// device failures with [`ErrorKind::Io`], and targets exceeding the
    /// metadata buffer with [`ErrorKind::LimitExceeded`].
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(node = ?node)))]
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        let icb = self.icb(node).await?;
        self.icb_metadata(&icb).await
    }

    /// The cursor is the byte offset of the next identifier in the
    /// directory. Parent and deleted identifiers are skipped. An entry
    /// whose file entry is damaged is still listed, with the type its
    /// identifier records, no permissions and length 0, so the rest of the
    /// directory stays readable; `stat` and `open` on it fail with
    /// [`ErrorKind::Corrupt`]. A damaged identifier ends the listing with
    /// that error, since the next one cannot be found.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(dir = ?dir)))]
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        let icb = self.dir(dir).await?;
        let mut pos = from.into_raw();
        let mut buf = [0u8; 1024];
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, &mut self.dir_walk, &mut self.directory_window, pos).await?;
            pos = fid.next;
            if fid.characteristics.intersects(FileCharacteristics::DELETED | FileCharacteristics::PARENT) {
                continue;
            }
            let len = fid_name(&fid, &mut buf)?;
            let node = fid_node(&self.info, &fid)?;
            let meta = match icb_at(&self.info, &mut self.dev, fid.icb).await {
                Ok(child) => self.icb_metadata(&child).await,
                Err(err) => Err(err),
            };
            let (meta, complete) = match meta {
                Ok(meta) => (meta, true),
                Err(err) if err.kind() == ErrorKind::Io => return Err(err),
                Err(_) => (damaged(&fid), false),
            };
            let entry = DirEntry::new(Name::new(&buf[..len]), node, meta, DirCursor::from_raw(pos))?;
            return Ok(Some(if complete { entry.with_complete_metadata() } else { entry }));
        }
        Ok(None)
    }

    /// The target's path components joined with `/`.
    /// [`ErrorKind::InvalidInput`] for other nodes.
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(node = ?node, bytes = buf.len())))]
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let icb = self.icb(node).await?;
        if icb.fs_type() != Some(FileType::Symlink) {
            return Err(ErrorKind::InvalidInput.into());
        }
        let len = link_target(&self.info, &mut self.dev, &icb, buf).await?;
        Ok(&buf[..len])
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(node = ?node)))]
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
        let icb = self.icb(node).await?;
        match icb.fs_type() {
            Some(FileType::Dir) => Err(ErrorKind::IsADirectory.into()),
            Some(FileType::Symlink) => Err(ErrorKind::Symlink.into()),
            None => Err(Detail::Icb.corrupt()),
            _ if mode == OpenMode::Write => Err(ErrorKind::ReadOnly.into()),
            _ => Ok(()),
        }
    }

    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(node = ?node)))]
    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        let _ = node;
        Ok(())
    }

    /// Allocated but unrecorded extents read as zeros.
    /// Transformed content returns [`ErrorKind::Unsupported`].
    #[cfg_attr(feature = "tracing", tracing::instrument(target = "hadris::udf", level = "trace", skip_all, fields(node = ?node, offset = offset, bytes = buf.len())))]
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let icb = self.icb(node).await?;
        match icb.fs_type() {
            Some(FileType::Dir) => return Err(ErrorKind::IsADirectory.into()),
            Some(FileType::Symlink) => return Err(ErrorKind::Symlink.into()),
            None => return Err(Detail::Icb.corrupt()),
            _ => {}
        }
        read_stream(&self.info, &mut self.dev, &icb, &mut self.file_walk, offset, buf).await
    }
}

}
