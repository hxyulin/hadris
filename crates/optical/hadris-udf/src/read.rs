use hadris_fs::{
    Capabilities, DirCursor, DirEntry, ErrorKind, FileType, FsResult, FsStats, Metadata,
    MountError, Name, NameBuf, NodeId,
};
use hadris_storage::BlockIndex;

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
    Icb, Identifier, Info, Location, MAX_BLOCK, MAX_PARTITIONS, Partition, check_tag,
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
    Disk { offset: u64, len: u64 },
    Zero { len: u64 },
    Embedded { start: usize, len: usize },
}

impl Piece {
    fn len(&self) -> u64 {
        match *self {
            Piece::Disk { len, .. } | Piece::Zero { len } => len,
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

/// A node id the caller gave that names nothing valid is an invalid handle,
/// unless the device failed or the volume is cut short.
fn handle<E>(err: Error<E>) -> hadris_fs::Error<E> {
    match (err.kind(), err.detail()) {
        (ErrorKind::Io, _) | (_, Some(Detail::OutsideImage)) => err.into(),
        _ => ErrorKind::InvalidHandle.into(),
    }
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
    volume_id: Option<(u32, [u8; 32])>,
    logical: Option<(u32, [u8; MAX_BLOCK])>,
    partitions: [(u16, u32, Partition); MAX_PARTITIONS],
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
        .ok_or(Error::corrupt(Detail::OutsideImage))?;
    if end > len {
        return Err(Error::corrupt(Detail::OutsideImage));
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
                .await
                .map_err(Error::device)?;
            done += whole;
            pos += whole as u64;
        } else {
            let mut scratch = [0u8; MAX_BLOCK];
            let scratch = &mut scratch[..bs as usize];
            dev.read_blocks(block, scratch).await.map_err(Error::device)?;
            let take = (bs as usize - within).min(left);
            buf[done..done + take].copy_from_slice(&scratch[within..within + take]);
            done += take;
            pos += take as u64;
        }
    }
    Ok(())
}

/// Finds an anchor at block 256, N-256 or N-1, trying logical block sizes
/// from the device's up to 4096 bytes.
async fn find_anchor<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    device_block: u32,
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
    Err(Error::corrupt(Detail::Anchor))
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
    nsr.ok_or(Error::corrupt(Detail::RecognitionSequence))
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
        volume_id: None,
        logical: None,
        partitions: [(0, 0, Partition::default()); MAX_PARTITIONS],
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
        let tag = check_tag(data, None, block).map_err(|()| Error::corrupt(Detail::Descriptor))?;
        let number = u32::from_le_bytes([data[16], data[17], data[18], data[19]]);
        match tag.identifier.get() {
            tag::PRIMARY_VOLUME => {
                if found.volume_id.is_none_or(|(seen, _)| number >= seen) {
                    let mut id = [0u8; 32];
                    id.copy_from_slice(&data[24..56]);
                    found.volume_id = Some((number, id));
                }
            }
            tag::LOGICAL_VOLUME => {
                if found.logical.as_ref().is_none_or(|(seen, _)| number >= *seen) {
                    found.logical = Some((number, buf));
                }
            }
            tag::PARTITION => {
                let pd: raw::PartitionDescriptor = bytemuck::pod_read_unaligned(&data[..512]);
                let part = Partition::new(pd.number.get(), pd.start.get(), pd.length.get());
                let count = found.partition_count;
                match found.partitions[..count].iter_mut().find(|(n, _, _)| *n == part.number()) {
                    Some(slot) if number >= slot.1 => *slot = (part.number(), number, part),
                    Some(_) => {}
                    None if count < MAX_PARTITIONS => {
                        found.partitions[count] = (part.number(), number, part);
                        found.partition_count += 1;
                    }
                    None => return Err(Error::new(ErrorKind::Unsupported, Detail::PartitionMap)),
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
        block = block.checked_add(1).ok_or(Error::corrupt(Detail::DescriptorSequence))?;
    }
    if found.volume_id.is_none() || found.logical.is_none() || found.partition_count == 0 {
        return Err(Error::corrupt(Detail::DescriptorSequence));
    }
    Ok(found)
}

/// The free blocks a closed integrity descriptor records, following the
/// integrity sequence to its last descriptor.
async fn free_blocks<D: BlockDevice>(
    dev: &mut D,
    len: u64,
    block_size: u32,
    mut next: raw::ExtentAd,
    partitions: usize,
) -> Option<u64> {
    let mut free = None;
    let mut buf = [0u8; MAX_BLOCK];
    for _ in 0..MAX_INTEGRITY {
        if next.length.get() == 0 {
            break;
        }
        let block = next.location.get();
        let data = &mut buf[..block_size as usize];
        read_bytes(dev, len, u64::from(block) * u64::from(block_size), data).await.ok()?;
        check_tag(data, Some(tag::INTEGRITY), block).ok()?;
        let lvid: LogicalVolumeIntegrityDescriptor = bytemuck::pod_read_unaligned(&data[..80]);
        let count = (lvid.partition_count.get() as usize).min(partitions);
        let table = data.get(80..80 + 4 * count)?;
        free = (lvid.integrity_type.get() == 1)
            .then(|| {
                table
                    .chunks_exact(4)
                    .map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
                    .try_fold(0u64, |sum, v| (v != u32::MAX).then_some(sum + u64::from(v)))
            })
            .flatten();
        next = lvid.next;
    }
    free
}

/// Reads the volume structures of `dev`.
async fn mount<D: BlockDevice>(dev: &mut D) -> Result<Info, Error<D::Error>> {
    let device_block = dev.block_size().get();
    if device_block as usize > MAX_BLOCK {
        return Err(Error::new(ErrorKind::Unsupported, Detail::BlockSize));
    }
    let len = dev.block_count().saturating_mul(u64::from(device_block));
    let (block_size, anchor) = find_anchor(dev, len, device_block).await?;
    let nsr03 = recognition(dev, len, block_size).await?;
    let found = match sequence(dev, len, block_size, anchor.main).await {
        Ok(found) => found,
        Err(err) if err.kind() == ErrorKind::Io => return Err(err),
        Err(err) => sequence(dev, len, block_size, anchor.reserve).await.map_err(|_| err)?,
    };

    let Some((_, lvd_block)) = found.logical else {
        return Err(Error::corrupt(Detail::DescriptorSequence));
    };
    let lvd: LogicalVolumeDescriptor = bytemuck::pod_read_unaligned(&lvd_block[..440]);
    if lvd.block_size.get() != block_size {
        return Err(Error::corrupt(Detail::BlockSize));
    }
    let table_len = lvd.map_table_length.get() as usize;
    let maps = lvd_block
        .get(440..440 + table_len)
        .filter(|_| 440 + table_len <= block_size as usize)
        .ok_or(Error::corrupt(Detail::Descriptor))?;
    let mut partitions = [Partition::default(); MAX_PARTITIONS];
    let mut count = 0;
    let mut at = 0;
    for _ in 0..lvd.map_count.get() {
        let (kind, map_len) = match maps.get(at..at + 2) {
            Some(head) => (head[0], usize::from(head[1])),
            None => return Err(Error::corrupt(Detail::Descriptor)),
        };
        let map = maps
            .get(at..at + map_len)
            .filter(|_| map_len >= 2)
            .ok_or(Error::corrupt(Detail::Descriptor))?;
        match (kind, map_len) {
            (1, 6) => {
                if count == MAX_PARTITIONS {
                    return Err(Error::new(ErrorKind::Unsupported, Detail::PartitionMap));
                }
                let number = u16::from_le_bytes([map[4], map[5]]);
                let (_, _, part) = found.partitions[..found.partition_count]
                    .iter()
                    .find(|(n, _, _)| *n == number)
                    .ok_or(Error::corrupt(Detail::Partition))?;
                partitions[count] = *part;
                count += 1;
            }
            (2, _) => return Err(Error::new(ErrorKind::Unsupported, Detail::PartitionMap)),
            _ => return Err(Error::corrupt(Detail::Descriptor)),
        }
        at += map_len;
    }
    let (_, volume_id) = found.volume_id.ok_or(Error::corrupt(Detail::DescriptorSequence))?;
    let revision = domain_revision(&lvd.domain, nsr03).unwrap_or(if nsr03 {
        UdfRevision::V2_01
    } else {
        UdfRevision::V1_02
    });
    let mut info = Info {
        block_size,
        len,
        partitions,
        partition_count: count,
        root: Location { partition: 0, block: 0 },
        revision,
        volume_id: Identifier::decode(&volume_id),
        logical_volume_id: Identifier::decode(&lvd.logical_volume_identifier),
        free_blocks: None,
    };

    let fsd_at: LongAd = bytemuck::pod_read_unaligned(&lvd.contents_use);
    let fsd_at = Location::from_long(&fsd_at);
    let mut buf = [0u8; MAX_BLOCK];
    let data = &mut buf[..block_size as usize];
    let offset = info
        .offset::<D::Error>(fsd_at.partition, fsd_at.block, u64::from(block_size))
        .map_err(|_| Error::corrupt(Detail::FileSet))?;
    read_bytes(dev, len, offset, data).await?;
    check_tag(data, Some(tag::FILE_SET), fsd_at.block).map_err(|()| Error::corrupt(Detail::FileSet))?;
    let fsd: FileSetDescriptor = bytemuck::pod_read_unaligned(&data[..512]);
    info.root = Location::from_long(&fsd.root);
    let root = icb_at(&info, dev, info.root).await.map_err(|err| match err.kind() {
        ErrorKind::Io => err,
        _ => Error::corrupt(Detail::FileSet),
    })?;
    if !root.is_dir() {
        return Err(Error::corrupt(Detail::FileSet));
    }
    info.free_blocks = free_blocks(dev, len, block_size, lvd.integrity_sequence, count).await;
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
        let bad = || Error::corrupt(Detail::AllocationDescriptor);
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
                _ => return Ok(Some(Piece::Zero { len: length })),
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
            return Err(Error::corrupt(Detail::AllocationDescriptor));
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
        return Err(Error::corrupt(bad));
    }
    Ok(())
}

/// The file identifier at `pos` in directory `dir`.
async fn fid_at<D: BlockDevice>(info: &Info, dev: &mut D, dir: &Icb, pos: u64) -> Result<Fid, Error<D::Error>> {
    let bad = || Error::corrupt(Detail::FileIdentifier);
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
        return Err(Error::corrupt(Detail::FileIdentifier));
    }
    let mut raw = [0u8; 255];
    let raw = &mut raw[..fid.name_len];
    read_exact(info, dev, dir, fid.name_at, raw, Detail::FileIdentifier).await?;
    crate::name::decode_name(raw, out).ok_or(Error::corrupt(Detail::FileIdentifier))
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
                    .ok_or(Error::corrupt(Detail::PathComponent))?;
                push(out, &mut len, &name[..n])?;
            }
            _ => return Err(Error::corrupt(Detail::PathComponent).into()),
        }
    }
    Ok(len)
}

/// An open UDF volume on a block device.
///
/// It reads the volume structures once, when opened, and needs no
/// allocator. It implements `hadris_fs::FsDriver` read-only through its
/// inherent methods: node ids are ICB locations, so they are stable and
/// [`forget`](Self::forget) does nothing; a directory's id is the id its
/// name in the parent lists, and hard links share one id. Write methods
/// fail with [`ErrorKind::ReadOnly`].
///
/// ```rust,ignore
/// let mut udf = UdfFs::open(dev)?;
/// let data = hadris_fs::sync::DriverExt::read_to_vec(&mut udf, "/docs/readme.txt")?;
/// ```
#[derive(Debug)]
pub struct UdfFs<D> {
    dev: D,
    info: Info,
}

impl<D: BlockDevice> UdfFs<D> {
    /// Opens the volume on `dev`: finds an anchor, checks the recognition
    /// sequence, and reads the prevailing volume descriptors, from the
    /// reserve sequence when the main one is damaged, and the file set.
    ///
    /// Fails with [`ErrorKind::Corrupt`] when the structures are invalid,
    /// and with [`ErrorKind::Unsupported`] for partition maps other than
    /// type 1 or device blocks above 4096 bytes. The device comes back in
    /// the [`MountError`].
    pub async fn open(mut dev: D) -> Result<Self, MountError<D, D::Error>> {
        match mount(&mut dev).await {
            Ok(info) => Ok(Self { dev, info }),
            Err(err) => Err(MountError::new(err.into(), dev)),
        }
    }

    /// The volume identifier of the primary volume descriptor.
    pub fn volume_id(&self) -> &str {
        self.info.volume_id.as_str()
    }

    /// The logical volume identifier, which operating systems show as the
    /// label.
    pub fn logical_volume_id(&self) -> &str {
        self.info.logical_volume_id.as_str()
    }

    /// The UDF revision the domain identifier records, or 1.02 or 2.01 by
    /// the recognition sequence when it records none.
    pub fn revision(&self) -> UdfRevision {
        self.info.revision
    }

    /// The logical block size.
    pub fn block_size(&self) -> u32 {
        self.info.block_size
    }

    /// The partitions, indexed by partition reference number.
    pub fn partitions(&self) -> &[Partition] {
        self.info.partitions()
    }

    /// Reads `buf.len()` bytes from byte `offset` of the device.
    pub async fn read_bytes(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Error<D::Error>> {
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
        let at = Location::of(node).ok_or(ErrorKind::InvalidHandle)?;
        icb_at(&self.info, &mut self.dev, at).await.map_err(handle)
    }

    async fn dir(&mut self, node: NodeId) -> FsResult<Icb, D::Error> {
        let icb = self.icb(node).await?;
        if !icb.is_dir() {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok(icb)
    }

    /// Read-only, with symlinks, hard links, permissions and owners;
    /// names are UTF-16 and compared exactly.
    pub fn capabilities(&self) -> Capabilities {
        self.info.capabilities()
    }

    /// The root directory.
    pub fn root(&self) -> NodeId {
        self.info.root.id()
    }

    /// Finds `name` in `dir`.
    pub async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
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
                return Ok(fid.icb.id());
            }
        }
        Err(ErrorKind::NotFound.into())
    }

    /// Writes the entry after `cursor` into `name` and advances `cursor`,
    /// the byte offset of the next identifier in the directory. Parent and
    /// deleted identifiers are skipped.
    pub async fn read_dir_entry(
        &mut self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, D::Error> {
        let icb = self.dir(dir).await?;
        let mut pos = cursor.into_raw();
        let mut buf = [0u8; 1024];
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, pos).await?;
            pos = fid.next;
            if fid.characteristics.intersects(FileCharacteristics::DELETED | FileCharacteristics::PARENT) {
                continue;
            }
            let len = fid_name(&self.info, &mut self.dev, &icb, &fid, &mut buf).await?;
            let file_type = if fid.characteristics.contains(FileCharacteristics::DIRECTORY) {
                FileType::Dir
            } else {
                let child = icb_at(&self.info, &mut self.dev, fid.icb).await?;
                child.fs_type().ok_or(Error::corrupt(Detail::Icb))?
            };
            name.set_bytes(&buf[..len])?;
            *cursor = DirCursor::from_raw(pos);
            return Ok(Some(DirEntry::new(fid.icb.id(), file_type, len)));
        }
        *cursor = DirCursor::from_raw(pos.max(icb.size));
        Ok(None)
    }

    /// Metadata of a node: type, size, times (creation only from extended
    /// file entries), permissions, owner and link count. A symlink's size
    /// is the length of its target.
    pub async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        let icb = self.icb(node).await?;
        let file_type = icb.fs_type().ok_or(ErrorKind::Corrupt)?;
        let mut meta = icb.metadata(file_type);
        if file_type == FileType::Symlink {
            let mut target = [0u8; 4096];
            if let Ok(len) = link_target(&self.info, &mut self.dev, &icb, &mut target).await {
                meta = meta.with_len(len as u64);
            }
        }
        Ok(meta)
    }

    /// Reads from a file at `offset`. Allocated but unrecorded extents read
    /// as zeros. Directories fail with [`ErrorKind::IsADirectory`] and
    /// symlinks with [`ErrorKind::Symlink`].
    pub async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let icb = self.icb(node).await?;
        match icb.fs_type() {
            Some(FileType::Dir) => return Err(ErrorKind::IsADirectory.into()),
            Some(FileType::Symlink) => return Err(ErrorKind::Symlink.into()),
            None => return Err(ErrorKind::Corrupt.into()),
            _ => {}
        }
        Ok(read_stream(&self.info, &mut self.dev, &icb, offset, buf).await?)
    }

    /// The partitions' size, and the free space a closed integrity
    /// descriptor records.
    pub async fn stats(&mut self) -> FsResult<FsStats, D::Error> {
        let total = self.info.partitions().iter().map(|p| u64::from(p.len())).sum();
        Ok(FsStats::new(total, self.info.free_blocks.unwrap_or(0), self.info.block_size))
    }

    /// Does nothing: UDF node ids are stable.
    pub fn forget(&mut self, node: NodeId) {
        let _ = node;
    }

    /// The directory containing `dir`, from its parent identifier. The
    /// root is its own parent.
    pub async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        let icb = self.dir(dir).await?;
        let mut pos = 0;
        while pos < icb.size {
            let fid = fid_at(&self.info, &mut self.dev, &icb, pos).await?;
            if fid.characteristics.contains(FileCharacteristics::PARENT) {
                return Ok(fid.icb.id());
            }
            pos = fid.next;
        }
        Err(Error::corrupt(Detail::FileIdentifier).into())
    }

    /// Writes a symlink's target into `buf`, its path components joined
    /// with `/`. [`ErrorKind::InvalidInput`] for other nodes,
    /// [`ErrorKind::LimitExceeded`] when `buf` is too small.
    pub async fn read_link(&mut self, link: NodeId, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let icb = self.icb(link).await?;
        if icb.fs_type() != Some(FileType::Symlink) {
            return Err(ErrorKind::InvalidInput.into());
        }
        link_target(&self.info, &mut self.dev, &icb, buf).await
    }

    /// Calls `visit` with the byte range of each recorded extent of a
    /// node's data, in order. Embedded data and unrecorded extents are not
    /// visited.
    pub async fn extents(&mut self, node: NodeId, mut visit: impl FnMut(hadris_fs::Extent)) -> FsResult<(), D::Error> {
        let icb = self.icb(node).await?;
        let mut walk = Walk::new(&icb);
        let mut pos = 0u64;
        while pos < icb.size {
            let Some(piece) = walk.next(&self.info, &mut self.dev, &icb).await? else {
                return Err(Error::corrupt(Detail::AllocationDescriptor).into());
            };
            let len = piece.len().min(icb.size - pos);
            if let Piece::Disk { offset, .. } = piece {
                visit(hadris_fs::Extent::new(offset, len));
            }
            pos += len;
        }
        Ok(())
    }
}

}

impl_udf_driver!(impl[D: BlockDevice] UdfFs<D>, error = D::Error, read_only; also = [parent, read_link]);
