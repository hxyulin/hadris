use hadris_fs::{Clock, ErrorKind, FsResult, NodeTable};

use super::super::block_io::read_bytes;
use super::super::storage::BlockDevice;
use super::{Alloc, DirStart, ExFatFs, MAX_DEPTH, ROOT_ENTRY, Walk};
use hadris_fat_raw::exfat::{self as raw, ENTRY_SIZE, NameUnits, RawEntry};

use crate::exfat::findings::{CheckReport, Finding};
use crate::exfat::{le16, le32, le64};

/// The bitmap [`check`] uses: 32768 clusters a pass.
const DEFAULT_BITMAP: usize = 4096;
/// Bytes compared or summed at once.
const CHUNK: usize = 64;

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

/// An allocation named by an entry, checked and ready to mark.
#[derive(Clone, Copy)]
struct Claim {
    alloc: Alloc,
    /// Clusters that may be claimed and walked.
    clusters: u32,
}

/// A directory being scanned.
#[derive(Clone, Copy)]
struct Dir {
    walk: Walk,
    slot: u32,
}

struct Checker<'a, D, T: NodeTable, C: Clock, F> {
    fs: &'a mut ExFatFs<D, T, C>,
    bits: &'a mut [u8],
    /// The clusters this pass tracks, `lo..hi`.
    lo: u32,
    hi: u32,
    first_pass: bool,
    report: CheckReport,
    sink: F,
    /// A run of lost clusters not yet reported.
    lost: (u32, u32),
    /// A run of free clusters in use not yet reported.
    free_in_use: (u32, u32),
    /// Root entries seen: bitmap, up-case table, label, and the bitmap of
    /// the second FAT.
    seen: [bool; 4],
}

io_transform! {

/// Checks the volume without changing it, and returns how many problems of
/// each kind it found. See [`check_with`] for what is checked; this uses a
/// 4 KiB bitmap and discards the individual [`Finding`]s.
pub async fn check<D: BlockDevice, T: NodeTable, C: Clock>(
    fs: &mut ExFatFs<D, T, C>,
) -> FsResult<CheckReport, D::Error> {
    let mut bitmap = [0u8; DEFAULT_BITMAP];
    check_with(fs, &mut bitmap, |_| {}).await
}

/// Checks the volume without changing it, passing each [`Finding`] to
/// `on_finding`, and returns the totals.
///
/// Checked are the main boot region and its checksum, the backup boot
/// region, `VolumeFlags` and `PercentInUse`, the first two FAT entries,
/// the up-case table's checksum and mandatory mappings, and every entry
/// reached from the root: that system entries are in the root and there
/// once, that each File entry set is complete with a matching
/// `SetChecksum`, valid names and `NameHash`, and sizes that fit, and that
/// each allocation starts and continues in the heap, ends, has no cycle
/// and no bad cluster, fits its `DataLength` and shares no cluster with
/// another. The allocation bitmap is compared with the allocations: marked
/// clusters no allocation reaches are lost, and clusters in use that it
/// marks free are reported too. On a TexFAT volume the FAT and bitmap
/// that `ActiveFat` selects are checked.
///
/// `bitmap` holds one bit per cluster. The directory tree is walked once
/// for each `bitmap.len() * 8` clusters, so a bitmap of
/// `stats().total_blocks().div_ceil(8)` bytes checks in one pass; the
/// findings are the same with any size. The tree is walked with a fixed
/// stack of 64 directories; deeper directories are reported and not
/// entered.
///
/// The device is read as it is: sizes `ExFatFs` has not yet written, and
/// the `VolumeDirty` flag and `PercentInUse` it records in `sync`, show up
/// as findings. The node table is not used. An empty `bitmap` fails with
/// [`ErrorKind::InvalidInput`]; device errors end the check.
pub async fn check_with<D: BlockDevice, T: NodeTable, C: Clock, F: FnMut(Finding)>(
    fs: &mut ExFatFs<D, T, C>,
    bitmap: &mut [u8],
    on_finding: F,
) -> FsResult<CheckReport, D::Error> {
    if bitmap.is_empty() {
        return Err(ErrorKind::InvalidInput.into());
    }
    let mut checker = Checker {
        fs,
        bits: bitmap,
        lo: raw::FIRST_CLUSTER,
        hi: raw::FIRST_CLUSTER,
        first_pass: true,
        report: CheckReport::new(),
        sink: on_finding,
        lost: (0, 0),
        free_in_use: (0, 0),
        seen: [false; 4],
    };
    checker.check().await?;
    Ok(checker.report)
}

impl<D: BlockDevice, T: NodeTable, C: Clock, F: FnMut(Finding)> Checker<'_, D, T, C, F> {
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
        self.boot().await?;
        let max = self.fs.geo.max_cluster();
        let window = (self.bits.len() as u64 * 8).min(1 << 31) as u32;
        let mut lo = raw::FIRST_CLUSTER;
        loop {
            self.lo = lo;
            self.hi = lo.saturating_add(window).min(max + 1);
            self.bits.fill(0);
            self.report.passes += 1;
            self.seen = [false; 4];
            self.walk_tree().await?;
            self.scan_window().await?;
            self.first_pass = false;
            if self.hi > max {
                break;
            }
            lo = self.hi;
        }
        self.flush_lost();
        self.flush_free_in_use();
        let mut recorded = [0u8; 1];
        self.read(112, &mut recorded).await?;
        let recorded = recorded[0];
        let count = self.fs.geo.cluster_count() as u64;
        let used = self.report.used as u64;
        let floor = (used * 100 / count) as u8;
        let ceil = (used * 100).div_ceil(count) as u8;
        if recorded != 0xFF && recorded != floor && recorded != ceil {
            self.report(Finding::PercentInUse { recorded, actual: floor });
        }
        Ok(())
    }

    async fn read(&mut self, offset: u64, out: &mut [u8]) -> FsResult<(), D::Error> {
        read_bytes(&mut self.fs.dev, &mut self.fs.block, offset, out).await
    }

    /// Checks the boot regions, the first FAT entries and the up-case
    /// table.
    async fn boot(&mut self) -> FsResult<(), D::Error> {
        let sector = self.fs.geo.sector_size();
        let region = raw::BOOT_REGION_SECTORS * sector;
        let mut sum = 0u32;
        let mut backup_differs = false;
        let mut chunk = [0u8; CHUNK];
        let mut copy = [0u8; CHUNK];
        let mut at = 0;
        while at < region {
            self.read(at, &mut chunk).await?;
            self.read(region + at, &mut copy).await?;
            let index = at / sector;
            let within = (at % sector) as usize;
            for (offset, (&a, &b)) in chunk.iter().zip(&copy).enumerate() {
                let skipped = index == 0 && raw::CHECKSUM_SKIPPED.contains(&(within + offset));
                backup_differs |= a != b && !skipped;
            }
            if index < raw::BOOT_REGION_SECTORS - 1 {
                if index == 0 {
                    for (offset, &byte) in chunk.iter().enumerate() {
                        if !raw::CHECKSUM_SKIPPED.contains(&(within + offset)) {
                            sum = sum.rotate_right(1).wrapping_add(byte as u32);
                        }
                    }
                } else {
                    sum = raw::table_checksum(sum, &chunk);
                }
                if (1..=8).contains(&index) && within as u64 + CHUNK as u64 == sector && le32(&chunk, CHUNK - 4) != raw::EXTENDED_BOOT_SIGNATURE {
                    self.report(Finding::BootSector("ExtendedBootSignature"));
                }
            }
            at += CHUNK as u64;
        }
        let checksum_at = (raw::BOOT_REGION_SECTORS - 1) * sector;
        let mut stored = 0;
        while stored < sector {
            self.read(checksum_at + stored, &mut chunk).await?;
            if chunk.chunks_exact(4).any(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]) != sum) {
                self.report(Finding::BootChecksum);
                break;
            }
            stored += CHUNK as u64;
        }
        if backup_differs {
            self.report(Finding::BackupBootRegion);
        }
        let mut flags = [0u8; 2];
        self.read(106, &mut flags).await?;
        if u16::from_le_bytes(flags) & raw::VOLUME_DIRTY != 0 {
            self.report(Finding::VolumeDirty);
        }
        if self.fs.fat_entry(0).await? != raw::FAT_MEDIA || self.fs.fat_entry(1).await? != raw::FAT_END {
            self.report(Finding::FatEntries);
        }
        if !self.fs.upcase.is_valid() {
            self.report(Finding::UpcaseTable);
        }
        Ok(())
    }

    async fn link(&mut self, cluster: u32) -> FsResult<Link, D::Error> {
        let stored = self.fs.fat_entry(cluster).await?;
        Ok(match stored {
            raw::FAT_END => Link::End(End::Eoc),
            raw::FAT_BAD => Link::End(End::Bad(cluster)),
            next if self.fs.geo.is_cluster(next) => Link::Next(next),
            next => Link::End(End::Broken { cluster, next }),
        })
    }

    async fn step(&mut self, cluster: u32) -> FsResult<u32, D::Error> {
        match self.link(cluster).await? {
            Link::Next(next) => Ok(next),
            Link::End(_) => Err(ErrorKind::Corrupt.into()),
        }
    }

    /// Measures the chain at `first`, a heap cluster, finding a cycle with
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

    /// Checks the allocation an entry names and returns what to mark:
    /// `len` bytes from `first`, contiguous or chained. `None` when it has
    /// no clusters.
    async fn claim(&mut self, entry: u64, first: u32, len: u64, contiguous: bool) -> FsResult<Option<Claim>, D::Error> {
        let cluster_size = self.fs.geo.cluster_size();
        let needed = len.div_ceil(cluster_size);
        if first == 0 {
            if len > 0 {
                self.once(Finding::InvalidCluster { entry, cluster: 0 });
            }
            return Ok(None);
        }
        if !self.fs.geo.is_cluster(first) {
            self.once(Finding::InvalidCluster { entry, cluster: first });
            return Ok(None);
        }
        let alloc = Alloc { first, contiguous };
        if contiguous {
            let room = (self.fs.geo.max_cluster() - first + 1) as u64;
            if needed > room {
                self.once(Finding::ChainTooShort { entry, size: len, clusters: room as u32 });
            }
            return Ok(Some(Claim { alloc, clusters: needed.min(room) as u32 }));
        }
        let chain = self.analyze(first).await?;
        match chain.end {
            End::Eoc => {}
            End::Bad(cluster) => self.once(Finding::BadCluster { entry, cluster }),
            End::Broken { cluster, next } => self.once(Finding::BrokenChain { entry, cluster, next }),
            End::Cycle(cluster) => self.once(Finding::CyclicChain { entry, cluster }),
        }
        let clusters = chain.len as u64;
        if len == u64::MAX {
            return Ok(Some(Claim { alloc, clusters: chain.len }));
        }
        if clusters > needed {
            self.once(Finding::ChainTooLong { entry, size: len, clusters: chain.len });
        } else if clusters < needed && matches!(chain.end, End::Eoc) {
            self.once(Finding::ChainTooShort { entry, size: len, clusters: chain.len });
        }
        Ok(Some(Claim { alloc, clusters: chain.len }))
    }

    /// Claims the clusters of `claim` for `entry`.
    async fn mark(&mut self, claim: Claim, entry: u64) -> FsResult<(), D::Error> {
        let mut cluster = claim.alloc.first;
        for index in 0..claim.clusters {
            if (self.lo..self.hi).contains(&cluster) {
                let bit = (cluster - self.lo) as usize;
                let (byte, mask) = (bit / 8, 1u8 << (bit % 8));
                if self.bits[byte] & mask != 0 {
                    self.report(Finding::CrossLinked { entry, cluster });
                } else {
                    self.bits[byte] |= mask;
                }
            }
            if index + 1 < claim.clusters {
                cluster = if claim.alloc.contiguous { cluster + 1 } else { self.step(cluster).await? };
            }
        }
        Ok(())
    }

    /// The directory an allocation holds, limited to its checked clusters.
    fn dir(&self, claim: Option<Claim>, len: u64) -> Option<Dir> {
        let claim = claim?;
        let size = len.min(claim.clusters as u64 * self.fs.geo.cluster_size());
        Some(Dir { walk: Walk::new(DirStart { alloc: claim.alloc, size }), slot: 0 })
    }

    async fn entry_at(&mut self, dir: &mut Dir, slot: u32) -> FsResult<Option<(u64, RawEntry)>, D::Error> {
        let Some(at) = self.fs.slot_offset(&mut dir.walk, slot).await? else {
            return Ok(None);
        };
        let mut entry = [0u8; ENTRY_SIZE];
        self.read(at, &mut entry).await?;
        Ok(Some((at, entry)))
    }

    async fn walk_tree(&mut self) -> FsResult<(), D::Error> {
        let root = self.fs.geo.root();
        let Some(claim) = self.claim(ROOT_ENTRY, root, u64::MAX, false).await? else {
            return Ok(());
        };
        self.mark(claim, ROOT_ENTRY).await?;
        if self.first_pass {
            self.report.directories += 1;
        }
        let Some(root) = self.dir(Some(claim), u64::MAX) else {
            return Ok(());
        };
        let mut stack = [root; MAX_DEPTH];
        let mut depth = 1;
        while depth > 0 {
            let mut dir = stack[depth - 1];
            let slot = dir.slot;
            let Some((at, entry)) = self.entry_at(&mut dir, slot).await? else {
                depth -= 1;
                continue;
            };
            dir.slot += 1;
            let is_root = depth == 1;
            let kind = entry[0];
            if kind == raw::ENTRY_END {
                depth -= 1;
                continue;
            }
            if kind & raw::IN_USE == 0 {
                stack[depth - 1] = dir;
                continue;
            }
            match kind {
                raw::ENTRY_BITMAP | raw::ENTRY_UPCASE | raw::ENTRY_LABEL => {
                    let second_bitmap = kind == raw::ENTRY_BITMAP && entry[1] & 1 != 0;
                    let which = if second_bitmap { 3 } else { (kind - raw::ENTRY_BITMAP) as usize };
                    let bad_label = kind == raw::ENTRY_LABEL && entry[1] as usize > raw::MAX_LABEL_UNITS;
                    let one_fat = second_bitmap && self.fs.geo.mirror_fat().is_none();
                    if !is_root || self.seen[which] || bad_label || one_fat {
                        self.once(Finding::RootEntry { entry: at });
                    }
                    self.seen[which] = true;
                    if kind != raw::ENTRY_LABEL
                        && let Some(claim) = self.claim(at, le32(&entry, 20), le64(&entry, 24), false).await?
                    {
                        self.mark(claim, at).await?;
                    }
                }
                raw::ENTRY_FILE => {
                    let (count, child) = self.file_set(&mut dir, slot, at, entry).await?;
                    dir.slot = slot + count;
                    if let Some(child) = child {
                        if depth == MAX_DEPTH {
                            self.once(Finding::TooDeep { entry: at });
                        } else {
                            stack[depth - 1] = dir;
                            stack[depth] = child;
                            depth += 1;
                            continue;
                        }
                    }
                }
                kind if kind & raw::CATEGORY_SECONDARY != 0 => self.once(Finding::EntrySet { entry: at }),
                kind if kind & raw::IMPORTANCE_BENIGN != 0 => dir.slot = slot + 1 + entry[1] as u32,
                _ => self.once(Finding::EntrySet { entry: at }),
            }
            stack[depth - 1] = dir;
        }
        Ok(())
    }

    /// Checks the File entry set at `slot` and marks its allocations.
    /// Returns the entries it spans and, for a directory to enter, where
    /// its entries are.
    async fn file_set(&mut self, dir: &mut Dir, slot: u32, at: u64, primary: RawEntry) -> FsResult<(u32, Option<Dir>), D::Error> {
        let count = 1 + primary[1] as usize;
        let mut set = [[0u8; ENTRY_SIZE]; raw::MAX_SET];
        set[0] = primary;
        let mut read = 1;
        while read < count.min(raw::MAX_SET) {
            let Some((_, entry)) = self.entry_at(dir, slot + read as u32).await? else {
                break;
            };
            if entry[0] & (raw::IN_USE | raw::CATEGORY_SECONDARY) != raw::IN_USE | raw::CATEGORY_SECONDARY {
                break;
            }
            set[read] = entry;
            read += 1;
        }
        let stream = set[1];
        let name_len = stream[3] as usize;
        let name_entries = name_len.div_ceil(raw::NAME_UNITS_PER_ENTRY);
        let complete = (3..=raw::MAX_SET).contains(&count)
            && read == count
            && stream[0] == raw::ENTRY_STREAM
            && name_len > 0
            && 2 + name_entries <= count
            && set[2..2 + name_entries].iter().all(|entry| entry[0] == raw::ENTRY_NAME);
        if !complete {
            self.once(Finding::EntrySet { entry: at });
            return Ok((read as u32, None));
        }
        if raw::set_checksum(&set[..count]) != le16(&primary, 2) {
            self.once(Finding::SetChecksum { entry: at });
        }
        let mut name = NameUnits::new();
        for index in 0..name_len {
            let entry = &set[2 + index / raw::NAME_UNITS_PER_ENTRY];
            name.push(le16(entry, 2 + (index % raw::NAME_UNITS_PER_ENTRY) * 2));
        }
        let units = name.as_slice();
        if units.iter().any(|&unit| !raw::valid_unit(unit)) || units == [0x2E] || units == [0x2E, 0x2E] {
            self.once(Finding::BadName { entry: at });
        }
        if self.first_pass {
            let mut upcased = name;
            for unit in upcased.as_mut_slice() {
                *unit = self.fs.upcase_unit(*unit).await?;
            }
            if raw::name_hash(upcased.as_slice()) != le16(&stream, 4) {
                self.report(Finding::NameHash { entry: at });
            }
        }
        let is_dir = le16(&primary, 4) & raw::ATTR_DIRECTORY != 0;
        let len = le64(&stream, 24);
        let valid = le64(&stream, 8);
        if is_dir {
            if len % self.fs.geo.cluster_size() != 0 || len > raw::MAX_DIRECTORY_SIZE || valid != len {
                self.once(Finding::DirectorySize { entry: at });
            }
        } else if valid > len {
            self.once(Finding::ValidDataLength { entry: at });
        }
        if self.first_pass {
            if is_dir {
                self.report.directories += 1;
            } else {
                self.report.files += 1;
            }
        }
        let mut child = None;
        if stream[1] & raw::ALLOCATION_POSSIBLE != 0 {
            let claim = self.claim(at, le32(&stream, 20), len, stream[1] & raw::NO_FAT_CHAIN != 0).await?;
            if let Some(claim) = claim {
                self.mark(claim, at).await?;
            }
            if is_dir {
                child = self.dir(claim, len.min(raw::MAX_DIRECTORY_SIZE));
            }
        }
        for extra in &set[2 + name_entries..count] {
            if extra[0] & raw::IMPORTANCE_BENIGN != 0
                && extra[1] & raw::ALLOCATION_POSSIBLE != 0
                && let Some(claim) = self.claim(at, le32(extra, 20), le64(extra, 24), extra[1] & raw::NO_FAT_CHAIN != 0).await?
            {
                self.mark(claim, at).await?;
            }
        }
        Ok((count as u32, child))
    }

    /// Compares this pass's clusters in the allocation bitmap with the
    /// claims.
    async fn scan_window(&mut self) -> FsResult<(), D::Error> {
        let mut chunk = [0u8; CHUNK];
        let mut cluster = self.lo;
        while cluster < self.hi {
            let pos = ((cluster - raw::FIRST_CLUSTER) / 8) as u64;
            let want = ((self.hi - cluster).div_ceil(8) as usize).min(CHUNK);
            let n = self.fs.bitmap_bytes(pos, &mut chunk[..want]).await?;
            for &byte in &chunk[..n] {
                for bit in 0..8 {
                    if cluster >= self.hi {
                        break;
                    }
                    let index = (cluster - self.lo) as usize;
                    let claimed = self.bits[index / 8] & (1 << (index % 8)) != 0;
                    let allocated = byte & (1 << bit) != 0;
                    self.tally(cluster, allocated, claimed).await?;
                    cluster += 1;
                }
            }
        }
        Ok(())
    }

    async fn tally(&mut self, cluster: u32, allocated: bool, claimed: bool) -> FsResult<(), D::Error> {
        if !allocated {
            self.report.free += 1;
            self.flush_lost();
            if claimed {
                if self.free_in_use.1 > 0 && self.free_in_use.0 + self.free_in_use.1 == cluster {
                    self.free_in_use.1 += 1;
                } else {
                    self.flush_free_in_use();
                    self.free_in_use = (cluster, 1);
                }
            } else {
                self.flush_free_in_use();
            }
            return Ok(());
        }
        self.flush_free_in_use();
        self.report.used += 1;
        if claimed {
            self.flush_lost();
            return Ok(());
        }
        if self.fs.fat_entry(cluster).await? == raw::FAT_BAD {
            self.report.bad += 1;
            self.flush_lost();
            return Ok(());
        }
        self.report.lost += 1;
        if self.lost.1 > 0 && self.lost.0 + self.lost.1 == cluster {
            self.lost.1 += 1;
        } else {
            self.flush_lost();
            self.lost = (cluster, 1);
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

    fn flush_free_in_use(&mut self) {
        if self.free_in_use.1 > 0 {
            let (first, count) = self.free_in_use;
            self.free_in_use = (0, 0);
            self.report(Finding::FreeInUse { first, count });
        }
    }
}

}
