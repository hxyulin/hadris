use hadris_fs::{CheckReport, ErrorKind, Finding, FsResult, Location, Severity};

use super::block::read_bytes;
use super::fat::{get, read_slot, slot_offset};
use super::storage::BlockDevice;
use crate::boot::{BOOT_SECTOR_LEN, Geometry, RootLocation, check_fs_info, parse_boot};
use crate::bpb::{RawBpb, RawBpbExt32, RawFsInfo};
use crate::detail::Detail;
use crate::dirent::{ENTRY_SIZE, LFN_LAST_ENTRY, LFN_SEQUENCE_MASK};
use crate::entry::{ChainError, FIRST_DATA_CLUSTER, FatKind};
use crate::io::{BlockBuf, DirStart, DirWalk, Fat};
use crate::layout::BACKUP_BOOT_SECTOR;
use crate::lfn;
use crate::short_name;
use crate::slot::{MAX_DIR_ENTRIES, ShortEntry, Slot};

/// The largest device block `check` reads.
const MAX_BLOCK: usize = 4096;
/// Bytes of the scratch buffer that hold the path of a finding.
const PATH_LEN: usize = 1024;
/// The smallest cluster bitmap: 4096 clusters a pass.
const MIN_BITMAP: usize = 512;
/// Bytes of each FAT copy compared at once.
const CHUNK: usize = 512;
const DOT: [u8; 11] = *b".          ";
const DOT_DOT: [u8; 11] = *b"..         ";
/// Bytes a short name may not hold, besides controls and lowercase.
const BAD_NAME_BYTES: &[u8] = b"\"*+,./:;<=>?[\\]|\x7F";
/// Offset of `FSI_Free_Count` in the FSInfo sector.
const FSINFO_FREE_COUNT: u64 = 488;
const UNKNOWN_FREE: u32 = u32::MAX;
const BPB_LEN: usize = size_of::<RawBpb>();

/// Where a chain ends.
#[derive(Clone, Copy)]
enum End {
    Eoc,
    Bad(u32),
    Broken(u32),
    Cycle(u32),
}

/// A chain's length in distinct clusters and how it ends.
#[derive(Clone, Copy)]
struct Chain {
    len: u32,
    end: End,
}

enum Link {
    Next(u32),
    End(End),
}

/// A directory being scanned.
#[derive(Clone, Copy)]
struct Dir {
    start: DirStart,
    /// Its first cluster, 0 for the FAT12/16 root.
    cluster: u32,
    /// The slots its chain holds.
    slots: u32,
    root: bool,
}

/// A long-name run being read.
#[derive(Clone, Copy, Default)]
struct Run {
    pending: bool,
    start: u64,
    /// The sequence number the next fragment must carry; 0 once complete.
    expect: u8,
    checksum: u8,
    /// Code units of the name so far.
    units: usize,
    /// Fragments since the last reported orphan belong to it.
    orphan: bool,
}

/// What a finding's path names.
#[derive(Clone, Copy)]
enum Scope {
    Volume,
    /// The directory being walked.
    Dir,
    /// The entry being checked, or the directory when there is none.
    Entry,
}

fn bad_name(name: &[u8; 11]) -> bool {
    name[0] == b' '
        || name.iter().enumerate().any(|(i, &byte)| {
            (byte < 0x20 && !(i == 0 && byte == 0x05))
                || byte.is_ascii_lowercase()
                || BAD_NAME_BYTES.contains(&byte)
        })
}

/// Writes `/` and the name of `entry` to `out`: the long name as UTF-8 when
/// there is one, else the short name with its case bits, other bytes as
/// stored. Returns the length, or `None` when it does not fit.
fn put_name(out: &mut [u8], entry: &ShortEntry, long: Option<&[u16]>) -> Option<usize> {
    let mut len = 0;
    let mut push = |bytes: &[u8]| -> Option<()> {
        out.get_mut(len..len + bytes.len())?.copy_from_slice(bytes);
        len += bytes.len();
        Some(())
    };
    push(b"/")?;
    if let Some(units) = long {
        for ch in char::decode_utf16(units.iter().copied()) {
            let mut utf8 = [0u8; 4];
            push(
                ch.unwrap_or(char::REPLACEMENT_CHARACTER)
                    .encode_utf8(&mut utf8)
                    .as_bytes(),
            )?;
        }
        return Some(len);
    }
    let mut name = entry.name();
    short_name::from_disk(&mut name);
    let (base, ext) = name.split_at(8);
    let trim = |part: &[u8]| part.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    let lower = |byte: u8, bit: u8| {
        if entry.nt_case() & bit != 0 {
            byte.to_ascii_lowercase()
        } else {
            byte
        }
    };
    for &byte in &base[..trim(base)] {
        push(&[lower(byte, crate::dirent::NT_LOWER_BASE)])?;
    }
    let ext = &ext[..trim(ext)];
    if !ext.is_empty() {
        push(b".")?;
        for &byte in ext {
            push(&[lower(byte, crate::dirent::NT_LOWER_EXTENSION)])?;
        }
    }
    Some(len)
}

struct Checker<'a, D, F> {
    dev: &'a mut D,
    block: BlockBuf<[u8; MAX_BLOCK]>,
    fat: Fat,
    /// The path of the directory being walked, `dir_len` bytes, followed by
    /// `/` and the name of the entry being checked, `name_len` bytes.
    path: &'a mut [u8],
    dir_len: usize,
    name_len: usize,
    /// Directories entered whose names did not fit `path`.
    hidden: u32,
    bits: &'a mut [u8],
    /// The clusters this pass tracks, `lo..hi`.
    lo: u32,
    hi: u32,
    first_pass: bool,
    findings: u64,
    passes: u32,
    free: u32,
    sink: F,
    run: Run,
    names: lfn::Assembler,
    /// A run of lost clusters not yet reported.
    lost: (u32, u32),
}

io_transform! {

/// Checks an unmounted FAT12, FAT16 or FAT32 volume without changing it,
/// passes each [`Finding`] to `on_finding`, and returns the totals.
///
/// Checked are the boot sector, the FAT32 backup boot sector and FSInfo
/// sector with its free count, the reserved FAT entries and the
/// clean-shutdown bit, that mirrored FAT copies match the active one,
/// every directory entry reached from the root (names, dot entries,
/// labels, long-name runs and their checksums), and every chain: that it
/// starts and continues at data clusters, ends, has no cycle and no bad
/// cluster, matches the file's size, and shares no cluster with another
/// chain. Clusters in use that no chain reaches are reported as lost. Each
/// finding carries a [`Detail`] code, the one mount errors use. A dirty
/// volume and a wrong FSInfo free count are [`Severity::Notice`]s, the rest
/// [`Severity::Error`]s.
///
/// A FAT32 boot sector that does not mount is a finding, and the check
/// goes on from the backup boot sector; without a usable backup, `check`
/// fails with the error mounting gives.
///
/// Nothing is allocated. The first 1 KiB of `scratch` holds the path of
/// each finding, and the rest a bitmap of one bit per cluster. The tree is
/// walked once for each `(scratch.len() - 1024) * 8` clusters, so the
/// findings are the same with any size. The walk follows `..` entries
/// rather than a stack, which is why a directory whose `..` does not name
/// its parent is not entered. Paths are the long names, or the short
/// names with bytes above `0x7F` as stored, and a path that does not fit
/// ends at the last whole name that does.
///
/// Fails with [`ErrorKind::LimitExceeded`] when `scratch` is shorter than
/// 1536 bytes and with [`ErrorKind::Unsupported`] for device blocks above
/// 4096 bytes. Device errors end the check.
pub async fn check<D: BlockDevice, F: FnMut(&Finding<'_>)>(
    dev: &mut D,
    scratch: &mut [u8],
    on_finding: F,
) -> FsResult<CheckReport, D::Error> {
    if scratch.len() < PATH_LEN + MIN_BITMAP {
        return Err(ErrorKind::LimitExceeded.into());
    }
    let mut block = BlockBuf::<[u8; MAX_BLOCK]>::new(dev.block_size().get() as usize)
        .ok_or(ErrorKind::Unsupported)?;
    let mut sector = [0u8; BOOT_SECTOR_LEN];
    read_bytes(dev, &mut block, 0, &mut sector).await?;
    let (geo, backup) = match parse_boot(&sector) {
        Ok(geo) => (geo, None),
        Err(err) => match read_backup(dev, &mut block, &mut sector).await? {
            Some((geo, at)) => (geo, Some(at)),
            None => return Err(Detail::boot(err)),
        },
    };
    let device_len = dev.block_count().saturating_mul(block.block_size() as u64);
    if geo.data_end() > device_len {
        return Err(hadris_fs::Error::new(ErrorKind::Corrupt, "volume is larger than the device")
            .with_detail(Detail::BootSector.code()));
    }
    let (path, bits) = scratch.split_at_mut(PATH_LEN);
    let mut checker = Checker {
        dev,
        block,
        fat: Fat::new(geo),
        path,
        dir_len: 0,
        name_len: 0,
        hidden: 0,
        bits,
        lo: FIRST_DATA_CLUSTER,
        hi: FIRST_DATA_CLUSTER,
        first_pass: true,
        findings: 0,
        passes: 0,
        free: 0,
        sink: on_finding,
        run: Run::default(),
        names: lfn::Assembler::new(),
        lost: (0, 0),
    };
    checker.check(&sector, backup).await?;
    Ok(CheckReport::new(checker.findings, checker.passes))
}

/// The geometry of a FAT32 backup boot sector at sector 6, read into
/// `sector`, and its byte offset.
pub(super) async fn read_backup<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    sector: &mut [u8; BOOT_SECTOR_LEN],
) -> FsResult<Option<(Geometry, u64)>, D::Error> {
    let device_len = dev.block_count().saturating_mul(block.block_size() as u64);
    for shift in 9..=12u32 {
        let at = (BACKUP_BOOT_SECTOR as u64) << shift;
        if at + BOOT_SECTOR_LEN as u64 > device_len {
            break;
        }
        let mut copy = [0u8; BOOT_SECTOR_LEN];
        read_bytes(dev, block, at, &mut copy).await?;
        let Ok(geo) = parse_boot(&copy) else { continue };
        let ext: RawBpbExt32 =
            bytemuck::pod_read_unaligned(&copy[BPB_LEN..BPB_LEN + size_of::<RawBpbExt32>()]);
        if geo.kind() == FatKind::Fat32
            && geo.sector_size() == 1 << shift
            && u16::from_le_bytes(ext.boot_sector) == BACKUP_BOOT_SECTOR
        {
            *sector = copy;
            return Ok(Some((geo, at)));
        }
    }
    Ok(None)
}

impl<D: BlockDevice, F: FnMut(&Finding<'_>)> Checker<'_, D, F> {
    fn report(&mut self, finding: Finding<'static>, scope: Scope) {
        self.findings += 1;
        let end = match scope {
            Scope::Volume => None,
            Scope::Dir => Some(self.dir_len),
            Scope::Entry => Some(self.dir_len + self.name_len),
        };
        let mut finding: Finding<'_> = finding;
        if let Some(end) = end {
            finding = finding.with_path(if end == 0 { b"/" } else { &self.path[..end] });
        }
        (self.sink)(&finding);
    }

    /// Reports a finding that every pass would make, in the first pass only.
    fn once(&mut self, finding: Finding<'static>, scope: Scope) {
        if self.first_pass {
            self.report(finding, scope);
        }
    }

    async fn check(&mut self, sector: &[u8; BOOT_SECTOR_LEN], backup: Option<u64>) -> FsResult<(), D::Error> {
        let recorded = self.boot(sector, backup).await?;
        self.fat_copies().await?;
        let max = self.fat.geometry().max_cluster();
        let window = (self.bits.len() as u64 * 8).min(u32::MAX as u64) as u32;
        let mut lo = FIRST_DATA_CLUSTER;
        loop {
            self.lo = lo;
            self.hi = lo.saturating_add(window).min(max + 1);
            self.bits.fill(0);
            self.passes += 1;
            self.walk_tree().await?;
            self.scan_window().await?;
            self.first_pass = false;
            if self.hi > max {
                break;
            }
            lo = self.hi;
        }
        self.flush_lost();
        if let Some((at, recorded)) = recorded
            && recorded != UNKNOWN_FREE
            && recorded != self.free
        {
            let finding = found(Detail::FreeCount).with_severity(Severity::Notice).with_location(Location::Byte(at + FSINFO_FREE_COUNT));
            self.report(finding, Scope::Volume);
        }
        Ok(())
    }

    async fn read(&mut self, offset: u64, out: &mut [u8]) -> FsResult<(), D::Error> {
        read_bytes(self.dev, &mut self.block, offset, out).await
    }

    /// Checks the boot sector the geometry came from, read from `backup`
    /// when the main one is damaged, and returns the offset and free count
    /// of a valid FSInfo sector.
    async fn boot(&mut self, sector: &[u8; BOOT_SECTOR_LEN], backup: Option<u64>) -> FsResult<Option<(u64, u32)>, D::Error> {
        let at = Location::Byte(backup.unwrap_or(0));
        if backup.is_some() {
            let finding = said(Detail::BootSector, "boot sector is damaged; checked from the backup").with_location(Location::Byte(0));
            self.report(finding, Scope::Volume);
        }
        let bpb: RawBpb = bytemuck::pod_read_unaligned(&sector[..BPB_LEN]);
        let geo = *self.fat.geometry();
        let reserved = bpb.reserved_sector_count.get();
        let media = bpb.media_type;
        if !matches!(media, 0xF0 | 0xF8..=0xFF) {
            self.report(said(Detail::BootSector, "invalid media descriptor").with_location(at), Scope::Volume);
        }
        if reserved == 0 {
            self.report(said(Detail::BootSector, "reserved sector count is 0").with_location(at), Scope::Volume);
        }
        let total16 = u16::from_le_bytes(bpb.total_sectors_16);
        let total32 = u32::from_le_bytes(bpb.total_sectors_32);
        if total16 != 0 && (total32 != 0 || geo.kind() == FatKind::Fat32) {
            self.report(said(Detail::BootSector, "conflicting total sector counts").with_location(at), Scope::Volume);
        }
        let mut recorded = None;
        if geo.kind() == FatKind::Fat32 {
            let ext: RawBpbExt32 =
                bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt32>()]);
            let copy_at = u16::from_le_bytes(ext.boot_sector);
            if backup.is_none() && copy_at != 0 && copy_at != 0xFFFF {
                if copy_at >= reserved {
                    let message = "backup boot sector is outside the reserved sectors";
                    self.report(said(Detail::BootSector, message).with_location(at), Scope::Volume);
                } else {
                    let offset = copy_at as u64 * geo.sector_size() as u64;
                    let mut copy = [0u8; BOOT_SECTOR_LEN];
                    self.read(offset, &mut copy).await?;
                    if copy != *sector {
                        self.report(found(Detail::BackupBootSector).with_location(Location::Byte(offset)), Scope::Volume);
                    }
                }
            }
            let info = ext.fs_info_sector.get();
            if info != 0 && info != 0xFFFF {
                if info >= reserved {
                    let message = "FSInfo sector is outside the reserved sectors";
                    self.report(said(Detail::BootSector, message).with_location(at), Scope::Volume);
                } else {
                    let offset = info as u64 * geo.sector_size() as u64;
                    let mut raw = [0u8; BOOT_SECTOR_LEN];
                    self.read(offset, &mut raw).await?;
                    let info: RawFsInfo = bytemuck::pod_read_unaligned(&raw);
                    if check_fs_info(&info).is_ok() {
                        recorded = Some((offset, info.free_count.get()));
                    } else {
                        self.report(found(Detail::FsInfo).with_location(Location::Byte(offset)), Scope::Volume);
                    }
                }
            }
        }
        let active = geo.active_fat();
        let first = self.entry(active, 0).await?;
        let second = self.entry(active, 1).await?;
        let fat = Location::Byte(geo.fat_copy(active));
        if !geo.kind().reserved_entries_valid(media, first, second) {
            self.report(found(Detail::ReservedEntries).with_location(fat), Scope::Volume);
        } else if second & clean_bit(geo.kind()) == 0 && geo.kind() != FatKind::Fat12 {
            self.report(found(Detail::Dirty).with_severity(Severity::Notice).with_location(fat), Scope::Volume);
        }
        Ok(recorded)
    }

    /// The stored entry of `cluster` in FAT copy `copy`.
    async fn entry(&mut self, copy: u8, cluster: u32) -> FsResult<u32, D::Error> {
        let kind = self.fat.geometry().kind();
        let at = self.fat.geometry().fat_copy(copy) + kind.entry_offset(cluster as u64);
        let mut bytes = [0u8; 4];
        self.read(at, &mut bytes[..kind.entry_len()]).await?;
        Ok(kind.decode(cluster as u64, &bytes))
    }

    /// Compares every mirrored FAT copy with the active one.
    async fn fat_copies(&mut self) -> FsResult<(), D::Error> {
        let geo = *self.fat.geometry();
        if !geo.mirrored() || geo.fat_count() < 2 {
            return Ok(());
        }
        let kind = geo.kind();
        let bits = kind.entry_bits();
        let len = kind.entry_offset(geo.max_cluster() as u64) + kind.entry_len() as u64;
        let active = geo.active_fat();
        for copy in (0..geo.fat_count()).filter(|&copy| copy != active) {
            let (mut first, mut differ, mut next) = (0, false, 0u64);
            let mut at = 0u64;
            while at < len && !differ {
                let n = (len - at).min(CHUNK as u64) as usize;
                let mut a = [0u8; CHUNK];
                let mut b = [0u8; CHUNK];
                self.read(geo.fat_copy(active) + at, &mut a[..n]).await?;
                self.read(geo.fat_copy(copy) + at, &mut b[..n]).await?;
                if a[..n] != b[..n] {
                    let from = (at * 8 / bits).max(next);
                    let to = ((at + n as u64) * 8).div_ceil(bits).min(geo.max_cluster() as u64 + 1);
                    for cluster in from..to {
                        let ours = self.entry(active, cluster as u32).await? & kind.mask();
                        let theirs = self.entry(copy, cluster as u32).await? & kind.mask();
                        if ours != theirs {
                            first = cluster as u32;
                            differ = true;
                            break;
                        }
                    }
                    next = to;
                }
                at += n as u64;
            }
            if differ {
                let finding = found(Detail::FatCopiesDiffer).with_location(Location::Cluster(first as u64));
                self.report(finding, Scope::Volume);
            }
        }
        Ok(())
    }

    async fn link(&mut self, cluster: u32) -> FsResult<Link, D::Error> {
        let stored = get(self.dev, &mut self.block, &self.fat, cluster).await?;
        Ok(match self.fat.geometry().kind().next(stored, self.fat.geometry().max_cluster()) {
            Ok(Some(next)) => Link::Next(next),
            Ok(None) => Link::End(End::Eoc),
            Err(ChainError::Bad) => Link::End(End::Bad(cluster)),
            Err(ChainError::OutOfBounds(_)) => Link::End(End::Broken(cluster)),
        })
    }

    /// Follows a link known to be valid.
    async fn step(&mut self, cluster: u32) -> FsResult<u32, D::Error> {
        match self.link(cluster).await? {
            Link::Next(next) => Ok(next),
            Link::End(_) => Err(ErrorKind::Corrupt.into()),
        }
    }

    /// Measures the chain at `first`, a data cluster, finding a cycle with
    /// Brent's algorithm.
    async fn analyze(&mut self, first: u32) -> FsResult<Chain, D::Error> {
        let mut tortoise = first;
        let mut hare = match self.link(first).await? {
            Link::Next(next) => next,
            Link::End(end) => return Ok(Chain { len: 1, end }),
        };
        let (mut steps, mut power, mut lambda) = (1u32, 1u32, 1u32);
        while tortoise != hare {
            if power == lambda {
                tortoise = hare;
                power = power.saturating_mul(2);
                lambda = 0;
            }
            hare = match self.link(hare).await? {
                Link::Next(next) => next,
                Link::End(end) => return Ok(Chain { len: steps + 1, end }),
            };
            steps += 1;
            lambda += 1;
        }
        let (mut tortoise, mut hare, mut last) = (first, first, first);
        for _ in 0..lambda {
            last = hare;
            hare = self.step(hare).await?;
        }
        let mut mu = 0;
        while tortoise != hare {
            tortoise = self.step(tortoise).await?;
            last = hare;
            hare = self.step(hare).await?;
            mu += 1;
        }
        Ok(Chain { len: mu + lambda, end: End::Cycle(last) })
    }

    fn chain_end(&mut self, chain: &Chain) {
        let (detail, cluster) = match chain.end {
            End::Eoc => return,
            End::Bad(cluster) => (Detail::BadCluster, cluster),
            End::Broken(cluster) => (Detail::BrokenChain, cluster),
            End::Cycle(cluster) => (Detail::CyclicChain, cluster),
        };
        self.once(found(detail).with_location(Location::Cluster(cluster as u64)), Scope::Entry);
    }

    /// Claims the first `len` clusters of the chain at `first` for the
    /// entry being checked.
    async fn mark(&mut self, first: u32, len: u32) -> FsResult<(), D::Error> {
        let mut cluster = first;
        for index in 0..len {
            if (self.lo..self.hi).contains(&cluster) {
                let bit = (cluster - self.lo) as usize;
                let (byte, mask) = (bit / 8, 1u8 << (bit % 8));
                if self.bits[byte] & mask != 0 {
                    self.report(found(Detail::CrossLink).with_location(Location::Cluster(cluster as u64)), Scope::Entry);
                } else {
                    self.bits[byte] |= mask;
                }
            }
            if index + 1 < len {
                cluster = self.step(cluster).await?;
            }
        }
        Ok(())
    }

    fn slots(&self, len: u32) -> u32 {
        let per_cluster = self.fat.geometry().cluster_size() / ENTRY_SIZE as u32;
        len.saturating_mul(per_cluster).min(MAX_DIR_ENTRIES)
    }

    fn root_cluster(&self) -> Option<u32> {
        match self.fat.geometry().root() {
            RootLocation::Cluster(cluster) => Some(cluster),
            RootLocation::Fixed { .. } => None,
        }
    }

    async fn root(&mut self) -> FsResult<Dir, D::Error> {
        let start = self.fat.root();
        Ok(match start {
            DirStart::Fixed { slots, .. } => Dir { start, cluster: 0, slots, root: true },
            DirStart::Chain(cluster) => {
                let chain = self.analyze(cluster).await?;
                self.chain_end(&chain);
                self.mark(cluster, chain.len).await?;
                Dir { start, cluster, slots: self.slots(chain.len), root: true }
            }
        })
    }

    /// The directory whose first cluster is `cluster`, or the root.
    async fn dir_at(&mut self, cluster: u32) -> FsResult<Dir, D::Error> {
        if cluster == 0 || self.root_cluster() == Some(cluster) {
            let start = self.fat.root();
            return Ok(match start {
                DirStart::Fixed { slots, .. } => Dir { start, cluster: 0, slots, root: true },
                DirStart::Chain(cluster) => {
                    let len = self.analyze(cluster).await?.len;
                    Dir { start, cluster, slots: self.slots(len), root: true }
                }
            });
        }
        let len = self.analyze(cluster).await?.len;
        Ok(Dir { start: DirStart::Chain(cluster), cluster, slots: self.slots(len), root: false })
    }

    async fn slot(&mut self, dir: &Dir, walk: &mut DirWalk, slot: u32) -> FsResult<Option<(u64, Slot)>, D::Error> {
        if slot >= dir.slots {
            return Ok(None);
        }
        let offset = slot_offset(self.dev, &mut self.block, &self.fat, walk, slot).await?.ok_or(ErrorKind::Corrupt)?;
        Ok(Some((offset, read_slot(self.dev, &mut self.block, offset).await?)))
    }

    /// The first slot of `dir` before `before` holding a subdirectory entry
    /// that names `cluster`.
    async fn first_naming(&mut self, dir: &Dir, cluster: u32, before: u32) -> FsResult<Option<u32>, D::Error> {
        let kind = self.fat.geometry().kind();
        let mut walk = DirWalk::new(dir.start);
        for slot in 0..before.min(dir.slots) {
            match self.slot(dir, &mut walk, slot).await? {
                Some((_, Slot::End)) | None => break,
                Some((_, Slot::Short(entry)))
                    if entry.is_dir()
                        && !entry.is_label()
                        && entry.name() != DOT
                        && entry.name() != DOT_DOT
                        && entry.first_cluster(kind) == cluster =>
                {
                    return Ok(Some(slot));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Whether the `..` value `up` names `parent`.
    fn names_parent(&self, parent: &Dir, up: u32) -> bool {
        if parent.root {
            up == 0 || Some(up) == self.root_cluster()
        } else {
            up == parent.cluster
        }
    }

    /// Checks the dot entries of the directory at `cluster`, named by the
    /// entry at `slot` of `parent`, and returns whether to enter it: its
    /// `..` must name `parent`, and no earlier entry of `parent` may name
    /// it.
    async fn may_enter(&mut self, parent: &Dir, slot: u32, cluster: u32) -> FsResult<bool, D::Error> {
        let kind = self.fat.geometry().kind();
        let at = self.fat.cluster_at(cluster)?;
        let dot = match read_slot(self.dev, &mut self.block, at).await? {
            Slot::Short(dot) => dot.name() == DOT && dot.is_dir() && dot.first_cluster(kind) == cluster,
            _ => false,
        };
        let dot_dot = match read_slot(self.dev, &mut self.block, at + ENTRY_SIZE as u64).await? {
            Slot::Short(up) => up.name() == DOT_DOT && up.is_dir() && self.names_parent(parent, up.first_cluster(kind)),
            _ => false,
        };
        if !dot || !dot_dot {
            self.once(found(Detail::DotEntries).with_location(Location::Byte(at)), Scope::Entry);
        }
        Ok(dot_dot && self.first_naming(parent, cluster, slot).await?.is_none())
    }

    /// Leaves `dir` for its parent and returns the parent and the slot
    /// after the entry naming `dir`.
    async fn ascend(&mut self, dir: &Dir) -> FsResult<(Dir, u32), D::Error> {
        let kind = self.fat.geometry().kind();
        let at = self.fat.cluster_at(dir.cluster)? + ENTRY_SIZE as u64;
        let Slot::Short(up) = read_slot(self.dev, &mut self.block, at).await? else {
            return Err(ErrorKind::Corrupt.into());
        };
        let parent = self.dir_at(up.first_cluster(kind)).await?;
        let slot = self
            .first_naming(&parent, dir.cluster, u32::MAX)
            .await?
            .ok_or(ErrorKind::Corrupt)?;
        if self.hidden > 0 {
            self.hidden -= 1;
        } else {
            self.dir_len = self.path[..self.dir_len].iter().rposition(|&b| b == b'/').unwrap_or(0);
        }
        self.name_len = 0;
        Ok((parent, slot + 1))
    }

    /// Makes the directory entry being checked the directory walked.
    fn descend(&mut self) {
        if self.hidden > 0 || self.name_len == 0 {
            self.hidden += 1;
        } else {
            self.dir_len += self.name_len;
        }
        self.name_len = 0;
    }

    fn orphan(&mut self, entry: u64) {
        if !self.run.orphan {
            self.once(found(Detail::OrphanLfn).with_location(Location::Byte(entry)), Scope::Dir);
            self.run.orphan = true;
        }
    }

    /// Abandons a pending long-name run as an orphan.
    fn abandon(&mut self) {
        if self.run.pending {
            self.run.pending = false;
            self.run.orphan = false;
            self.orphan(self.run.start);
        }
    }

    /// Takes one long-name fragment holding `units` code units.
    fn fragment(&mut self, offset: u64, sequence: u8, checksum: u8, units: usize) {
        if sequence & LFN_LAST_ENTRY != 0 {
            self.abandon();
            let count = sequence & LFN_SEQUENCE_MASK;
            if count == 0 || count as usize > lfn::MAX_ENTRIES {
                self.run.orphan = false;
                self.orphan(offset);
            } else {
                self.run = Run { pending: true, start: offset, expect: count - 1, checksum, units, orphan: false };
            }
        } else if self.run.pending
            && self.run.expect != 0
            && sequence == self.run.expect
            && checksum == self.run.checksum
            && self.run.units + units <= lfn::MAX_UNITS
        {
            self.run.expect -= 1;
            self.run.units += units;
        } else if self.run.pending {
            self.abandon();
        } else {
            self.orphan(offset);
        }
    }

    /// Ends a long-name run at the short entry being checked.
    fn close_run(&mut self, name: &[u8; 11]) {
        if self.run.pending {
            let at = Location::Byte(self.run.start);
            if self.run.expect != 0 {
                self.once(found(Detail::OrphanLfn).with_location(at), Scope::Dir);
            } else if self.run.checksum != lfn::checksum(name) {
                self.once(found(Detail::LfnChecksum).with_location(at), Scope::Entry);
            }
        }
        self.run = Run::default();
    }

    /// Ends a long-name run at a free slot or the end of a directory.
    fn break_run(&mut self) {
        self.abandon();
        self.run = Run::default();
        self.names.reset();
    }

    /// Stages the name of the short entry `entry` after the directory's
    /// path.
    fn stage_name(&mut self, entry: &ShortEntry) {
        let long = self.names.finish(entry.lfn_checksum());
        self.name_len = match self.hidden {
            0 => put_name(&mut self.path[self.dir_len..], entry, long).unwrap_or(0),
            _ => 0,
        };
    }

    /// Checks the chain of the file or directory entry `short` and claims
    /// its clusters. Returns the chain's length when it starts at a data
    /// cluster.
    async fn entry_chain(&mut self, entry: u64, short: &ShortEntry) -> FsResult<Option<u32>, D::Error> {
        let kind = self.fat.geometry().kind();
        let first = short.first_cluster(kind);
        let is_dir = short.is_dir();
        if first == 0 {
            if is_dir {
                self.once(found(Detail::InvalidCluster).with_location(Location::Cluster(0)), Scope::Entry);
            } else if short.size() > 0 {
                let finding = said(Detail::SizeMismatch, "file has a size but no clusters");
                self.once(finding.with_location(Location::Byte(entry)), Scope::Entry);
            }
            return Ok(None);
        }
        if !self.fat.is_cluster(first) || Some(first) == self.root_cluster() {
            self.once(found(Detail::InvalidCluster).with_location(Location::Cluster(first as u64)), Scope::Entry);
            return Ok(None);
        }
        let chain = self.analyze(first).await?;
        self.chain_end(&chain);
        self.mark(first, chain.len).await?;
        if !is_dir && matches!(chain.end, End::Eoc) {
            let needed = short.size().div_ceil(self.fat.geometry().cluster_size());
            let message = if chain.len > needed {
                "chain is longer than the file size needs"
            } else if chain.len < needed {
                "chain is shorter than the file size needs"
            } else {
                return Ok(Some(chain.len));
            };
            let finding = said(Detail::SizeMismatch, message);
            self.once(finding.with_location(Location::Byte(entry)), Scope::Entry);
        }
        Ok(Some(chain.len))
    }

    /// Walks the tree from the root, depth first, through `..` entries.
    async fn walk_tree(&mut self) -> FsResult<(), D::Error> {
        let kind = self.fat.geometry().kind();
        (self.dir_len, self.name_len, self.hidden) = (0, 0, 0);
        let mut dir = self.root().await?;
        let mut walk = DirWalk::new(dir.start);
        let mut slot = 0;
        let mut label = false;
        self.run = Run::default();
        self.names.reset();
        loop {
            let found_slot = self.slot(&dir, &mut walk, slot).await?;
            let (offset, entry) = match found_slot {
                None | Some((_, Slot::End)) => {
                    self.break_run();
                    if dir.root {
                        return Ok(());
                    }
                    (dir, slot) = self.ascend(&dir).await?;
                    walk = DirWalk::new(dir.start);
                    continue;
                }
                Some((_, Slot::Free)) => {
                    self.break_run();
                    slot += 1;
                    continue;
                }
                Some((offset, Slot::Long(part))) => {
                    let units = part.units().1;
                    self.fragment(offset, part.sequence(), part.checksum(), units);
                    self.names.push(&part);
                    slot += 1;
                    continue;
                }
                Some((offset, Slot::Short(entry))) => (offset, entry),
            };
            self.name_len = 0;
            slot += 1;
            if entry.is_label() {
                self.close_run(&entry.name());
                self.names.reset();
                if !dir.root || label {
                    self.once(found(Detail::Label).with_location(Location::Byte(offset)), Scope::Dir);
                }
                label |= dir.root;
                continue;
            }
            if entry.name() == DOT || entry.name() == DOT_DOT {
                self.close_run(&entry.name());
                self.names.reset();
                self.once(found(Detail::DotEntries).with_location(Location::Byte(offset)), Scope::Dir);
                continue;
            }
            self.stage_name(&entry);
            self.close_run(&entry.name());
            if bad_name(&entry.name()) {
                self.once(found(Detail::BadName).with_location(Location::Byte(offset)), Scope::Entry);
            }
            if !entry.is_dir() {
                self.entry_chain(offset, &entry).await?;
                continue;
            }
            if entry.size() != 0 {
                self.once(found(Detail::DirectorySize).with_location(Location::Byte(offset)), Scope::Entry);
            }
            let cluster = entry.first_cluster(kind);
            if self.entry_chain(offset, &entry).await?.is_some()
                && self.may_enter(&dir, slot - 1, cluster).await?
            {
                dir = self.dir_at(cluster).await?;
                self.descend();
                walk = DirWalk::new(dir.start);
                slot = 2;
            }
        }
    }

    /// Counts the free FAT entries of this pass's clusters and reports the
    /// allocated ones no chain claimed.
    async fn scan_window(&mut self) -> FsResult<(), D::Error> {
        let kind = self.fat.geometry().kind();
        for cluster in self.lo..self.hi {
            let value = get(self.dev, &mut self.block, &self.fat, cluster).await? & kind.mask();
            let bit = (cluster - self.lo) as usize;
            let claimed = self.bits[bit / 8] & (1 << (bit % 8)) != 0;
            if value == 0 {
                self.free += 1;
            } else if !kind.is_bad(value) && !claimed {
                if self.lost.1 > 0 && self.lost.0 + self.lost.1 == cluster {
                    self.lost.1 += 1;
                } else {
                    self.flush_lost();
                    self.lost = (cluster, 1);
                }
                continue;
            }
            self.flush_lost();
        }
        Ok(())
    }

    fn flush_lost(&mut self) {
        if self.lost.1 > 0 {
            let first = self.lost.0;
            self.lost = (0, 0);
            self.report(found(Detail::LostClusters).with_location(Location::Cluster(first as u64)), Scope::Volume);
        }
    }
}

}

/// A finding of `detail`, described by it.
const fn found(detail: Detail) -> Finding<'static> {
    Finding::new(detail.description(), detail.code())
}

/// A finding of `detail` with its own message.
const fn said(detail: Detail, message: &'static str) -> Finding<'static> {
    Finding::new(message, detail.code())
}

/// The FAT entry 1 bit that is set while the volume is cleanly unmounted.
const fn clean_bit(kind: FatKind) -> u32 {
    match kind {
        FatKind::Fat12 => 0,
        FatKind::Fat16 => 0x8000,
        FatKind::Fat32 => 0x0800_0000,
    }
}
