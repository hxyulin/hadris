use hadris_fs::{Error, ErrorKind, FsResult};

use super::block::{load, read_bytes, store, write_bytes};
use super::storage::BlockDevice;
use crate::exfat::io::{
    BootRegion, ClusterState, DirWalk, Dirty, ExFat, Extent, UPCASE_CACHE, Upcase,
};
use crate::exfat::{self as raw, Detail, ENTRY_SIZE, Geometry, RawEntry, UpcaseDecoder};
use crate::io::{BlockBuf, ChainPos, ClusterGroup, Held};

const BOOT_SECTOR_LEN: usize = 512;
/// Offset of `VolumeFlags` in the boot sector.
const VOLUME_FLAGS_AT: u64 = 106;
/// Offset of `PercentInUse` in the boot sector.
const PERCENT_AT: u64 = 112;
/// The largest up-case table: 65536 mappings, uncompressed.
const MAX_UPCASE_LEN: u64 = 0x2_0000;

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}

/// Whether the FAT entries of `a` and `b` lie in one device block of every
/// FAT written.
fn same_fat_block(geo: &Geometry, size: u64, a: u32, b: u32) -> bool {
    [Some(geo.fat_start()), geo.mirror_fat()]
        .into_iter()
        .flatten()
        .all(|base| (base + a as u64 * 4) / size == (base + b as u64 * 4) / size)
}

fn check_cluster(geo: &Geometry, cluster: u32) -> Result<u32, ErrorKind> {
    if geo.is_cluster(cluster) {
        Ok(cluster)
    } else {
        Err(ErrorKind::Corrupt)
    }
}

fn cluster_at(geo: &Geometry, cluster: u32) -> Result<u64, ErrorKind> {
    geo.cluster_offset(cluster).ok_or(ErrorKind::Corrupt)
}

/// `err`, with `detail` when the volume is corrupt.
fn blame<E>(err: Error<E>, detail: Detail) -> Error<E> {
    if err.kind() == ErrorKind::Corrupt {
        err.with_detail(detail.code())
    } else {
        err
    }
}

io_transform! {

/// The geometry of the boot region at `base`, if its boot sector is valid
/// and its checksum matches.
async fn boot_region<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, base: u64) -> FsResult<Option<Geometry>, D::Error> {
    let parsed = {
        let mut sector = [0u8; BOOT_SECTOR_LEN];
        read_bytes(dev, block, base, &mut sector).await?;
        raw::parse_boot(&bytemuck::pod_read_unaligned(&sector))
    };
    let Ok(geo) = parsed else {
        return Ok(None);
    };
    let size = geo.sector_size();
    let mut chunk = [0u8; 128];
    let mut sum = 0u32;
    let mut at = 0;
    while at < (raw::BOOT_REGION_SECTORS - 1) * size {
        read_bytes(dev, block, base + at, &mut chunk).await?;
        sum = raw::boot_checksum(sum, at.min(1), &chunk);
        at += chunk.len() as u64;
    }
    while at < raw::BOOT_REGION_SECTORS * size {
        read_bytes(dev, block, base + at, &mut chunk).await?;
        if chunk.chunks_exact(4).any(|word| le32(word, 0) != sum) {
            return Ok(None);
        }
        at += chunk.len() as u64;
    }
    Ok(Some(geo))
}

/// Reads the boot region, or the backup boot region when the main one is
/// damaged, and checks that the volume fits the device.
///
/// Fails with [`ErrorKind::InvalidInput`] when `block` is not sized to the
/// device's blocks, with [`ErrorKind::NotRecognized`] when the first sector
/// does not name exFAT, and with [`ErrorKind::Corrupt`] when neither boot
/// region holds a valid boot sector and checksum or the boot sector
/// describes a volume larger than the device.
pub async fn read_boot<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf) -> FsResult<(Geometry, BootRegion), D::Error> {
    boot_from(dev, block, true).await
}

/// Reads the backup boot region, ignoring the main one, and checks that
/// the volume fits the device, for mounting a volume whose main boot region
/// is damaged or suspect.
///
/// Fails as [`read_boot`] does, with [`ErrorKind::Corrupt`] when the backup
/// region holds no valid boot sector and checksum.
pub async fn read_backup_boot<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf) -> FsResult<Geometry, D::Error> {
    Ok(boot_from(dev, block, false).await?.0)
}

/// [`read_boot`], trying the main region first when `main` is set.
async fn boot_from<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, main: bool) -> FsResult<(Geometry, BootRegion), D::Error> {
    let size = block.block_size();
    if size as u64 != dev.block_size().get() as u64 {
        return Err(ErrorKind::InvalidInput.into());
    }
    let mut region = BootRegion::Main;
    let mut geo = match main {
        true => boot_region(dev, block, 0).await?,
        false => None,
    };
    for shift in 9..=12u8 {
        if geo.is_some() {
            break;
        }
        let base = raw::BOOT_REGION_SECTORS << shift;
        if base + (raw::BOOT_REGION_SECTORS << shift) > dev.block_count().saturating_mul(size as u64) {
            break;
        }
        geo = boot_region(dev, block, base).await?.filter(|geo| geo.sector_shift() == shift);
        if geo.is_some() {
            region = BootRegion::Backup;
        }
    }
    let Some(geo) = geo else {
        let mut head = [0u8; 11];
        read_bytes(dev, block, 0, &mut head).await?;
        return Err(match head[3..] == raw::FILE_SYSTEM_NAME {
            true => Detail::BootSector.corrupt(),
            false => Detail::BootSector.error(ErrorKind::NotRecognized),
        });
    };
    let device_len = dev.block_count().saturating_mul(size as u64);
    let heap_end = geo.heap_start() + ((geo.cluster_count() as u64) << geo.cluster_shift());
    if geo.volume_len() > device_len || heap_end > geo.volume_len() {
        return Err(Detail::BootSector.corrupt());
    }
    Ok((geo, region))
}

/// Finds the root directory's Allocation Bitmap and Up-case Table entries,
/// indexes the up-case table into `upcase`, and returns the volume's
/// state.
///
/// Fails with [`ErrorKind::Corrupt`] when the root directory lacks either
/// entry, or a chain does not hold the structure it names.
pub async fn read_volume<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    geo: Geometry,
    upcase: &mut Upcase,
) -> FsResult<ExFat, D::Error> {
    let (entries, upcase_entry) = system_entries(dev, block, &geo).await?;
    let mut bitmaps = [None; 2];
    for (entry, bitmap) in entries.into_iter().zip(&mut bitmaps) {
        let Some((first, len)) = entry else { continue };
        let extent = extent(dev, block, &geo, first, len).await.map_err(|err| blame(err, Detail::Bitmap))?;
        if extent.len < (geo.cluster_count() as u64).div_ceil(8) {
            return Err(Detail::Bitmap.corrupt());
        }
        *bitmap = Some(extent);
    }
    let (first, len, stored_checksum) = upcase_entry;
    if !(2..=MAX_UPCASE_LEN).contains(&len) {
        return Err(Detail::UpcaseTable.corrupt());
    }
    let table = extent(dev, block, &geo, first, len).await.map_err(|err| blame(err, Detail::UpcaseTable))?;
    index_upcase(dev, block, &geo, table, stored_checksum, upcase).await?;
    let [Some(bitmap), mirror] = bitmaps else {
        return Err(Detail::Bitmap.corrupt());
    };
    Ok(ExFat::new(geo, bitmap, mirror))
}

async fn fat<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, geo: &Geometry, cluster: u32) -> FsResult<u32, D::Error> {
    let mut bytes = [0u8; 4];
    read_bytes(dev, block, geo.fat_start() + cluster as u64 * 4, &mut bytes).await?;
    Ok(u32::from_le_bytes(bytes))
}

/// The first Allocation Bitmap entry of the active FAT, that of the other
/// FAT of a TexFAT volume, and the first Up-case Table entry of the root
/// directory.
#[allow(clippy::type_complexity)]
async fn system_entries<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    geo: &Geometry,
) -> FsResult<([Option<(u32, u64)>; 2], (u32, u64, u32)), D::Error> {
    let cluster_size = geo.cluster_size();
    let mut cluster = geo.root();
    let mut bitmap = None;
    let mut mirror = None;
    let mut upcase = None;
    for _ in 0..geo.cluster_count() {
        let base = geo.cluster_offset(cluster).ok_or(ErrorKind::Corrupt)?;
        let mut at = 0;
        while at < cluster_size {
            let mut entry = [0u8; ENTRY_SIZE];
            read_bytes(dev, block, base + at, &mut entry).await?;
            match entry[0] {
                raw::ENTRY_END => break,
                raw::ENTRY_BITMAP if entry[1] & 1 == geo.active() && bitmap.is_none() => {
                    bitmap = Some((le32(&entry, 20), le64(&entry, 24)));
                }
                raw::ENTRY_BITMAP if geo.mirror_fat().is_some() && entry[1] & 1 != geo.active() && mirror.is_none() => {
                    mirror = Some((le32(&entry, 20), le64(&entry, 24)));
                }
                raw::ENTRY_UPCASE if upcase.is_none() => {
                    upcase = Some((le32(&entry, 20), le64(&entry, 24), le32(&entry, 4)));
                }
                _ => {}
            }
            if let (Some(_), Some(upcase), true) = (bitmap, upcase, mirror.is_some() || geo.mirror_fat().is_none()) {
                return Ok(([bitmap, mirror], upcase));
            }
            at += ENTRY_SIZE as u64;
        }
        if at < cluster_size {
            break;
        }
        match fat(dev, block, geo, cluster).await? {
            raw::FAT_END => break,
            next if geo.is_cluster(next) => cluster = next,
            _ => return Err(Detail::BrokenChain.corrupt()),
        }
    }
    match (bitmap, upcase) {
        (Some(_), Some(upcase)) => Ok(([bitmap, mirror], upcase)),
        (None, _) => Err(Detail::Bitmap.corrupt()),
        (Some(_), None) => Err(Detail::UpcaseTable.corrupt()),
    }
}

/// Checks that the chain at `first` holds `len` bytes, and whether it is
/// contiguous.
async fn extent<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, geo: &Geometry, first: u32, len: u64) -> FsResult<Extent, D::Error> {
    let clusters = len.div_ceil(geo.cluster_size());
    if !geo.is_cluster(first) || clusters == 0 || clusters > geo.cluster_count() as u64 {
        return Err(ErrorKind::Corrupt.into());
    }
    let mut contiguous = true;
    let mut cluster = first;
    for _ in 1..clusters {
        match fat(dev, block, geo, cluster).await? {
            next if geo.is_cluster(next) => {
                contiguous &= next == cluster + 1;
                cluster = next;
            }
            _ => return Err(Detail::BrokenChain.corrupt()),
        }
    }
    Ok(Extent { first, len, contiguous })
}

/// Reads the up-case table once into the empty `upcase`: its checksum,
/// where each page starts, which pages map every code point to itself, and
/// page 0.
async fn index_upcase<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    geo: &Geometry,
    extent: Extent,
    stored_checksum: u32,
    upcase: &mut Upcase,
) -> FsResult<(), D::Error> {
    upcase.extent = extent;
    upcase.stored_checksum = stored_checksum;
    for (code, slot) in upcase.page0.iter_mut().enumerate() {
        *slot = code as u16;
    }
    let cluster_size = geo.cluster_size();
    let mut decoder = UpcaseDecoder::default();
    let mut cluster = extent.first;
    let mut pos = 0u64;
    let mut chunk = [0u8; 64];
    while pos < extent.len {
        let within = pos % cluster_size;
        if pos > 0 && within == 0 {
            cluster = if extent.contiguous {
                cluster + 1
            } else {
                fat(dev, block, geo, cluster).await?
            };
        }
        let base = geo.cluster_offset(cluster).ok_or(ErrorKind::Corrupt)?;
        let n = (chunk.len() as u64).min(cluster_size - within).min(extent.len - pos) as usize;
        read_bytes(dev, block, base + within, &mut chunk[..n]).await?;
        upcase.checksum = raw::table_checksum(upcase.checksum, &chunk[..n]);
        for pair in chunk[..n].chunks_exact(2) {
            let unit = u16::from_le_bytes([pair[0], pair[1]]);
            let Upcase { starts, identity, page0, .. } = &mut *upcase;
            decoder.feed(unit, &mut |code, upper, start| {
                let page = (code >> 8) as usize;
                if code & 0xFF == 0 {
                    starts[page] = start;
                }
                if upper as u32 != code {
                    identity[page / 8] &= !(1 << (page % 8));
                }
                if page == 0 {
                    page0[code as usize] = upper;
                }
            });
        }
        pos += n as u64;
    }
    upcase.identity[0] &= !1;
    upcase.cache[0].0 = 0;
    upcase.cache[1].0 = 0;
    Ok(())
}

/// Up-cases one code unit through the volume's table, decoding its page
/// from the device when it is not cached.
pub async fn upcase<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    geo: &Geometry,
    table: &mut Upcase,
    unit: u16,
) -> FsResult<u16, D::Error> {
    let page = (unit >> 8) as usize;
    let low = (unit & 0xFF) as usize;
    if page == 0 {
        return Ok(table.page0[low]);
    }
    if table.is_identity(page) {
        return Ok(unit);
    }
    if let Some((_, units)) = table.cache.iter().find(|(cached, _)| *cached as usize == page) {
        return Ok(units[low]);
    }
    let slot = table.victim;
    table.victim = (slot + 1) % UPCASE_CACHE;
    table.cache[slot].0 = 0;
    let units = &mut table.cache[slot].1;
    for (index, value) in units.iter_mut().enumerate() {
        *value = (page << 8 | index) as u16;
    }
    let code = (page as u32) << 8;
    let mut decoder = UpcaseDecoder::resume(table.starts[page], code);
    let mut emit = |at: u32, upper: u16, _| {
        if at >> 8 == page as u32 {
            units[(at & 0xFF) as usize] = upper;
        }
    };
    decoder.drain(&mut emit);
    let extent = table.extent;
    let mut hint = ChainPos::NONE;
    while decoder.code() < code + 256 && (decoder.next() as u64) * 2 + 1 < extent.len {
        let pos = decoder.next() as u64 * 2;
        hint = locate(dev, block, geo, extent, hint, (pos >> geo.cluster_shift()) as u32).await?;
        let at = cluster_at(geo, hint.cluster())? + (pos & (geo.cluster_size() - 1));
        let mut pair = [0u8; 2];
        read_bytes(dev, block, at, &mut pair).await?;
        decoder.feed(u16::from_le_bytes(pair), &mut emit);
    }
    table.cache[slot].0 = page as u16;
    Ok(table.cache[slot].1[low])
}

/// The position of cluster `want` of `extent`, walked from `from` when it
/// is not past `want`. A chain that ends first or loops fails with
/// [`ErrorKind::Corrupt`].
async fn locate<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    geo: &Geometry,
    extent: Extent,
    from: ChainPos,
    want: u32,
) -> FsResult<ChainPos, D::Error> {
    if extent.contiguous {
        let cluster = extent.first.checked_add(want).ok_or(ErrorKind::Corrupt)?;
        return Ok(ChainPos::new(want, check_cluster(geo, cluster)?));
    }
    let mut at = if from.cluster() != 0 && from.index() <= want {
        from
    } else {
        ChainPos::start(check_cluster(geo, extent.first)?)
    };
    while at.index() < want {
        let next = next(dev, block, geo, at.cluster()).await?.ok_or(ErrorKind::Corrupt)?;
        if !at.advance(next) {
            return Err(Detail::CyclicChain.corrupt());
        }
    }
    Ok(at)
}

/// Byte offset of slot `slot` of the directory `walk` reads, or `None`
/// past its end. A chain that loops, or is shorter than a contiguous
/// directory claims, fails with [`ErrorKind::Corrupt`].
pub async fn slot_offset<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    geo: &Geometry,
    walk: &mut DirWalk,
    slot: u32,
) -> FsResult<Option<u64>, D::Error> {
    let bytes = slot as u64 * ENTRY_SIZE as u64;
    let dir = walk.dir;
    if dir.first == 0 || bytes >= dir.len || bytes >= raw::MAX_DIRECTORY_SIZE {
        return Ok(None);
    }
    let want = (bytes >> geo.cluster_shift()) as u32;
    let within = bytes & (geo.cluster_size() - 1);
    let cluster = if dir.contiguous {
        check_cluster(geo, dir.first.checked_add(want).ok_or(ErrorKind::Corrupt)?)?
    } else {
        if walk.at.cluster() == 0 || want < walk.at.index() {
            walk.at = ChainPos::start(check_cluster(geo, dir.first)?);
        }
        while walk.at.index() < want {
            match next(dev, block, geo, walk.at.cluster()).await? {
                Some(next) if walk.at.advance(next) => {}
                Some(_) => return Err(Detail::CyclicChain.corrupt()),
                None => return Ok(None),
            }
        }
        walk.at.cluster()
    };
    Ok(Some(cluster_at(geo, cluster)? + within))
}

/// The stored FAT entry of `cluster`, which may be one of the reserved
/// entries 0 and 1. A cluster past the last fails with
/// [`ErrorKind::Corrupt`].
pub async fn get<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, geo: &Geometry, cluster: u32) -> FsResult<u32, D::Error> {
    if cluster > geo.max_cluster() {
        return Err(ErrorKind::Corrupt.into());
    }
    fat(dev, block, geo, cluster).await
}

/// The cluster after `cluster` in its chain, `None` at the end. Any other
/// link fails with [`ErrorKind::Corrupt`].
pub async fn next<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, geo: &Geometry, cluster: u32) -> FsResult<Option<u32>, D::Error> {
    if cluster > geo.max_cluster() {
        return Err(ErrorKind::Corrupt.into());
    }
    match fat(dev, block, geo, cluster).await? {
        raw::FAT_END => Ok(None),
        next if geo.is_cluster(next) => Ok(Some(next)),
        raw::FAT_BAD => Err(Detail::BadCluster.corrupt()),
        _ => Err(Detail::BrokenChain.corrupt()),
    }
}

/// Sets `VolumeDirty` before the first write after mounting or after
/// [`clear_dirty`], unless it was set at mount. Every writing primitive
/// here calls it before it writes.
pub async fn begin_write<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat) -> FsResult<(), D::Error> {
    if vol.dirty == Dirty::Clean {
        write_flags(dev, block, vol, (vol.flags | raw::VOLUME_DIRTY) & !raw::VOLUME_CLEAR_TO_ZERO).await?;
        vol.dirty = Dirty::Marked;
    }
    Ok(())
}

async fn write_flags<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat, flags: u16) -> FsResult<(), D::Error> {
    write_bytes(dev, block, VOLUME_FLAGS_AT, &flags.to_le_bytes()).await?;
    vol.flags = flags;
    Ok(())
}

/// Clears `VolumeDirty` when a write since mounting set it.
pub async fn clear_dirty<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat) -> FsResult<(), D::Error> {
    if vol.dirty == Dirty::Marked {
        write_flags(dev, block, vol, vol.flags & !raw::VOLUME_DIRTY).await?;
        vol.dirty = Dirty::Clean;
    }
    Ok(())
}

/// Writes `PercentInUse` from the free count, counting it first when it
/// is not known.
pub async fn write_percent_in_use<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat) -> FsResult<(), D::Error> {
    let free = count_free(dev, block, vol).await?;
    let used = (vol.geo.cluster_count() - free) as u64;
    let percent = (used * 100 / vol.geo.cluster_count() as u64) as u8;
    begin_write(dev, block, vol).await?;
    write_bytes(dev, block, PERCENT_AT, &[percent]).await?;
    vol.changed = false;
    Ok(())
}

/// Stores `value` as the FAT entry of `cluster`, in the FAT that is not
/// active first on a TexFAT volume.
pub async fn set<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat, cluster: u32, value: u32) -> FsResult<(), D::Error> {
    check_cluster(&vol.geo, cluster)?;
    if let Some(mirror) = vol.geo.mirror_fat() {
        begin_write(dev, block, vol).await?;
        write_bytes(dev, block, mirror + cluster as u64 * 4, &value.to_le_bytes()).await?;
    }
    begin_write(dev, block, vol).await?;
    write_bytes(dev, block, vol.geo.fat_start() + cluster as u64 * 4, &value.to_le_bytes()).await
}

/// Byte offset of byte `pos` of the active bitmap.
async fn bitmap_offset<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat, pos: u64) -> FsResult<u64, D::Error> {
    let hint = locate(dev, block, &vol.geo, vol.bitmap, vol.hint, (pos >> vol.geo.cluster_shift()) as u32).await?;
    vol.hint = hint;
    Ok(cluster_at(&vol.geo, hint.cluster())? + (pos & (vol.geo.cluster_size() - 1)))
}

/// Reads bytes of the active bitmap from byte `pos`, at most to the end of
/// a cluster, and returns how many it read.
pub async fn bitmap_bytes<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    pos: u64,
    out: &mut [u8],
) -> FsResult<usize, D::Error> {
    let within = pos & (vol.geo.cluster_size() - 1);
    let n = (out.len() as u64).min(vol.geo.cluster_size() - within) as usize;
    let at = bitmap_offset(dev, block, vol, pos).await?;
    read_bytes(dev, block, at, &mut out[..n]).await?;
    Ok(n)
}

/// Whether the active bitmap marks `cluster` used.
pub async fn bit<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat, cluster: u32) -> FsResult<bool, D::Error> {
    let index = check_cluster(&vol.geo, cluster)? - raw::FIRST_CLUSTER;
    let mut byte = [0u8; 1];
    bitmap_bytes(dev, block, vol, index as u64 / 8, &mut byte).await?;
    Ok(byte[0] & (1 << (index % 8)) != 0)
}

/// Sets the bitmap bit of `cluster` to `state`, in the bitmap of the FAT
/// that is not active first on a TexFAT volume. A bit already in that
/// state is not written.
pub async fn set_bit<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    cluster: u32,
    state: ClusterState,
) -> FsResult<(), D::Error> {
    let used = state == ClusterState::Used;
    let index = check_cluster(&vol.geo, cluster)? - raw::FIRST_CLUSTER;
    let at = bitmap_offset(dev, block, vol, index as u64 / 8).await?;
    let mut byte = [0u8; 1];
    read_bytes(dev, block, at, &mut byte).await?;
    let bit = 1 << (index % 8);
    let was = byte[0] & bit != 0;
    if was == used {
        return Ok(());
    }
    byte[0] ^= bit;
    if let Some(mirror) = vol.mirror {
        let cluster = locate(dev, block, &vol.geo, mirror, ChainPos::NONE, index >> (vol.geo.cluster_shift() + 3)).await?.cluster();
        let within = (index as u64 / 8) & (vol.geo.cluster_size() - 1);
        let mut other = [0u8; 1];
        let other_at = cluster_at(&vol.geo, cluster)? + within;
        read_bytes(dev, block, other_at, &mut other).await?;
        other[0] = (other[0] & !bit) | (byte[0] & bit);
        begin_write(dev, block, vol).await?;
        write_bytes(dev, block, other_at, &other).await?;
    }
    begin_write(dev, block, vol).await?;
    write_bytes(dev, block, at, &byte).await?;
    vol.adjust_free(1, state);
    Ok(())
}

/// The number of free clusters, counted from the active bitmap when it is
/// not known, and kept in `vol`.
pub async fn count_free<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat) -> FsResult<u32, D::Error> {
    if let Some(free) = vol.free {
        return Ok(free);
    }
    let count = vol.geo.cluster_count();
    let mut used = 0u32;
    let mut pos = 0u64;
    let mut chunk = [0u8; 64];
    let bytes = (count as u64).div_ceil(8);
    while pos < bytes {
        let want = ((bytes - pos) as usize).min(chunk.len());
        let n = bitmap_bytes(dev, block, vol, pos, &mut chunk[..want]).await?;
        for (index, &byte) in chunk[..n].iter().enumerate() {
            let first = (pos + index as u64) * 8;
            let mask = if first + 8 > count as u64 { (1u16 << (count as u64 - first)) as u8 - 1 } else { 0xFF };
            used += (byte & mask).count_ones();
        }
        pos += n as u64;
    }
    let free = count - used.min(count);
    vol.free = Some(free);
    Ok(free)
}

/// Takes a free cluster from the allocation hint on: its FAT entry ends a
/// chain and its bitmap bit is set. With `held`, the cluster is recorded
/// there before anything is written: as the head when there is none, else
/// as the extra cluster.
pub async fn allocate<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    held: Option<&mut Held>,
) -> FsResult<u32, D::Error> {
    let count = vol.geo.cluster_count();
    let from = vol.next_free.clamp(raw::FIRST_CLUSTER, vol.geo.max_cluster()) - raw::FIRST_CLUSTER;
    let mut scanned = 0u32;
    let mut chunk = [0u8; 64];
    let mut index = from & !7;
    while scanned < count + 8 {
        let pos = (index / 8) as u64;
        let want = ((count as u64).div_ceil(8) - pos).min(chunk.len() as u64) as usize;
        let n = bitmap_bytes(dev, block, vol, pos, &mut chunk[..want]).await?;
        for (step, &byte) in chunk[..n].iter().enumerate() {
            if byte == 0xFF {
                continue;
            }
            for bit in 0..8 {
                let candidate = (pos as u32 + step as u32) * 8 + bit;
                if candidate >= count || byte & (1 << bit) != 0 || (scanned == 0 && candidate < from) {
                    continue;
                }
                let cluster = candidate + raw::FIRST_CLUSTER;
                if let Some(held) = held {
                    if held.head() == 0 {
                        held.set_head(cluster);
                    } else {
                        held.set_extra(cluster);
                    }
                }
                set(dev, block, vol, cluster, raw::FAT_END).await?;
                set_bit(dev, block, vol, cluster, ClusterState::Used).await?;
                vol.next_free = if cluster == vol.geo.max_cluster() { raw::FIRST_CLUSTER } else { cluster + 1 };
                return Ok(cluster);
            }
        }
        scanned += n as u32 * 8;
        index += n as u32 * 8;
        if index >= count {
            index = 0;
        }
    }
    vol.free = Some(0);
    Err(ErrorKind::NoSpace.into())
}

/// Allocates a chain of `count` unzeroed clusters, the first `count` free
/// ones from the allocation hint on, and returns its first cluster, 0 when
/// `count` is 0.
///
/// The FAT entries are written a device block at a time from the chain's
/// end back to its start, then the bitmap bits a block at a time; with
/// `held`, the chain is recorded as its head before any bit is set. One
/// cluster is taken with [`allocate`]. On failure the clusters whose bits
/// were set stay allocated, for the caller to free from `held`.
pub async fn allocate_run<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    held: Option<&mut Held>,
    count: u32,
) -> FsResult<u32, D::Error> {
    if count == 0 {
        return Ok(0);
    }
    if count == 1 {
        return allocate(dev, block, vol, held).await;
    }
    let total = vol.geo.cluster_count();
    let from = vol.next_free.clamp(raw::FIRST_CLUSTER, vol.geo.max_cluster()) - raw::FIRST_CLUSTER;
    let at = |step: u32| raw::FIRST_CLUSTER + (from + step) % total;
    let mut found = 0;
    let mut last = None;
    for step in 0..total {
        if !bit(dev, block, vol, at(step)).await? {
            found += 1;
            if found == count {
                last = Some(step);
                break;
            }
        }
    }
    let Some(last) = last else {
        vol.free = Some(found);
        return Err(ErrorKind::NoSpace.into());
    };
    let head = link_free(dev, block, vol, from, last).await?;
    if let Some(held) = held {
        held.set_head(head);
    }
    mark_chain(dev, block, vol, None, head, count, ClusterState::Used).await?;
    let tail = at(last);
    vol.next_free = if tail == vol.geo.max_cluster() { raw::FIRST_CLUSTER } else { tail + 1 };
    Ok(head)
}

/// Links the free clusters among the `last + 1` from cluster index `from`
/// on into a chain, a device block of FAT entries at a time from its end
/// back to its start, and returns its first cluster.
async fn link_free<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat, from: u32, last: u32) -> FsResult<u32, D::Error> {
    let size = block.block_size() as u64;
    let total = vol.geo.cluster_count();
    let at = |step: u32| raw::FIRST_CLUSTER + (from + step) % total;
    let mut head = raw::FAT_END;
    let mut top = last + 1;
    while top > 0 {
        let high = at(top - 1);
        let mut group = ClusterGroup::new(high.saturating_sub(ClusterGroup::SPAN - 1));
        while top > 0 {
            let cluster = at(top - 1);
            if cluster > high || !same_fat_block(&vol.geo, size, cluster, high) {
                break;
            }
            if !bit(dev, block, vol, cluster).await? {
                group.add(cluster);
            }
            top -= 1;
        }
        if group.count > 0 {
            patch_fat(dev, block, vol, &group, head).await?;
            head = group.lowest();
        }
    }
    Ok(head)
}

/// Clears the bitmap bits of the chain at `first`, which nothing links any
/// more, following the FAT; the FAT entries are left as they are. What is
/// left to free is recorded in `held` before each write: `extra` is the
/// part being freed and `head` the rest; both are 0 once the chain is free.
/// The bits of consecutive clusters of the chain that share a device block
/// of each bitmap are cleared with one write.
pub async fn free_chain<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    held: &mut Held,
    first: u32,
) -> FsResult<(), D::Error> {
    let cluster = check_cluster(&vol.geo, first)?;
    mark_chain(dev, block, vol, Some(held), cluster, vol.geo.cluster_count(), ClusterState::Free).await?;
    *held = Held::NONE;
    Ok(())
}

/// Sets the bitmap bits of the chain at `cluster` to `state`, at most
/// `limit` clusters of it, and fails with [`ErrorKind::Corrupt`] when it is
/// longer. Clusters whose bits share a device block of each bitmap take
/// one write. With `held`, what is left of the chain is recorded there
/// before each write.
#[allow(clippy::too_many_arguments)]
async fn mark_chain<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    mut held: Option<&mut Held>,
    mut cluster: u32,
    limit: u32,
    state: ClusterState,
) -> FsResult<(), D::Error> {
    let used = state == ClusterState::Used;
    let mut steps = 0;
    loop {
        let start = cluster;
        let mut group = ClusterGroup::new(cluster);
        let place = bit_place(dev, block, vol, cluster).await?;
        let next = loop {
            if steps == limit {
                break Err(ErrorKind::Corrupt.into());
            }
            steps += 1;
            let next = if used && steps == limit { Ok(None) } else { next(dev, block, &vol.geo, cluster).await };
            if next.is_ok() {
                group.add(cluster);
            }
            match next {
                Ok(Some(next)) if group.spans(next) && !group.has(next) && bit_place(dev, block, vol, next).await? == place => {
                    cluster = next;
                }
                other => break other,
            }
        };
        if group.count > 0 {
            if let Some(held) = held.as_deref_mut() {
                *held = Held::new(next.as_ref().ok().copied().flatten().unwrap_or(0), start);
            }
            patch_bits(dev, block, vol, &group, place, state).await?;
        }
        match next? {
            Some(next) => cluster = next,
            None => return Ok(()),
        }
    }
}

/// Where the bitmap bits around `cluster` are: for the bitmap and the
/// mirror bitmap, the device block holding its bit and the offset bitmap
/// byte 0 would have if the bitmap were contiguous up to it. Clusters with
/// the same place have their bits in one device block.
async fn bit_place<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, vol: &mut ExFat, cluster: u32) -> FsResult<[(u64, u64); 2], D::Error> {
    let size = block.block_size() as u64;
    let byte = (check_cluster(&vol.geo, cluster)? - raw::FIRST_CLUSTER) as u64 / 8;
    let at = bitmap_offset(dev, block, vol, byte).await?;
    let mut place = [(at / size, at - byte), (0, 0)];
    if let Some(mirror) = vol.mirror {
        let other = locate(dev, block, &vol.geo, mirror, ChainPos::NONE, (byte >> vol.geo.cluster_shift()) as u32).await?.cluster();
        let at = cluster_at(&vol.geo, other)? + (byte & (vol.geo.cluster_size() - 1));
        place[1] = (at / size, at - byte);
    }
    Ok(place)
}

/// Sets or clears the bitmap bits of `group`, which share `place`: the
/// mirror bitmap first, as `set_bit` does.
async fn patch_bits<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    group: &ClusterGroup,
    place: [(u64, u64); 2],
    state: ClusterState,
) -> FsResult<(), D::Error> {
    let used = state == ClusterState::Used;
    begin_write(dev, block, vol).await?;
    let size = block.block_size() as u64;
    let mut flips = 0;
    let order = if vol.mirror.is_some() { &[1, 0][..] } else { &[0][..] };
    for &which in order {
        let (index, origin) = place[which];
        load(dev, block, index).await?;
        let data = block.contents_mut();
        for cluster in group.descending() {
            let bit = cluster - raw::FIRST_CLUSTER;
            let byte = (origin + bit as u64 / 8 - index * size) as usize;
            let mask = 1 << (bit % 8);
            let Some(byte) = data.get_mut(byte) else {
                return Err(ErrorKind::Corrupt.into());
            };
            if which == 0 && (*byte & mask != 0) != used {
                flips += 1;
            }
            if used {
                *byte |= mask;
            } else {
                *byte &= !mask;
            }
        }
        store(dev, block, index).await?;
    }
    vol.adjust_free(flips, state);
    Ok(())
}

/// Writes the FAT entries of `group`, which share a device block of every
/// FAT, linking them in ascending order with the highest pointing to
/// `next`: the mirror FAT first, as `set` does.
async fn patch_fat<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    group: &ClusterGroup,
    next: u32,
) -> FsResult<(), D::Error> {
    begin_write(dev, block, vol).await?;
    let size = block.block_size() as u64;
    for base in [vol.geo.mirror_fat(), Some(vol.geo.fat_start())].into_iter().flatten() {
        let index = (base + group.lowest() as u64 * 4) / size;
        load(dev, block, index).await?;
        let data = block.contents_mut();
        let mut value = next;
        for cluster in group.descending() {
            let at = (base + cluster as u64 * 4 - index * size) as usize;
            let Some(entry) = data.get_mut(at..at + 4) else {
                return Err(ErrorKind::Corrupt.into());
            };
            entry.copy_from_slice(&value.to_le_bytes());
            value = cluster;
        }
        store(dev, block, index).await?;
    }
    Ok(())
}

/// Writes `entries`, a set or its first entries, at the offsets `at`, one
/// device write for each run of entries that follow each other within one
/// device block, the run holding the first entry last. A set inside one
/// block is written at once.
pub async fn write_set<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    at: &[u64],
    entries: &[RawEntry],
) -> FsResult<(), D::Error> {
    put_entries(dev, block, vol, at, entries, SetOrder::PrimaryLast).await
}

/// Marks the entries of a set unused in `entries` and writes them at the
/// offsets `at` as [`write_set`] does, but the run holding the File entry
/// first.
pub async fn clear_set<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    at: &[u64],
    entries: &mut [RawEntry],
) -> FsResult<(), D::Error> {
    for entry in entries.iter_mut() {
        entry[0] &= !raw::IN_USE;
    }
    put_entries(dev, block, vol, at, entries, SetOrder::PrimaryFirst).await
}

async fn put_entries<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    vol: &mut ExFat,
    at: &[u64],
    entries: &[RawEntry],
    order: SetOrder,
) -> FsResult<(), D::Error> {
    let size = block.block_size() as u64;
    let joined = |index: usize| {
        index > 0 && at[index] == at[index - 1] + ENTRY_SIZE as u64 && at[index] / size == at[index - 1] / size
    };
    let count = at.len().min(entries.len());
    let mut done = 0;
    while done < count {
        let (from, to) = match order {
            SetOrder::PrimaryLast => {
                let to = count - done;
                let mut from = to - 1;
                while joined(from) {
                    from -= 1;
                }
                (from, to)
            }
            SetOrder::PrimaryFirst => {
                let from = done;
                let mut to = from + 1;
                while to < count && joined(to) {
                    to += 1;
                }
                (from, to)
            }
        };
        begin_write(dev, block, vol).await?;
        write_bytes(dev, block, at[from], entries[from..to].as_flattened()).await?;
        done += to - from;
    }
    Ok(())
}

}

#[derive(Clone, Copy)]
enum SetOrder {
    PrimaryLast,
    PrimaryFirst,
}
