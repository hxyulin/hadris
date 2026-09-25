use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats, Metadata,
    MountError, MountOptions, Name, NodeId, OpenMode, Permissions,
};
use hadris_storage::BlockIndex;

use super::FileSystem;
use super::storage::BlockDevice;
use crate::UdfRevision;
use crate::error::{Detail, Error};
use crate::raw::{
    self, AllocationExtentDescriptor, AnchorVolumeDescriptorPointer, EntityId, ExtAd,
    FileCharacteristics, FileIdentifierDescriptor, FileSetDescriptor, LogicalVolumeDescriptor,
    LogicalVolumeIntegrityDescriptor, LongAd, PathComponent, ShortAd, Tag, allocation, extent, tag,
    vsd,
};
use crate::volume::{
    EntityId as Entity, Icb, Identifier, Info, Location, MAX_BLOCK, MAX_PARTITIONS, PartitionInfo,
    VolumeInfo, check_tag,
};

/// Allocation descriptors walked for one file.
const MAX_DESCRIPTORS: u32 = 1 << 20;
/// Allocation extent descriptors followed for one file.
const MAX_CONTINUATIONS: u32 = 1 << 12;
/// Blocks read from one volume descriptor sequence, pointers included.
const MAX_SEQUENCE: u32 = 512;
/// Volume structure descriptors read from the recognition sequence.
const MAX_RECOGNITION: u64 = 64;
/// Integrity descriptors followed.
const MAX_INTEGRITY: u32 = 16;
/// The buffer a directory identifier is read into when it fits.
const FID_BUFFER: usize = 512;

/// One stretch of a file's data.
#[derive(Clone, Copy)]
enum Piece {
    Disk {
        offset: u64,
        len: u64,
    },
    /// Reads as zeros; `offset` is where an allocated but unrecorded
    /// extent lies.
    Zero {
        len: u64,
        offset: Option<u64>,
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
    in_aed: bool,
    pos: usize,
    end: usize,
    steps: u32,
    hops: u32,
    done: bool,
}

/// A File Identifier Descriptor found in a directory.
struct Fid {
    characteristics: FileCharacteristics,
    icb: Location,
    name_at: u64,
    name_len: usize,
    next: u64,
}

/// The id of the entry `fid` names, when its ICB lies inside a partition.
fn fid_node<E>(info: &Info, fid: &Fid) -> Result<NodeId, Error<E>> {
    info.offset::<E>(fid.icb.partition, fid.icb.block, 1)
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

io_transform! {

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
    open: bool,
    recorded: Option<hadris_fs::DateTime>,
}

/// Follows the integrity sequence to its last descriptor. A damaged
/// sequence records nothing.
async fn integrity<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    block_size: u32,
    mut next: raw::ExtentAd,
    partitions: usize,
) -> Integrity {
    let mut found = Integrity::default();
    let mut buf = [0u8; MAX_BLOCK];
    for _ in 0..MAX_INTEGRITY {
        if next.length.get() == 0 {
            break;
        }
        let block = next.location.get();
        let data = &mut buf[..block_size as usize];
        if read_bytes(dev, len, u64::from(block) * u64::from(block_size), data).await.is_err()
            || check_tag(data, Some(tag::INTEGRITY), block).is_err()
        {
            return Integrity::default();
        }
        let lvid: LogicalVolumeIntegrityDescriptor = bytemuck::pod_read_unaligned(&data[..80]);
        let count = (lvid.partition_count.get() as usize).min(partitions);
        let Some(table) = data.get(80..80 + 4 * count) else {
            return Integrity::default();
        };
        found.free = (lvid.integrity_type.get() == 1)
            .then(|| {
                table
                    .chunks_exact(4)
                    .map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
                    .try_fold(0u64, |sum, v| (v != u32::MAX).then_some(sum + u64::from(v)))
            })
            .flatten();
        found.open = lvid.integrity_type.get() == 0;
        found.recorded = crate::time::to_datetime(&lvid.recorded);
        next = lvid.next;
    }
    found
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
        Err(err) => sequence(dev, len, block_size, second).await.map_err(|_| err)?,
    };

    let Some((_, lvd_block)) = found.logical else {
        return Err(Detail::DescriptorSequence.corrupt());
    };
    let lvd: LogicalVolumeDescriptor = bytemuck::pod_read_unaligned(&lvd_block[..440]);
    if lvd.block_size.get() != block_size {
        return Err(Detail::BlockSize.corrupt());
    }
    let table_len = lvd.map_table_length.get() as usize;
    let maps = lvd_block
        .get(440..440 + table_len)
        .filter(|_| 440 + table_len <= block_size as usize)
        .ok_or(Detail::Descriptor.corrupt())?;
    let mut partitions = [PartitionInfo::default(); MAX_PARTITIONS];
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
        match (kind, map_len) {
            (1, 6) => {
                if count == MAX_PARTITIONS {
                    return Err(Detail::PartitionMap.error(ErrorKind::Unsupported));
                }
                let number = u16::from_le_bytes([map[4], map[5]]);
                let (_, _, part) = found.partitions[..found.partition_count]
                    .iter()
                    .find(|(n, _, _)| *n == number)
                    .ok_or(Detail::Partition.corrupt())?;
                partitions[count] = *part;
                count += 1;
            }
            (2, _) => return Err(Detail::PartitionMap.error(ErrorKind::Unsupported)),
            _ => return Err(Detail::Descriptor.corrupt()),
        }
        at += map_len;
    }
    let (_, pvd) = found.primary.ok_or(Detail::DescriptorSequence.corrupt())?;
    let revision = domain_revision(&lvd.domain, nsr03).unwrap_or(if nsr03 {
        UdfRevision::V2_01
    } else {
        UdfRevision::V1_02
    });
    let mut info = Info {
        block_size,
        len,
        root: Location { partition: 0, block: 0 },
        free_blocks: None,
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
            was_dirty: false,
        },
    };

    let fsd_at: LongAd = bytemuck::pod_read_unaligned(&lvd.contents_use);
    let fsd_at = Location::from_long(&fsd_at);
    let mut buf = [0u8; MAX_BLOCK];
    let data = &mut buf[..block_size as usize];
    let offset = info
        .offset::<D::Error>(fsd_at.partition, fsd_at.block, u64::from(block_size))
        .map_err(|_| Detail::FileSet.corrupt())?;
    read_bytes(dev, len, offset, data).await?;
    check_tag(data, Some(tag::FILE_SET), fsd_at.block).map_err(|()| Detail::FileSet.corrupt())?;
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
    let integrity = integrity(dev, len, block_size, lvd.integrity_sequence, count).await;
    info.free_blocks = integrity.free;
    info.volume.was_dirty = integrity.open;
    info.volume.integrity_recorded = integrity.recorded;
    Ok(info)
}

/// Reads and decodes the entry at `at`.
async fn icb_at<D: BlockDevice>(info: &Info, dev: &mut D, at: Location) -> Result<Icb, Error<D::Error>> {
    let bs = info.block_size as usize;
    let offset = info.offset(at.partition, at.block, bs as u64)?;
    let mut block = [0u8; MAX_BLOCK];
    read_bytes(dev, info.len, offset, &mut block[..bs]).await?;
    Icb::parse(at, block, bs)
}

impl Walk {
    fn new(icb: &Icb) -> Self {
        Self {
            aed: [0; MAX_BLOCK],
            in_aed: false,
            pos: icb.ad_start,
            end: icb.ad_start + icb.ad_len,
            steps: 0,
            hops: 0,
            done: false,
        }
    }

    /// The next stretch of data, or `None` after the last descriptor.
    async fn next<D: BlockDevice>(&mut self, info: &Info, dev: &mut D, icb: &Icb) -> Result<Option<Piece>, Error<D::Error>> {
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
            let source = if self.in_aed { &self.aed[..] } else { &icb.block[..] };
            let bytes = &source[self.pos..self.pos + size];
            let (length, kind, at) = match size {
                8 => {
                    let ad: ShortAd = bytemuck::pod_read_unaligned(bytes);
                    let at = Location { partition: icb.at.partition, block: ad.position.get() };
                    (ad.len(), ad.extent_type(), at)
                }
                16 => {
                    let ad: LongAd = bytemuck::pod_read_unaligned(bytes);
                    (ad.len(), ad.extent_type(), Location::from_long(&ad))
                }
                _ => {
                    let ad: ExtAd = bytemuck::pod_read_unaligned(bytes);
                    let at = Location {
                        partition: ad.location.partition.get(),
                        block: ad.location.block.get(),
                    };
                    (ad.len(), ad.extent_type(), at)
                }
            };
            self.pos += size;
            if length == 0 {
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
                    let bs = info.block_size as usize;
                    let offset = info.offset(at.partition, at.block, bs as u64)?;
                    read_bytes(dev, info.len, offset, &mut self.aed[..bs]).await?;
                    check_tag(&self.aed[..bs], Some(tag::ALLOCATION_EXTENT), at.block).map_err(|()| bad())?;
                    let aed: AllocationExtentDescriptor = bytemuck::pod_read_unaligned(&self.aed[..24]);
                    let ads = aed.allocation_descriptors_length.get() as usize;
                    let end = 24usize.checked_add(ads).ok_or_else(bad)?;
                    if end > bs || end as u64 > length.max(24) {
                        return Err(bad());
                    }
                    self.in_aed = true;
                    self.pos = 24;
                    self.end = end;
                }
                extent::RECORDED => {
                    let offset = info.offset(at.partition, at.block, length)?;
                    return Ok(Some(Piece::Disk { offset, len: length }));
                }
                extent::ALLOCATED => {
                    let offset = info.offset::<D::Error>(at.partition, at.block, length).ok();
                    return Ok(Some(Piece::Zero { len: length, offset }));
                }
                _ => return Ok(Some(Piece::Zero { len: length, offset: None })),
            }
        }
    }
}

/// Reads up to `buf.len()` bytes of the data of `icb` from `offset`;
/// fewer only at the end of the data.
async fn read_stream<D: BlockDevice>(
    info: &Info,
    dev: &mut D,
    icb: &Icb,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, Error<D::Error>> {
    if offset >= icb.size {
        return Ok(0);
    }
    let want = usize::try_from(icb.size - offset).unwrap_or(usize::MAX).min(buf.len());
    let mut walk = Walk::new(icb);
    let mut pos = 0u64;
    let mut done = 0usize;
    while done < want {
        let Some(piece) = walk.next(info, dev, icb).await? else {
            return Err(Detail::AllocationDescriptor.corrupt());
        };
        let len = piece.len();
        let at = offset + done as u64;
        if at < pos + len {
            let within = at - pos;
            let take = usize::try_from(len - within).unwrap_or(usize::MAX).min(want - done);
            let out = &mut buf[done..done + take];
            match piece {
                Piece::Disk { offset, .. } => read_bytes(dev, info.len, offset + within, out).await?,
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
    offset: u64,
    buf: &mut [u8],
    bad: Detail,
) -> Result<(), Error<D::Error>> {
    if read_stream(info, dev, icb, offset, buf).await? != buf.len() {
        return Err(bad.corrupt());
    }
    Ok(())
}

/// The file identifier at `pos` in directory `dir`.
async fn fid_at<D: BlockDevice>(info: &Info, dev: &mut D, dir: &Icb, pos: u64) -> Result<Fid, Error<D::Error>> {
    let bad = || Detail::FileIdentifier.corrupt();
    let mut buf = [0u8; FID_BUFFER];
    read_exact(info, dev, dir, pos, &mut buf[..38], Detail::FileIdentifier).await?;
    let fid: FileIdentifierDescriptor = bytemuck::pod_read_unaligned(&buf[..38]);
    let tag = fid.tag;
    if !tag.is_checksum_valid()
        || tag.identifier.get() != tag::FILE_IDENTIFIER
        || !matches!(tag.version.get(), 2 | 3)
    {
        return Err(bad());
    }
    let total = fid.total_len() as u64;
    let covered = u64::from(tag.crc_length.get());
    if covered + 16 > total {
        return Err(bad());
    }
    let mut crc = 0u16;
    let mut at = 16u64;
    while at < 16 + covered {
        let take = (16 + covered - at).min(FID_BUFFER as u64) as usize;
        read_exact(info, dev, dir, pos + at, &mut buf[..take], Detail::FileIdentifier).await?;
        crc = raw::crc16_update(crc, &buf[..take]);
        at += take as u64;
    }
    if crc != tag.crc.get() {
        return Err(bad());
    }
    Ok(Fid {
        characteristics: FileCharacteristics::from_bits_retain(fid.characteristics),
        icb: Location::from_long(&fid.icb),
        name_at: pos + 38 + u64::from(fid.implementation_use_length.get()),
        name_len: usize::from(fid.identifier_length),
        next: pos + total,
    })
}

/// The name of `fid`, decoded into `out`.
async fn fid_name<D: BlockDevice>(
    info: &Info,
    dev: &mut D,
    dir: &Icb,
    fid: &Fid,
    out: &mut [u8],
) -> Result<usize, Error<D::Error>> {
    if fid.name_len == 0 {
        return Err(Detail::FileIdentifier.corrupt());
    }
    let mut raw = [0u8; 255];
    let raw = &mut raw[..fid.name_len];
    read_exact(info, dev, dir, fid.name_at, raw, Detail::FileIdentifier).await?;
    crate::name::decode_name(raw, out).ok_or(Detail::FileIdentifier.corrupt())
}

/// Writes the target of the symlink `icb` into `out`.
async fn link_target<D: BlockDevice>(info: &Info, dev: &mut D, icb: &Icb, out: &mut [u8]) -> FsResult<usize, D::Error> {
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
        read_exact(info, dev, icb, pos, &mut head, Detail::PathComponent).await?;
        let component: PathComponent = bytemuck::pod_read_unaligned(&head);
        pos += 4;
        let mut ident = [0u8; 255];
        let ident = &mut ident[..usize::from(component.identifier_length)];
        read_exact(info, dev, icb, pos, ident, Detail::PathComponent).await?;
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
/// allocator. It implements the `hadris_fs` `FileSystem` trait read-only:
/// node ids are ICB locations, so they are stable and `forget` does nothing; a directory's id is the id its
/// name in the parent lists, and hard links share one id. An id outside
/// every partition fails with [`ErrorKind::InvalidHandle`]; one inside a
/// partition is read as an entry, and a damaged entry fails with
/// [`ErrorKind::Corrupt`]. Write methods fail with [`ErrorKind::ReadOnly`].
///
/// ```rust,ignore
/// let mut udf = UdfFs::mount(dev, MountOptions::new())?;
/// let file = udf.resolve(b"/docs/readme.txt", Resolve::Lexical)?;
/// let mut buf = [0u8; 64];
/// let n = udf.read(file, 0, &mut buf)?;
/// ```
#[derive(Debug)]
pub struct UdfFs<D> {
    dev: D,
    info: Info,
}

impl<D: BlockDevice> UdfFs<D> {
    /// Mounts the volume on `dev`, read-only in every version: finds an
    /// anchor, checks the recognition sequence, and reads the prevailing
    /// volume descriptors, from the reserve sequence when the main one is
    /// damaged, and the file set. With [`MountOptions::backup_boot`] it
    /// skips the anchor at block 256 for those at the end of the volume
    /// and reads the reserve sequence first. Other options do not apply.
    ///
    /// Fails with [`ErrorKind::NotRecognized`] without a UDF recognition
    /// sequence, with [`ErrorKind::Corrupt`] when the structures are
    /// invalid, and with [`ErrorKind::Unsupported`] for partition maps
    /// other than type 1 or device blocks above 4096 bytes. The device
    /// comes back in the [`MountError`].
    pub async fn mount(mut dev: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        match mount(&mut dev, options.is_backup_boot()).await {
            Ok(info) => Ok(Self { dev, info }),
            Err(err) => Err(MountError::new(err, dev)),
        }
    }

    /// Gives the device back. The volume is read-only, so there is nothing
    /// to sync and this never fails.
    pub async fn unmount(self) -> Result<D, MountError<D, D::Error>> {
        Ok(self.dev)
    }

    /// What the volume descriptors record: the revision, block size,
    /// partitions, identifiers, domain, implementation and times.
    pub fn info(&self) -> &VolumeInfo {
        &self.info.volume
    }

    /// Whether the logical volume integrity descriptor was open at mount:
    /// the volume was not cleanly closed. A volume whose integrity
    /// sequence cannot be read counts as clean.
    pub fn was_dirty(&self) -> bool {
        self.info.volume.was_dirty
    }

    /// Reads `buf.len()` bytes from byte `offset` of the device.
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
            .filter(|at| self.info.offset::<D::Error>(at.partition, at.block, 1).is_ok())
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
            if let Ok(len) = link_target(&self.info, &mut self.dev, icb, &mut target).await {
                meta = meta.with_len(len as u64);
            }
        }
        Ok(meta)
    }

    /// Maps a node's data to the device: fills `out` with its extents
    /// that end after byte `from` of the data, in order, and returns how
    /// many it filled. Call again from the end of the last one for more; 0
    /// means there are none. Allocated but unrecorded extents are marked
    /// unwritten, holes are left out, and data embedded in the file entry
    /// is one extent inside it.
    pub async fn extents(&mut self, node: NodeId, from: u64, out: &mut [hadris_fs::Extent]) -> FsResult<usize, D::Error> {
        let icb = self.icb(node).await?;
        let mut walk = Walk::new(&icb);
        let mut pos = 0u64;
        let mut count = 0;
        while pos < icb.size {
            let Some(piece) = walk.next(&self.info, &mut self.dev, &icb).await? else {
                return Err(Detail::AllocationDescriptor.corrupt());
            };
            let len = piece.len().min(icb.size - pos);
            let extent = match piece {
                Piece::Disk { offset, .. } => Some(hadris_fs::Extent::new(offset, len)),
                Piece::Zero { offset: Some(offset), .. } => Some(hadris_fs::Extent::new(offset, len).with_unwritten()),
                Piece::Zero { offset: None, .. } => None,
                Piece::Embedded { start, .. } => {
                    let block = self.info.offset::<D::Error>(icb.at.partition, icb.at.block, 1)?;
                    Some(hadris_fs::Extent::new(block + start as u64, len))
                }
            };
            if let Some(extent) = extent.filter(|_| pos + len > from && len > 0) {
                let Some(slot) = out.get_mut(count) else {
                    return Ok(count);
                };
                *slot = extent.with_file_offset(pos);
                count += 1;
            }
            pos += len;
        }
        Ok(count)
    }

    /// Locates a node's on-disk record: its file entry, one logical block.
    /// Returns how many it filled; fails with [`ErrorKind::LimitExceeded`]
    /// when `out` is empty.
    pub async fn records(&mut self, node: NodeId, out: &mut [hadris_fs::Extent]) -> FsResult<usize, D::Error> {
        let at = Location::of(node).ok_or(ErrorKind::InvalidHandle)?;
        let block_size = u64::from(self.info.block_size);
        let offset = self
            .info
            .offset::<D::Error>(at.partition, at.block, block_size)
            .map_err(|_| ErrorKind::InvalidHandle)?;
        self.icb(node).await?;
        *out.first_mut().ok_or(ErrorKind::LimitExceeded)? = hadris_fs::Extent::new(offset, block_size);
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
    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        let total = self.info.partitions().iter().map(|p| u64::from(p.len())).sum();
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

    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        let icb = self.dir(dir).await?;
        let mut pos = 0;
        let mut buf = [0u8; 1024];
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, pos).await?;
            pos = fid.next;
            if fid.characteristics.intersects(FileCharacteristics::DELETED | FileCharacteristics::PARENT) {
                continue;
            }
            let len = fid_name(&self.info, &mut self.dev, &icb, &fid, &mut buf).await?;
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
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        let icb = self.dir(dir).await?;
        let mut pos = 0;
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, pos).await?;
            if fid.characteristics.contains(FileCharacteristics::PARENT) {
                return fid_node(&self.info, &fid);
            }
            pos = fid.next;
        }
        Err(Detail::FileIdentifier.corrupt())
    }

    /// Type, size, times (creation only from extended file entries),
    /// permissions, owner and link count. A symlink's size is the length of
    /// its target.
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
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        let icb = self.dir(dir).await?;
        let mut pos = from.into_raw();
        let mut buf = [0u8; 1024];
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, pos).await?;
            pos = fid.next;
            if fid.characteristics.intersects(FileCharacteristics::DELETED | FileCharacteristics::PARENT) {
                continue;
            }
            let len = fid_name(&self.info, &mut self.dev, &icb, &fid, &mut buf).await?;
            let node = fid_node(&self.info, &fid)?;
            let meta = match icb_at(&self.info, &mut self.dev, fid.icb).await {
                Ok(child) => self.icb_metadata(&child).await,
                Err(err) => Err(err),
            };
            let meta = match meta {
                Ok(meta) => meta,
                Err(err) if err.kind() == ErrorKind::Io => return Err(err),
                Err(_) => damaged(&fid),
            };
            let entry = DirEntry::new(Name::new(&buf[..len]), node, meta, DirCursor::from_raw(pos))?;
            return Ok(Some(entry));
        }
        Ok(None)
    }

    /// The target's path components joined with `/`.
    /// [`ErrorKind::InvalidInput`] for other nodes.
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let icb = self.icb(node).await?;
        if icb.fs_type() != Some(FileType::Symlink) {
            return Err(ErrorKind::InvalidInput.into());
        }
        let len = link_target(&self.info, &mut self.dev, &icb, buf).await?;
        Ok(&buf[..len])
    }

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

    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        let _ = node;
        Ok(())
    }

    /// Allocated but unrecorded extents read as zeros.
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let icb = self.icb(node).await?;
        match icb.fs_type() {
            Some(FileType::Dir) => return Err(ErrorKind::IsADirectory.into()),
            Some(FileType::Symlink) => return Err(ErrorKind::Symlink.into()),
            None => return Err(Detail::Icb.corrupt()),
            _ => {}
        }
        read_stream(&self.info, &mut self.dev, &icb, offset, buf).await
    }
}

}
