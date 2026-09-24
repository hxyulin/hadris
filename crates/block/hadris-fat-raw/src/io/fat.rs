use hadris_fs::{DateTime, ErrorKind, FsResult};

use super::block::{load, put, read_bytes, store, write_bytes, write_zeros};
use super::storage::BlockDevice;
use crate::boot::{BOOT_SECTOR_LEN, Geometry, check_fs_info, parse_boot};
use crate::bpb::RawFsInfo;
use crate::date;
use crate::dirent::{ATTR_VOLUME_ID, ENTRY_FREE, ENTRY_SIZE};
use crate::entry::{FIRST_DATA_CLUSTER, FatKind};
use crate::io::{BlockBuf, ChainPos, ClusterGroup, DirStart, DirWalk, Fat, Held};
use crate::layout::{self, BACKUP_BOOT_SECTOR, BootFields, FS_INFO_SECTOR, Layout, ROOT_CLUSTER};
use crate::slot::{MAX_DIR_ENTRIES, ShortEntry, Slot};

/// Offset of `FSI_Free_Count` in the FSInfo sector; `FSI_Nxt_Free` follows.
const FSINFO_FREE_COUNT: u64 = 488;
const UNKNOWN_FREE: u32 = u32::MAX;

/// Whether the entries of `a` and `b` lie in one device block of every FAT
/// copy written.
fn same_fat_block(fat: &Fat, size: u64, a: u32, b: u32) -> bool {
    let geo = &fat.geo;
    let kind = geo.kind();
    (0..geo.copies()).all(|step| {
        let base = geo.fat_copy((geo.active_fat() + step) % geo.fat_count());
        (base + kind.entry_offset(a as u64)) / size == (base + kind.entry_offset(b as u64)) / size
    })
}

/// The first cluster whose entry lies in the active copy's device block
/// holding the entry of `cluster`.
fn block_base(fat: &Fat, size: u64, cluster: u32) -> u32 {
    let geo = &fat.geo;
    let base = geo.fat_copy(geo.active_fat());
    let at = base + geo.kind().entry_offset(cluster as u64);
    let start = (at / size * size).saturating_sub(base);
    (start / geo.kind().entry_len() as u64) as u32
}

fn after(fat: &Fat, cluster: u32) -> u32 {
    if cluster == fat.geo.max_cluster() {
        FIRST_DATA_CLUSTER
    } else {
        cluster + 1
    }
}

io_transform! {

/// Reads and checks the boot sector, and checks that the volume fits the
/// device.
///
/// Fails with [`ErrorKind::InvalidInput`] when `block` is not sized to the
/// device's blocks, with [`ErrorKind::NotRecognized`] when the first sector
/// has no FAT BIOS parameter block, and with [`ErrorKind::Corrupt`] when
/// the boot sector is not a valid FAT12, FAT16 or FAT32 boot sector or
/// describes a volume larger than the device.
pub async fn read_geometry<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf) -> FsResult<Geometry, D::Error> {
    if block.size as u64 != dev.block_size().get() as u64 {
        return Err(ErrorKind::InvalidInput.into());
    }
    let mut sector = [0u8; BOOT_SECTOR_LEN];
    read_bytes(dev, block, 0, &mut sector).await?;
    let geo = parse_boot(&sector).map_err(|err| err.kind())?;
    let device_len = dev.block_count().saturating_mul(block.size as u64);
    if geo.data_end() > device_len {
        return Err(ErrorKind::Corrupt.into());
    }
    Ok(geo)
}

/// The [`Fat`] state of a volume with geometry `geo`, with the free count
/// and allocation hint of its FAT32 FSInfo sector when that is valid.
pub async fn read_fat<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, geo: Geometry) -> FsResult<Fat, D::Error> {
    let mut fat = Fat::new(geo);
    if let Some(at) = geo.fs_info_sector() {
        let offset = at as u64 * geo.sector_size() as u64;
        let mut sector = [0u8; BOOT_SECTOR_LEN];
        read_bytes(dev, block, offset, &mut sector).await?;
        let info: RawFsInfo = bytemuck::pod_read_unaligned(&sector);
        let valid = check_fs_info(&info).is_ok();
        let free = info.free_count.get();
        let hint = info.next_free.get();
        if valid && free < geo.max_cluster() {
            fat.free = Some(free);
        }
        if valid && (FIRST_DATA_CLUSTER..=geo.max_cluster()).contains(&hint) {
            fat.next_free = hint;
        }
        fat.fs_info = valid.then_some(offset);
    }
    Ok(fat)
}

/// The stored entry of data cluster `cluster` in the active FAT copy.
/// Other clusters fail with [`ErrorKind::Corrupt`].
pub async fn get<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, fat: &Fat, cluster: u32) -> FsResult<u32, D::Error> {
    fat.check_cluster(cluster)?;
    let kind = fat.geo.kind();
    let mut bytes = [0u8; 4];
    let at = fat.geo.fat_copy(fat.geo.active_fat()) + kind.entry_offset(cluster as u64);
    read_bytes(dev, block, at, &mut bytes[..kind.entry_len()]).await?;
    Ok(kind.decode(cluster as u64, &bytes))
}

/// The stored entry of data cluster `cluster` in FAT copy `copy`. Fails
/// with [`ErrorKind::InvalidInput`] for a copy the volume does not have and
/// with [`ErrorKind::Corrupt`] for a cluster that is not a data cluster.
pub async fn get_copy<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    copy: u8,
    cluster: u32,
) -> FsResult<u32, D::Error> {
    if copy >= fat.geo.fat_count() {
        return Err(ErrorKind::InvalidInput.into());
    }
    fat.check_cluster(cluster)?;
    let kind = fat.geo.kind();
    let mut bytes = [0u8; 4];
    let at = fat.geo.fat_copy(copy) + kind.entry_offset(cluster as u64);
    read_bytes(dev, block, at, &mut bytes[..kind.entry_len()]).await?;
    Ok(kind.decode(cluster as u64, &bytes))
}

/// The cluster after `cluster` in its chain, `None` at the end. A link to
/// a free, reserved or bad cluster, or out of range, fails with
/// [`ErrorKind::Corrupt`].
pub async fn next<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, fat: &Fat, cluster: u32) -> FsResult<Option<u32>, D::Error> {
    let stored = get(dev, block, fat, cluster).await?;
    Ok(fat.geo.kind().next(stored, fat.geo.max_cluster()).map_err(|_| ErrorKind::Corrupt)?)
}

/// Stores `value` as the FAT entry of `cluster`, in the active copy first,
/// then in the others when the volume mirrors its FATs.
///
/// Until every copy has it, the entry is recorded as
/// [`unmirrored`](Fat::unmirrored), and the next `set` or [`mirror`]
/// copies it. Entries an earlier `set` left unmirrored are copied first.
pub async fn set<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, fat: &mut Fat, cluster: u32, value: u32) -> FsResult<(), D::Error> {
    fat.check_cluster(cluster)?;
    let copies = fat.geo.copies();
    if copies > 1 {
        if let Some(other) = fat.unmirrored
            && other != (cluster, cluster)
        {
            mirror(dev, block, fat).await?;
        }
        fat.unmirrored = Some((cluster, cluster));
    }
    for step in 0..copies {
        put_entry(dev, block, fat, step, cluster, value).await?;
    }
    fat.unmirrored = None;
    Ok(())
}

/// Copies the active FAT entries an interrupted [`set`], `allocate_run` or
/// `free_chain` left [`unmirrored`](Fat::unmirrored) to the other copies,
/// where they differ.
pub async fn mirror<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, fat: &mut Fat) -> FsResult<(), D::Error> {
    let Some((first, last)) = fat.unmirrored else {
        return Ok(());
    };
    let kind = fat.geo.kind();
    let count = fat.geo.fat_count();
    if fat.geo.mirrored() {
        for cluster in first..=last.min(fat.geo.max_cluster()) {
            let value = get(dev, block, fat, cluster).await? & kind.mask();
            for step in 1..count {
                let copy = (fat.geo.active_fat() + step) % count;
                let at = fat.geo.fat_copy(copy) + kind.entry_offset(cluster as u64);
                let mut bytes = [0u8; 4];
                read_bytes(dev, block, at, &mut bytes[..kind.entry_len()]).await?;
                if kind.decode(cluster as u64, &bytes) & kind.mask() != value {
                    put_entry(dev, block, fat, step, cluster, value).await?;
                }
            }
        }
    }
    fat.unmirrored = None;
    Ok(())
}

/// Stores `value` as the entry of `cluster` in the FAT copy `step` copies
/// after the active one. A write to the active copy that frees or takes
/// the cluster updates the free count.
async fn put_entry<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &mut Fat,
    step: u8,
    cluster: u32,
    value: u32,
) -> FsResult<(), D::Error> {
    let kind = fat.geo.kind();
    let len = kind.entry_len();
    let copy = (fat.geo.active_fat() + step) % fat.geo.fat_count();
    let at = fat.geo.fat_copy(copy) + kind.entry_offset(cluster as u64);
    let mut bytes = [0u8; 4];
    read_bytes(dev, block, at, &mut bytes[..len]).await?;
    let was_free = kind.decode(cluster as u64, &bytes) & kind.mask() == 0;
    kind.encode(cluster as u64, value, &mut bytes);
    put(dev, block, at, Some(&bytes[..len]), len).await?;
    let is_free = value & kind.mask() == 0;
    if step == 0 && was_free != is_free {
        if is_free {
            fat.adjust_free(1, 0);
        } else {
            fat.adjust_free(0, 1);
        }
    }
    Ok(())
}

/// Walks the chain at `first` towards cluster index `want`, from `from`
/// when that is a known position at or before `want`. Returns the position
/// reached, short of `want` when the chain ends first. A chain that loops
/// fails with [`ErrorKind::Corrupt`].
pub async fn walk<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    first: u32,
    from: ChainPos,
    want: u32,
) -> FsResult<ChainPos, D::Error> {
    let mut at = if from.cluster != 0 && from.index <= want {
        from
    } else {
        ChainPos::start(fat.check_cluster(first)?)
    };
    while at.index < want {
        match next(dev, block, fat, at.cluster).await? {
            Some(next) if at.advance(next) => {}
            Some(_) => return Err(ErrorKind::Corrupt.into()),
            None => break,
        }
    }
    Ok(at)
}

/// Extends `n` bytes that start in the cluster at `at` over the clusters
/// that follow it on disk and in its chain, up to `max` bytes, so they can
/// take one device call. Returns the length and leaves `at` at the run's
/// last cluster.
pub async fn run<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    at: &mut ChainPos,
    mut n: usize,
    max: usize,
) -> FsResult<usize, D::Error> {
    let cluster_size = fat.geo.cluster_size() as usize;
    while n < max {
        match next(dev, block, fat, at.cluster).await? {
            Some(next) if next == at.cluster + 1 => {
                if !at.advance(next) {
                    return Err(ErrorKind::Corrupt.into());
                }
                n = (n + cluster_size).min(max);
            }
            _ => break,
        }
    }
    Ok(n)
}

/// Takes a free cluster, from [`next_free`](Fat::next_free) on, and marks
/// it as the end of a chain. The FAT is scanned whatever the free count
/// says, since FSInfo is only a hint.
///
/// With `held`, the cluster is recorded there before it is marked: as the
/// head when there is none, else as the extra cluster.
pub async fn allocate<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &mut Fat,
    held: Option<&mut Held>,
) -> FsResult<u32, D::Error> {
    let max = fat.geo.max_cluster();
    let kind = fat.geo.kind();
    let count = max - FIRST_DATA_CLUSTER + 1;
    let from = fat.next_free.clamp(FIRST_DATA_CLUSTER, max) - FIRST_DATA_CLUSTER;
    for step in 0..count {
        let cluster = FIRST_DATA_CLUSTER + (from + step) % count;
        if get(dev, block, fat, cluster).await? & kind.mask() != 0 {
            continue;
        }
        if let Some(held) = held {
            if held.head == 0 {
                held.head = cluster;
            } else {
                held.extra = cluster;
            }
        }
        set(dev, block, fat, cluster, kind.end_of_chain()).await?;
        fat.next_free = after(fat, cluster);
        fat.info_dirty = true;
        return Ok(cluster);
    }
    if fat.free != Some(0) {
        fat.free = Some(0);
        fat.info_dirty = true;
    }
    Err(ErrorKind::NoSpace.into())
}

/// Allocates a chain of `count` clusters, the first `count` free ones from
/// [`next_free`](Fat::next_free) on, and returns its first cluster, 0 when
/// `count` is 0. The clusters are not zeroed.
///
/// Every written entry points to an allocated cluster or ends the chain.
/// On FAT16 and FAT32 the entries are written a device block at a time,
/// from the chain's end back to its start; on FAT12, or for one cluster,
/// the clusters are taken one by one with [`allocate`] and each linked to
/// the one before. With `held`, what is written is recorded there before
/// it is written: `head` is the part of the chain already linked, and
/// `extra` a cluster taken but not yet linked to it. On failure the
/// clusters stay allocated, for the caller to free from `held`.
pub async fn allocate_run<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &mut Fat,
    mut held: Option<&mut Held>,
    count: u32,
) -> FsResult<u32, D::Error> {
    let kind = fat.geo.kind();
    if kind == FatKind::Fat12 || count <= 1 {
        let (mut first, mut last) = (0, 0);
        for _ in 0..count {
            let cluster = allocate(dev, block, fat, held.as_deref_mut()).await?;
            if last != 0 {
                set(dev, block, fat, last, cluster).await?;
                if let Some(held) = held.as_deref_mut() {
                    held.extra = 0;
                }
            }
            if first == 0 {
                first = cluster;
            }
            last = cluster;
        }
        return Ok(first);
    }
    let size = block.size as u64;
    let max = fat.geo.max_cluster();
    let total = max - FIRST_DATA_CLUSTER + 1;
    let from = fat.next_free.clamp(FIRST_DATA_CLUSTER, max) - FIRST_DATA_CLUSTER;
    let at = |step: u32| FIRST_DATA_CLUSTER + (from + step) % total;
    let mut found = 0;
    let mut last = None;
    for step in 0..total {
        if get(dev, block, fat, at(step)).await? & kind.mask() == 0 {
            found += 1;
            if found == count {
                last = Some(step);
                break;
            }
        }
    }
    let Some(last) = last else {
        if fat.free != Some(found) {
            fat.free = Some(found);
            fat.info_dirty = true;
        }
        return Err(ErrorKind::NoSpace.into());
    };
    let end = kind.end_of_chain();
    let mut head = end;
    let mut top = last + 1;
    while top > 0 {
        let high = at(top - 1);
        let mut group = ClusterGroup::new(high.saturating_sub(ClusterGroup::SPAN - 1));
        while top > 0 {
            let cluster = at(top - 1);
            if cluster > high || !same_fat_block(fat, size, cluster, high) {
                break;
            }
            let free = get(dev, block, fat, cluster).await? & kind.mask() == 0;
            if free {
                group.add(cluster);
            }
            top -= 1;
        }
        if group.count == 0 {
            continue;
        }
        if let Some(held) = held.as_deref_mut() {
            held.head = if head == end { 0 } else { head };
            held.extra = group.lowest();
        }
        patch(dev, block, fat, &group, Some(head)).await?;
        head = group.lowest();
        if let Some(held) = held.as_deref_mut() {
            held.head = head;
            held.extra = 0;
        }
    }
    fat.next_free = after(fat, at(last));
    Ok(head)
}

/// Frees the chain at `first`, which nothing links any more. What is left
/// to free is recorded in `held` before each write: `extra` is the part
/// being freed and `head` the rest; both are 0 once the chain is free.
///
/// A freed cluster reads as free, so a cyclic chain ends with
/// [`ErrorKind::Corrupt`], with what is left of it in `held`. On FAT16 and
/// FAT32 the entries of consecutive clusters of the chain that share a
/// device block of every FAT copy are freed with one write per copy.
pub async fn free_chain<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &mut Fat,
    held: &mut Held,
    first: u32,
) -> FsResult<(), D::Error> {
    let mut cluster = fat.check_cluster(first)?;
    if fat.geo.kind() == FatKind::Fat12 {
        loop {
            let next = next(dev, block, fat, cluster).await?;
            *held = Held::new(next.unwrap_or(0), cluster);
            set(dev, block, fat, cluster, 0).await?;
            match next {
                Some(next) => cluster = next,
                None => {
                    *held = Held::NONE;
                    return Ok(());
                }
            }
        }
    }
    let size = block.size as u64;
    loop {
        let start = cluster;
        let base = block_base(fat, size, cluster);
        let mut group = ClusterGroup::new(base);
        let next = loop {
            if group.has(cluster) {
                break Err(ErrorKind::Corrupt.into());
            }
            let next = next(dev, block, fat, cluster).await;
            if next.is_ok() {
                group.add(cluster);
            }
            match next {
                Ok(Some(next)) if next >= base && same_fat_block(fat, size, cluster, next) => cluster = next,
                other => break other,
            }
        };
        if group.count > 0 {
            let after = next.as_ref().ok().copied().flatten();
            *held = Held::new(after.unwrap_or(0), start);
            patch(dev, block, fat, &group, None).await?;
        }
        match next? {
            Some(next) => cluster = next,
            None => {
                *held = Held::NONE;
                return Ok(());
            }
        }
    }
}

/// Writes the entries of `group`, which share a device block in every
/// copy, one block write per copy, active copy first. With `chain`, the
/// clusters are linked in ascending order and the highest points to
/// `chain`; without, they are freed. The free count follows the active
/// copy.
async fn patch<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &mut Fat,
    group: &ClusterGroup,
    chain: Option<u32>,
) -> FsResult<(), D::Error> {
    let kind = fat.geo.kind();
    let len = kind.entry_len();
    let size = block.size as u64;
    let copies = fat.geo.copies();
    if copies > 1 {
        let range = (group.lowest(), group.descending().next().unwrap_or(group.lowest()));
        if let Some(other) = fat.unmirrored
            && other != range
        {
            mirror(dev, block, fat).await?;
        }
        fat.unmirrored = Some(range);
    }
    for step in 0..copies {
        let base = fat.geo.fat_copy((fat.geo.active_fat() + step) % fat.geo.fat_count());
        let index = (base + kind.entry_offset(group.lowest() as u64)) / size;
        load(dev, block, index).await?;
        let data = block.contents_mut();
        let mut value = chain.unwrap_or(0);
        let (mut taken, mut freed) = (0, 0);
        for cluster in group.descending() {
            let at = (base + kind.entry_offset(cluster as u64) - index * size) as usize;
            let Some(entry) = data.get_mut(at..at + len) else {
                return Err(ErrorKind::Corrupt.into());
            };
            let was_free = kind.decode(cluster as u64, entry) & kind.mask() == 0;
            kind.encode(cluster as u64, value, entry);
            match (was_free, value & kind.mask() == 0) {
                (true, false) => taken += 1,
                (false, true) => freed += 1,
                _ => {}
            }
            if chain.is_some() {
                value = cluster;
            }
        }
        store(dev, block, index).await?;
        if step == 0 && taken != freed {
            fat.adjust_free(freed, taken);
        }
    }
    fat.unmirrored = None;
    Ok(())
}

/// Counts the free clusters with one scan of the FAT and keeps the count
/// in `fat`.
pub async fn count_free<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, fat: &mut Fat) -> FsResult<u32, D::Error> {
    let mask = fat.geo.kind().mask();
    let mut free = 0;
    for cluster in FIRST_DATA_CLUSTER..=fat.geo.max_cluster() {
        if get(dev, block, fat, cluster).await? & mask == 0 {
            free += 1;
        }
    }
    fat.free = Some(free);
    Ok(free)
}

/// Writes the free count and allocation hint to the FAT32 FSInfo sector
/// when they changed since it was read or last written.
pub async fn write_fs_info<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, fat: &mut Fat) -> FsResult<(), D::Error> {
    if fat.info_dirty
        && let Some(at) = fat.fs_info
    {
        let mut info = [0u8; 8];
        info[..4].copy_from_slice(&fat.free.unwrap_or(UNKNOWN_FREE).to_le_bytes());
        info[4..].copy_from_slice(&fat.next_free.to_le_bytes());
        write_bytes(dev, block, at + FSINFO_FREE_COUNT, &info).await?;
        fat.info_dirty = false;
    }
    Ok(())
}

/// Byte offset of slot `slot` of the directory `walk` reads, or `None` past
/// its end. The walk keeps its position in the directory's chain, so the
/// slots of one cluster cost no FAT reads and later clusters are reached
/// from it.
pub async fn slot_offset<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    walk: &mut DirWalk,
    slot: u32,
) -> FsResult<Option<u64>, D::Error> {
    if slot >= MAX_DIR_ENTRIES {
        return Ok(None);
    }
    match walk.start {
        DirStart::Fixed { start, slots } => {
            Ok((slot < slots).then(|| start + slot as u64 * ENTRY_SIZE as u64))
        }
        DirStart::Chain(first) => {
            let per_cluster = fat.geo.cluster_size() / ENTRY_SIZE as u32;
            let want = slot / per_cluster;
            walk.at = self::walk(dev, block, fat, first, walk.at, want).await?;
            if walk.at.index < want {
                return Ok(None);
            }
            let within = (slot % per_cluster) as u64 * ENTRY_SIZE as u64;
            Ok(Some(fat.cluster_at(walk.at.cluster)? + within))
        }
    }
}


/// Parses the slot at byte `offset` in place in `block`. Slots are aligned
/// to 32 bytes; an unaligned offset whose slot crosses a device block
/// fails with [`ErrorKind::InvalidInput`].
pub async fn read_slot<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, offset: u64) -> FsResult<Slot, D::Error> {
    let size = block.size as u64;
    load(dev, block, offset / size).await?;
    let at = (offset % size) as usize;
    let raw = block
        .contents()
        .get(at..at + ENTRY_SIZE)
        .and_then(|raw| raw.try_into().ok())
        .ok_or(ErrorKind::InvalidInput)?;
    Ok(Slot::parse(raw))
}

/// Writes `count` consecutive slots of the directory `walk` reads, from
/// slot `first` on, in order, each as `encode` returns it for its index
/// from 0. `written` counts the slots written, so a caller can clear them
/// when a later one fails. A slot past the directory's end fails with
/// [`ErrorKind::Corrupt`].
///
/// A long name goes before its short entry, so an interruption leaves at
/// worst long-name slots without their short entry, which readers skip.
#[allow(clippy::too_many_arguments)]
pub async fn write_slots<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    walk: &mut DirWalk,
    first: u32,
    count: u32,
    mut encode: impl FnMut(u32) -> [u8; ENTRY_SIZE],
    written: &mut u32,
) -> FsResult<(), D::Error> {
    *written = 0;
    while *written < count {
        let at = slot_offset(dev, block, fat, walk, first + *written)
            .await?
            .ok_or(ErrorKind::Corrupt)?;
        write_bytes(dev, block, at, &encode(*written)).await?;
        *written += 1;
    }
    Ok(())
}

/// Marks slots `from..to` of the directory at `dir` deleted.
pub async fn clear_slots<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    fat: &Fat,
    dir: DirStart,
    from: u32,
    to: u32,
) -> FsResult<(), D::Error> {
    let mut walk = DirWalk::new(dir);
    for slot in from..to {
        let at = slot_offset(dev, block, fat, &mut walk, slot).await?.ok_or(ErrorKind::Corrupt)?;
        write_bytes(dev, block, at, &[ENTRY_FREE]).await?;
    }
    Ok(())
}

/// Writes the volume `layout` plans, with the boot sector fields `fields`,
/// and returns its geometry. `now` stamps the label entry.
///
/// Everything before the data region is written: the boot sector, on FAT32
/// the FSInfo sector and their backups, the FATs, the root directory and
/// its label entry. The data region is left as it is. The boot sector goes
/// last, so an interrupted format leaves a device that does not mount. The
/// device is not flushed.
pub async fn mkfs<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    layout: &Layout,
    fields: &BootFields,
    now: DateTime,
) -> FsResult<Geometry, D::Error> {
    let boot = layout::encode_boot_sector(layout, fields);
    let geo = parse_boot(&boot).map_err(|_| ErrorKind::InvalidInput)?;
    let sector = layout.sector_size() as u64;
    let data_start = layout.data_start();
    let root = if layout.kind() == FatKind::Fat32 {
        data_start
    } else {
        layout.root_start()
    };
    let root_end = if layout.kind() == FatKind::Fat32 {
        data_start + layout.cluster_size() as u64
    } else {
        data_start
    };
    let root_end = usize::try_from(root_end).map_err(|_| ErrorKind::LimitExceeded)?;
    write_zeros(dev, block, 0, root_end).await?;

    let (entries, len) = layout::reserved_fat_entries(layout.kind(), fields.media());
    for copy in 0..layout.fat_count() as u64 {
        let at = layout.fat_start() + copy * layout.fat_sectors() as u64 * sector;
        write_bytes(dev, block, at, &entries[..len]).await?;
    }
    if let Some(label) = fields.label() {
        let mut entry = ShortEntry::new(label, ATTR_VOLUME_ID);
        let (date, time, tenths) = date::encode(now);
        entry.set_created(date, time, tenths);
        entry.set_modified(date, time);
        entry.set_accessed_date(date);
        write_bytes(dev, block, root, &entry.encode()).await?;
    }

    if layout.kind() == FatKind::Fat32 {
        let free = layout.clusters() - 1;
        let info = layout::encode_fs_info(free, ROOT_CLUSTER + 1);
        let backup = BACKUP_BOOT_SECTOR as u64 * sector;
        write_bytes(dev, block, backup, &boot).await?;
        write_bytes(dev, block, backup + sector, &info).await?;
        write_bytes(dev, block, FS_INFO_SECTOR as u64 * sector, &info).await?;
    }
    write_bytes(dev, block, 0, &boot).await?;
    Ok(geo)
}

}
