use hadris_fs::{Clock, ErrorKind, FsResult, NodeTable};

use super::super::block_io::read_bytes;
use super::super::rawio;
use super::super::storage::BlockDevice;
use hadris_fat_raw::{
    self as raw, BOOT_SECTOR_LEN, ChainError, FIRST_DATA_CLUSTER, FatKind, RawBpb, RawBpbExt32,
    RawFsInfo, RootLocation, ShortEntry, Slot, lfn,
};

use super::{ENTRY_SIZE, FatFs, UNKNOWN_FREE};
use crate::code_page::CodePage;
use crate::findings::{CheckReport, Finding};
use hadris_fat_raw::io::{DirStart, DirWalk};

/// The bitmap [`check`] uses: 32768 clusters a pass.
const DEFAULT_BITMAP: usize = 4096;
/// Bytes of each FAT copy compared at once.
const CHUNK: usize = 512;
const DOT: [u8; 11] = *b".          ";
const DOT_DOT: [u8; 11] = *b"..         ";
/// Bytes a short name may not hold, besides controls and lowercase.
const BAD_NAME_BYTES: &[u8] = b"\"*+,./:;<=>?[\\]|\x7F";
/// The entry that stands for the root directory.
const ROOT_ENTRY: u64 = 0;

/// Where a chain ends.
#[derive(Clone, Copy)]
enum End {
    Eoc,
    Bad(u32),
    Broken { cluster: u32, next: u32 },
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

fn bad_name(name: &[u8; 11]) -> bool {
    name[0] == b' '
        || name.iter().enumerate().any(|(i, &byte)| {
            (byte < 0x20 && !(i == 0 && byte == 0x05))
                || byte.is_ascii_lowercase()
                || BAD_NAME_BYTES.contains(&byte)
        })
}

const BPB_LEN: usize = size_of::<RawBpb>();

struct Checker<'a, D, T: NodeTable, C: Clock, P: CodePage, F> {
    fs: &'a mut FatFs<D, T, C, P>,
    bits: &'a mut [u8],
    /// The clusters this pass tracks, `lo..hi`.
    lo: u32,
    hi: u32,
    first_pass: bool,
    report: CheckReport,
    sink: F,
    run: Run,
    /// A run of lost clusters not yet reported.
    lost: (u32, u32),
}

io_transform! {

/// Checks the volume without changing it, and returns how many problems of
/// each kind it found. See [`check_with`] for what is checked; this uses a
/// 4 KiB bitmap and discards the individual [`Finding`]s.
pub async fn check<D: BlockDevice, T: NodeTable, C: Clock, P: CodePage>(
    fs: &mut FatFs<D, T, C, P>,
) -> FsResult<CheckReport, D::Error> {
    let mut bitmap = [0u8; DEFAULT_BITMAP];
    check_with(fs, &mut bitmap, |_| {}).await
}

/// Checks the volume without changing it, passing each [`Finding`] to
/// `on_finding`, and returns the totals.
///
/// Checked are the boot sector, the FAT32 backup boot sector and FSInfo
/// sector with its free count, the reserved FAT entries, that mirrored FAT
/// copies match the active one, every directory entry reached from the
/// root (names, dot entries, labels, long-name runs and their checksums),
/// and every chain: that it starts and continues at data clusters, ends,
/// has no cycle and no bad cluster, matches the file's size, and shares no
/// cluster with another chain. Clusters in use that no chain reaches are
/// reported as lost.
///
/// `bitmap` holds one bit per cluster. The directory tree is walked once
/// for each `bitmap.len() * 8` clusters, so a bitmap of
/// `stats().total_blocks().div_ceil(8)` bytes checks in one pass; the
/// findings are the same with any size. Memory use is otherwise fixed:
/// the tree is walked through `..` entries without a stack, which is why a
/// directory whose `..` does not name its parent is not entered.
///
/// The device is read as it is: the sizes and FSInfo free count `FatFs`
/// has not yet written (see `sync`) show up as findings. The node table is
/// not used. An empty `bitmap` fails with [`ErrorKind::InvalidInput`];
/// device errors end the check.
pub async fn check_with<D: BlockDevice, T: NodeTable, C: Clock, P: CodePage, F: FnMut(Finding)>(
    fs: &mut FatFs<D, T, C, P>,
    bitmap: &mut [u8],
    on_finding: F,
) -> FsResult<CheckReport, D::Error> {
    if bitmap.is_empty() {
        return Err(ErrorKind::InvalidInput.into());
    }
    let mut checker = Checker {
        fs,
        bits: bitmap,
        lo: FIRST_DATA_CLUSTER,
        hi: FIRST_DATA_CLUSTER,
        first_pass: true,
        report: CheckReport::new(),
        sink: on_finding,
        run: Run::default(),
        lost: (0, 0),
    };
    checker.check().await?;
    Ok(checker.report)
}

impl<D: BlockDevice, T: NodeTable, C: Clock, P: CodePage, F: FnMut(Finding)> Checker<'_, D, T, C, P, F> {
    fn report(&mut self, finding: Finding) {
        self.report.record(&finding);
        (self.sink)(finding);
    }

    /// Reports a finding that every pass would make, in the first pass only.
    fn once(&mut self, finding: Finding) {
        if self.first_pass {
            self.report(finding);
        }
    }

    async fn check(&mut self) -> FsResult<(), D::Error> {
        let recorded = self.boot().await?;
        self.fat_copies().await?;
        let max = self.fs.fat.geometry().max_cluster();
        let window = (self.bits.len() as u64 * 8).min(u32::MAX as u64) as u32;
        let mut lo = FIRST_DATA_CLUSTER;
        loop {
            self.lo = lo;
            self.hi = lo.saturating_add(window).min(max + 1);
            self.bits.fill(0);
            self.report.passes += 1;
            self.walk_tree().await?;
            self.scan_window().await?;
            self.first_pass = false;
            if self.hi > max {
                break;
            }
            lo = self.hi;
        }
        self.flush_lost();
        if let Some(recorded) = recorded
            && recorded != UNKNOWN_FREE
            && recorded != self.report.free
        {
            self.report(Finding::FreeCount { recorded, actual: self.report.free });
        }
        Ok(())
    }

    async fn read(&mut self, offset: u64, out: &mut [u8]) -> FsResult<(), D::Error> {
        read_bytes(&mut self.fs.dev, &mut self.fs.block, offset, out).await
    }

    /// Checks the boot sector and returns the FSInfo free count.
    async fn boot(&mut self) -> FsResult<Option<u32>, D::Error> {
        let mut sector = [0u8; BOOT_SECTOR_LEN];
        self.read(0, &mut sector).await?;
        let bpb: RawBpb = bytemuck::pod_read_unaligned(&sector[..BPB_LEN]);
        let geo = *self.fs.fat.geometry();
        let reserved = bpb.reserved_sector_count.get();
        let media = bpb.media_type;
        if !matches!(media, 0xF0 | 0xF8..=0xFF) {
            self.report(Finding::BootSector("media descriptor"));
        }
        if reserved == 0 {
            self.report(Finding::BootSector("reserved sector count"));
        }
        let total16 = u16::from_le_bytes(bpb.total_sectors_16);
        let total32 = u32::from_le_bytes(bpb.total_sectors_32);
        if total16 != 0 && (total32 != 0 || geo.kind() == FatKind::Fat32) {
            self.report(Finding::BootSector("total sector count"));
        }
        let mut recorded = None;
        if geo.kind() == FatKind::Fat32 {
            let ext: RawBpbExt32 =
                bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt32>()]);
            let backup = u16::from_le_bytes(ext.boot_sector);
            if backup != 0 && backup != 0xFFFF {
                if backup >= reserved {
                    self.report(Finding::BootSector("backup boot sector"));
                } else {
                    let mut copy = [0u8; BOOT_SECTOR_LEN];
                    self.read(backup as u64 * geo.sector_size() as u64, &mut copy).await?;
                    if copy != sector {
                        self.report(Finding::BackupBootSector);
                    }
                }
            }
            let info = ext.fs_info_sector.get();
            if info != 0 && info != 0xFFFF {
                if info >= reserved {
                    self.report(Finding::BootSector("FSInfo sector"));
                } else {
                    let mut raw = [0u8; BOOT_SECTOR_LEN];
                    self.read(info as u64 * geo.sector_size() as u64, &mut raw).await?;
                    let info: RawFsInfo = bytemuck::pod_read_unaligned(&raw);
                    if raw::check_fs_info(&info).is_ok() {
                        recorded = Some(info.free_count.get());
                    } else {
                        self.report(Finding::FsInfo);
                    }
                }
            }
        }
        let first = self.entry(self.fs.fat.geometry().active_fat(), 0).await?;
        let second = self.entry(self.fs.fat.geometry().active_fat(), 1).await?;
        if !geo.kind().reserved_entries_valid(media, first, second) {
            self.report(Finding::ReservedEntries);
        }
        Ok(recorded)
    }

    /// The stored entry of `cluster` in FAT copy `copy`.
    async fn entry(&mut self, copy: u8, cluster: u32) -> FsResult<u32, D::Error> {
        let geo = *self.fs.fat.geometry();
        let at = geo.fat_start() + copy as u64 * geo.fat_size() + geo.kind().entry_offset(cluster as u64);
        let mut bytes = [0u8; 4];
        self.read(at, &mut bytes[..geo.kind().entry_len()]).await?;
        Ok(geo.kind().decode(cluster as u64, &bytes))
    }

    /// Compares every mirrored FAT copy with the active one.
    async fn fat_copies(&mut self) -> FsResult<(), D::Error> {
        let geo = *self.fs.fat.geometry();
        if !self.fs.fat.geometry().mirrored() || geo.fat_count() < 2 {
            return Ok(());
        }
        let kind = geo.kind();
        let bits = kind.entry_bits();
        let len = kind.entry_offset(geo.max_cluster() as u64) + kind.entry_len() as u64;
        let active = self.fs.fat.geometry().active_fat();
        for copy in (0..geo.fat_count()).filter(|&copy| copy != active) {
            let (mut first, mut entries, mut next) = (0, 0u32, 0u64);
            let mut at = 0u64;
            while at < len {
                let n = (len - at).min(CHUNK as u64) as usize;
                let mut a = [0u8; CHUNK];
                let mut b = [0u8; CHUNK];
                self.read(geo.fat_start() + active as u64 * geo.fat_size() + at, &mut a[..n]).await?;
                self.read(geo.fat_start() + copy as u64 * geo.fat_size() + at, &mut b[..n]).await?;
                if a[..n] != b[..n] {
                    let from = (at * 8 / bits).max(next);
                    let to = ((at + n as u64) * 8).div_ceil(bits).min(geo.max_cluster() as u64 + 1);
                    for cluster in from..to {
                        let mask = kind.mask();
                        let ours = self.entry(active, cluster as u32).await? & mask;
                        let theirs = self.entry(copy, cluster as u32).await? & mask;
                        if ours != theirs {
                            if entries == 0 {
                                first = cluster as u32;
                            }
                            entries += 1;
                        }
                    }
                    next = to;
                }
                at += n as u64;
            }
            if entries > 0 {
                self.report(Finding::FatCopy { copy, first, entries });
            }
        }
        Ok(())
    }

    async fn link(&mut self, cluster: u32) -> FsResult<Link, D::Error> {
        let stored = rawio::get(&mut self.fs.dev, &mut self.fs.block, &self.fs.fat, cluster).await?;
        Ok(match self.fs.fat.geometry().kind().next(stored, self.fs.fat.geometry().max_cluster()) {
            Ok(Some(next)) => Link::Next(next),
            Ok(None) => Link::End(End::Eoc),
            Err(ChainError::Bad) => Link::End(End::Bad(cluster)),
            Err(ChainError::OutOfBounds(next)) => Link::End(End::Broken { cluster, next }),
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

    fn chain_end(&mut self, entry: u64, chain: &Chain) {
        match chain.end {
            End::Eoc => {}
            End::Bad(cluster) => self.once(Finding::BadCluster { entry, cluster }),
            End::Broken { cluster, next } => self.once(Finding::BrokenChain { entry, cluster, next }),
            End::Cycle(cluster) => self.once(Finding::CyclicChain { entry, cluster }),
        }
    }

    /// Claims the first `len` clusters of the chain at `first` for `entry`.
    async fn mark(&mut self, first: u32, len: u32, entry: u64) -> FsResult<(), D::Error> {
        let mut cluster = first;
        for index in 0..len {
            if (self.lo..self.hi).contains(&cluster) {
                let bit = (cluster - self.lo) as usize;
                let (byte, mask) = (bit / 8, 1u8 << (bit % 8));
                if self.bits[byte] & mask != 0 {
                    self.report(Finding::CrossLinked { entry, cluster });
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
        let per_cluster = self.fs.fat.geometry().cluster_size() / ENTRY_SIZE as u32;
        len.saturating_mul(per_cluster).min(raw::MAX_DIR_ENTRIES)
    }

    fn root_cluster(&self) -> Option<u32> {
        match self.fs.fat.geometry().root() {
            RootLocation::Cluster(cluster) => Some(cluster),
            RootLocation::Fixed { .. } => None,
        }
    }

    async fn root(&mut self) -> FsResult<Dir, D::Error> {
        let start = self.fs.fat.root();
        Ok(match start {
            DirStart::Fixed { slots, .. } => Dir { start, cluster: 0, slots, root: true },
            DirStart::Chain(cluster) => {
                let chain = self.analyze(cluster).await?;
                self.chain_end(ROOT_ENTRY, &chain);
                self.mark(cluster, chain.len, ROOT_ENTRY).await?;
                Dir { start, cluster, slots: self.slots(chain.len), root: true }
            }
        })
    }

    /// The directory whose first cluster is `cluster`, or the root.
    async fn dir_at(&mut self, cluster: u32) -> FsResult<Dir, D::Error> {
        if cluster == 0 || self.root_cluster() == Some(cluster) {
            let start = self.fs.fat.root();
            let slots = match start {
                DirStart::Fixed { slots, .. } => slots,
                DirStart::Chain(cluster) => {
                    let len = self.analyze(cluster).await?.len;
                    self.slots(len)
                }
            };
            let cluster = match start {
                DirStart::Fixed { .. } => 0,
                DirStart::Chain(cluster) => cluster,
            };
            return Ok(Dir { start, cluster, slots, root: true });
        }
        let len = self.analyze(cluster).await?.len;
        Ok(Dir { start: DirStart::Chain(cluster), cluster, slots: self.slots(len), root: false })
    }

    async fn slot(&mut self, dir: &Dir, walk: &mut DirWalk, slot: u32) -> FsResult<Option<(u64, Slot)>, D::Error> {
        if slot >= dir.slots {
            return Ok(None);
        }
        let offset = rawio::slot_offset(&mut self.fs.dev, &mut self.fs.block, &self.fs.fat, walk, slot).await?.ok_or(ErrorKind::Corrupt)?;
        Ok(Some((offset, rawio::read_slot(&mut self.fs.dev, &mut self.fs.block, offset).await?)))
    }

    /// The first slot of `dir` before `before` holding a subdirectory entry
    /// that names `cluster`.
    async fn first_naming(&mut self, dir: &Dir, cluster: u32, before: u32) -> FsResult<Option<u32>, D::Error> {
        let kind = self.fs.fat.geometry().kind();
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
    async fn may_enter(&mut self, parent: &Dir, slot: u32, entry: u64, cluster: u32) -> FsResult<bool, D::Error> {
        let kind = self.fs.fat.geometry().kind();
        let at = self.fs.cluster_at(cluster)?;
        let dot = match rawio::read_slot(&mut self.fs.dev, &mut self.fs.block, at).await? {
            Slot::Short(dot) => dot.name() == DOT && dot.is_dir() && dot.first_cluster(kind) == cluster,
            _ => false,
        };
        let dot_dot = match rawio::read_slot(&mut self.fs.dev, &mut self.fs.block, at + ENTRY_SIZE).await? {
            Slot::Short(up) => up.name() == DOT_DOT && up.is_dir() && self.names_parent(parent, up.first_cluster(kind)),
            _ => false,
        };
        if !dot || !dot_dot {
            self.once(Finding::DotEntry { entry });
        }
        Ok(dot_dot && self.first_naming(parent, cluster, slot).await?.is_none())
    }

    /// Leaves `dir` for its parent and returns the parent and the slot
    /// after the entry naming `dir`.
    async fn ascend(&mut self, dir: &Dir) -> FsResult<(Dir, u32), D::Error> {
        let kind = self.fs.fat.geometry().kind();
        let at = self.fs.cluster_at(dir.cluster)? + ENTRY_SIZE;
        let Slot::Short(up) = rawio::read_slot(&mut self.fs.dev, &mut self.fs.block, at).await? else {
            return Err(ErrorKind::Corrupt.into());
        };
        let parent = self.dir_at(up.first_cluster(kind)).await?;
        let slot = self
            .first_naming(&parent, dir.cluster, u32::MAX)
            .await?
            .ok_or(ErrorKind::Corrupt)?;
        Ok((parent, slot + 1))
    }

    fn orphan(&mut self, entry: u64) {
        if !self.run.orphan {
            self.once(Finding::OrphanLfn { entry });
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
        if sequence & raw::LFN_LAST_ENTRY != 0 {
            self.abandon();
            let count = sequence & raw::LFN_SEQUENCE_MASK;
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

    /// Ends a long-name run at a short entry.
    fn close_run(&mut self, name: &[u8; 11]) {
        if self.run.pending {
            if self.run.expect != 0 {
                self.once(Finding::OrphanLfn { entry: self.run.start });
            } else if self.run.checksum != lfn::checksum(name) {
                self.once(Finding::LfnChecksum { entry: self.run.start });
            }
        }
        self.run = Run::default();
    }

    /// Ends a long-name run at a free slot or the end of a directory.
    fn break_run(&mut self) {
        self.abandon();
        self.run = Run::default();
    }

    /// Checks the chain of a file or directory entry and claims its
    /// clusters. Returns the chain's length when it starts at a data
    /// cluster.
    async fn entry_chain(&mut self, entry: u64, short: &ShortEntry) -> FsResult<Option<u32>, D::Error> {
        let kind = self.fs.fat.geometry().kind();
        let first = short.first_cluster(kind);
        let is_dir = short.is_dir();
        if first == 0 {
            if is_dir {
                self.once(Finding::InvalidCluster { entry, cluster: 0 });
            } else if short.size() > 0 {
                self.once(Finding::ChainTooShort { entry, size: short.size(), clusters: 0 });
            }
            return Ok(None);
        }
        if !(FIRST_DATA_CLUSTER..=self.fs.fat.geometry().max_cluster()).contains(&first) || Some(first) == self.root_cluster() {
            self.once(Finding::InvalidCluster { entry, cluster: first });
            return Ok(None);
        }
        let chain = self.analyze(first).await?;
        self.chain_end(entry, &chain);
        self.mark(first, chain.len, entry).await?;
        if !is_dir && matches!(chain.end, End::Eoc) {
            let needed = short.size().div_ceil(self.fs.fat.geometry().cluster_size());
            let clusters = chain.len;
            if clusters > needed {
                self.once(Finding::ChainTooLong { entry, size: short.size(), clusters });
            } else if clusters < needed {
                self.once(Finding::ChainTooShort { entry, size: short.size(), clusters });
            }
        }
        Ok(Some(chain.len))
    }

    /// Walks the tree from the root, depth first, through `..` entries.
    async fn walk_tree(&mut self) -> FsResult<(), D::Error> {
        let kind = self.fs.fat.geometry().kind();
        let mut dir = self.root().await?;
        if self.first_pass {
            self.report.directories += 1;
        }
        let mut walk = DirWalk::new(dir.start);
        let mut slot = 0;
        let mut label = false;
        self.run = Run::default();
        loop {
            let found = self.slot(&dir, &mut walk, slot).await?;
            let (offset, entry) = match found {
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
                    slot += 1;
                    continue;
                }
                Some((offset, Slot::Short(entry))) => (offset, entry),
            };
            self.close_run(&entry.name());
            slot += 1;
            if entry.is_label() {
                if !dir.root || label {
                    self.once(Finding::Label { entry: offset });
                }
                label |= dir.root;
                continue;
            }
            if entry.name() == DOT || entry.name() == DOT_DOT {
                self.once(Finding::DotEntry { entry: offset });
                continue;
            }
            if bad_name(&entry.name()) {
                self.once(Finding::BadName { entry: offset });
            }
            if !entry.is_dir() {
                if self.first_pass {
                    self.report.files += 1;
                }
                self.entry_chain(offset, &entry).await?;
                continue;
            }
            if entry.size() != 0 {
                self.once(Finding::DirectorySize { entry: offset });
            }
            let cluster = entry.first_cluster(kind);
            if self.entry_chain(offset, &entry).await?.is_some()
                && self.may_enter(&dir, slot - 1, offset, cluster).await?
            {
                dir = self.dir_at(cluster).await?;
                if self.first_pass {
                    self.report.directories += 1;
                }
                walk = DirWalk::new(dir.start);
                slot = 2;
            }
        }
    }

    /// Counts the FAT entries of this pass's clusters and reports the
    /// allocated ones no chain claimed.
    async fn scan_window(&mut self) -> FsResult<(), D::Error> {
        let kind = self.fs.fat.geometry().kind();
        for cluster in self.lo..self.hi {
            let value = rawio::get(&mut self.fs.dev, &mut self.fs.block, &self.fs.fat, cluster).await? & kind.mask();
            let bit = (cluster - self.lo) as usize;
            let claimed = self.bits[bit / 8] & (1 << (bit % 8)) != 0;
            if value == 0 {
                self.report.free += 1;
            } else if kind.is_bad(value) {
                self.report.bad += 1;
            } else {
                self.report.used += 1;
                if !claimed {
                    self.report.lost += 1;
                    if self.lost.1 > 0 && self.lost.0 + self.lost.1 == cluster {
                        self.lost.1 += 1;
                    } else {
                        self.flush_lost();
                        self.lost = (cluster, 1);
                    }
                    continue;
                }
            }
            self.flush_lost();
        }
        Ok(())
    }

    fn flush_lost(&mut self) {
        if self.lost.1 > 0 {
            let (first, count) = self.lost;
            self.lost = (0, 0);
            self.report(Finding::LostClusters { first, count });
        }
    }
}

}
