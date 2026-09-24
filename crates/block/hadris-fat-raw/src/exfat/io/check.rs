use hadris_fs::{CheckReport, ErrorKind, Finding, FsResult, Location, Severity};

use super::block::read_bytes;
use super::storage::BlockDevice;
use super::volume::{bitmap_bytes, get, read_boot, read_volume, slot_offset, upcase};
use crate::exfat::io::{BootRegion, DirWalk, ExFat, Extent, Upcase};
use crate::exfat::{self as raw, Detail, ENTRY_SIZE, NameUnits, RawEntry};
use crate::io::BlockBuf;

/// The largest device block `check` reads.
const MAX_BLOCK: usize = 4096;
/// Bytes of the scratch buffer that hold the path of a finding.
const PATH_LEN: usize = 1024;
/// The smallest cluster bitmap: 4096 clusters a pass.
const MIN_BITMAP: usize = 512;
/// Bytes compared or summed at once.
const CHUNK: usize = 64;
/// Directories deeper than this are reported and not entered.
const MAX_DEPTH: usize = 64;
/// The entry that stands for the root directory.
const ROOT_ENTRY: u64 = 0;
/// Offset of `VolumeFlags` in the boot sector.
const VOLUME_FLAGS_AT: u64 = 106;
/// Offset of `PercentInUse` in the boot sector.
const PERCENT_AT: u64 = 112;

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}

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

/// An allocation named by an entry, checked and ready to mark.
#[derive(Clone, Copy)]
struct Claim {
    first: u32,
    contiguous: bool,
    /// Clusters that may be claimed and walked.
    clusters: u32,
}

/// A directory being scanned.
#[derive(Clone, Copy)]
struct Dir {
    walk: DirWalk,
    slot: u32,
}

/// What a finding's path names.
#[derive(Clone, Copy)]
enum Scope {
    Volume,
    /// The directory being walked.
    Dir,
    /// The entry set being checked, or the directory when there is none.
    Entry,
}

/// Writes `/` and `units` as UTF-8 to `out`, unpaired surrogates as
/// U+FFFD. Returns the length, or `None` when it does not fit.
fn put_name(out: &mut [u8], units: &[u16]) -> Option<usize> {
    *out.first_mut()? = b'/';
    let mut len = 1;
    for ch in char::decode_utf16(units.iter().copied()) {
        let mut utf8 = [0u8; 4];
        let bytes = ch
            .unwrap_or(char::REPLACEMENT_CHARACTER)
            .encode_utf8(&mut utf8)
            .as_bytes();
        out.get_mut(len..len + bytes.len())?.copy_from_slice(bytes);
        len += bytes.len();
    }
    Some(len)
}

struct Checker<'a, D, F> {
    dev: &'a mut D,
    block: BlockBuf<[u8; MAX_BLOCK]>,
    vol: ExFat,
    upcase: Upcase,
    /// The path of the directory being walked, `dir_len` bytes, followed by
    /// `/` and the name of the entry set being checked, `name_len` bytes.
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
    used: u32,
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

/// Checks an unmounted exFAT volume without changing it, passes each
/// [`Finding`] to `on_finding`, and returns the totals.
///
/// Checked are the main boot region and its checksum, the backup boot
/// region, `VolumeFlags` and `PercentInUse`, the first two FAT entries,
/// the up-case table's checksum and mandatory mappings, and every entry
/// reached from the root: that system entries are in the root and there
/// once, that each File entry set is complete with a matching
/// `SetChecksum`, valid names and `NameHash`, and sizes that fit, and that
/// each allocation starts and continues in the heap, ends, has no cycle
/// and no bad cluster, fits its `DataLength` and shares no cluster with
/// another. The allocation bitmap is compared with the allocations:
/// marked clusters no allocation reaches are lost, and clusters in use
/// that it marks free are reported too. On a TexFAT volume the FAT and
/// bitmap that `ActiveFat` selects are checked. Each finding carries a
/// [`Detail`] code, the one mount errors use. `VolumeDirty` and a wrong
/// `PercentInUse` are [`Severity::Notice`]s, the rest
/// [`Severity::Error`]s.
///
/// A main boot region that does not mount is a finding, and the check
/// goes on from the backup; without a usable backup, or without the
/// Allocation Bitmap and Up-case Table entries, `check` fails with the
/// error mounting gives.
///
/// Nothing is allocated. The first 1 KiB of `scratch` holds the path of
/// each finding, and the rest a bitmap of one bit per cluster. The tree is
/// walked once for each `(scratch.len() - 1024) * 8` clusters, so the
/// findings are the same with any size, with a fixed stack of 64
/// directories; deeper directories are reported and not entered. A path
/// that does not fit ends at the last whole name that does.
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
    let (geo, region) = read_boot(dev, &mut block).await?;
    let mut upcase = Upcase::new();
    let vol = read_volume(dev, &mut block, geo, &mut upcase).await?;
    let (path, bits) = scratch.split_at_mut(PATH_LEN);
    let mut checker = Checker {
        dev,
        block,
        vol,
        upcase,
        path,
        dir_len: 0,
        name_len: 0,
        hidden: 0,
        bits,
        lo: raw::FIRST_CLUSTER,
        hi: raw::FIRST_CLUSTER,
        first_pass: true,
        findings: 0,
        passes: 0,
        used: 0,
        sink: on_finding,
        lost: (0, 0),
        free_in_use: (0, 0),
        seen: [false; 4],
    };
    checker.check(region).await?;
    Ok(CheckReport::new(checker.findings, checker.passes))
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

    async fn check(&mut self, region: BootRegion) -> FsResult<(), D::Error> {
        self.boot(region).await?;
        let max = self.vol.geometry().max_cluster();
        let window = (self.bits.len() as u64 * 8).min(1 << 31) as u32;
        let mut lo = raw::FIRST_CLUSTER;
        loop {
            self.lo = lo;
            self.hi = lo.saturating_add(window).min(max + 1);
            self.bits.fill(0);
            self.passes += 1;
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
        self.read(PERCENT_AT, &mut recorded).await?;
        let recorded = recorded[0];
        let count = self.vol.geometry().cluster_count() as u64;
        let used = self.used as u64;
        let floor = (used * 100 / count) as u8;
        let ceil = (used * 100).div_ceil(count) as u8;
        if recorded != 0xFF && recorded != floor && recorded != ceil {
            let finding = found(Detail::PercentInUse).with_severity(Severity::Notice);
            self.report(finding.with_location(Location::Byte(PERCENT_AT)), Scope::Volume);
        }
        Ok(())
    }

    async fn read(&mut self, offset: u64, out: &mut [u8]) -> FsResult<(), D::Error> {
        read_bytes(self.dev, &mut self.block, offset, out).await
    }

    async fn fat_entry(&mut self, cluster: u32) -> FsResult<u32, D::Error> {
        get(self.dev, &mut self.block, self.vol.geometry(), cluster).await
    }

    /// Checks the boot regions, the first FAT entries and the up-case
    /// table. `region` is the one the geometry came from.
    async fn boot(&mut self, region: BootRegion) -> FsResult<(), D::Error> {
        let sector = self.vol.geometry().sector_size();
        let region_len = raw::BOOT_REGION_SECTORS * sector;
        if region == BootRegion::Backup {
            let mut main = [0u8; 512];
            self.read(0, &mut main).await?;
            let boot: raw::BootSector = bytemuck::pod_read_unaligned(&main);
            if raw::parse_boot(&boot).is_err() {
                let finding = said(Detail::BootSector, "boot sector is damaged; checked from the backup");
                self.report(finding.with_location(Location::Byte(0)), Scope::Volume);
            }
        }
        let mut sum = 0u32;
        let mut backup_differs = false;
        let mut chunk = [0u8; CHUNK];
        let mut copy = [0u8; CHUNK];
        let mut at = 0;
        while at < region_len {
            self.read(at, &mut chunk).await?;
            self.read(region_len + at, &mut copy).await?;
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
                if (1..=8).contains(&index)
                    && within as u64 + CHUNK as u64 == sector
                    && le32(&chunk, CHUNK - 4) != raw::EXTENDED_BOOT_SIGNATURE
                {
                    let finding = said(Detail::BootSector, "missing ExtendedBootSignature");
                    self.report(finding.with_location(Location::Byte(index * sector)), Scope::Volume);
                }
            }
            at += CHUNK as u64;
        }
        let checksum_at = (raw::BOOT_REGION_SECTORS - 1) * sector;
        let mut stored = 0;
        while stored < sector {
            self.read(checksum_at + stored, &mut chunk).await?;
            if chunk.chunks_exact(4).any(|word| le32(word, 0) != sum) {
                self.report(found(Detail::BootChecksum).with_location(Location::Byte(checksum_at)), Scope::Volume);
                break;
            }
            stored += CHUNK as u64;
        }
        if backup_differs {
            self.report(found(Detail::BackupBootRegion).with_location(Location::Byte(region_len)), Scope::Volume);
        }
        let mut flags = [0u8; 2];
        self.read(VOLUME_FLAGS_AT, &mut flags).await?;
        if u16::from_le_bytes(flags) & raw::VOLUME_DIRTY != 0 {
            let finding = found(Detail::Dirty).with_severity(Severity::Notice);
            self.report(finding.with_location(Location::Byte(VOLUME_FLAGS_AT)), Scope::Volume);
        }
        if self.fat_entry(0).await? != raw::FAT_MEDIA || self.fat_entry(1).await? != raw::FAT_END {
            let at = Location::Byte(self.vol.geometry().fat_start());
            self.report(found(Detail::FatEntries).with_location(at), Scope::Volume);
        }
        if !self.upcase.is_valid() {
            let at = Location::Cluster(self.upcase.extent().first() as u64);
            self.report(found(Detail::UpcaseTable).with_location(at), Scope::Volume);
        }
        Ok(())
    }

    async fn link(&mut self, cluster: u32) -> FsResult<Link, D::Error> {
        let stored = self.fat_entry(cluster).await?;
        Ok(match stored {
            raw::FAT_END => Link::End(End::Eoc),
            raw::FAT_BAD => Link::End(End::Bad(cluster)),
            next if self.vol.geometry().is_cluster(next) => Link::Next(next),
            _ => Link::End(End::Broken(cluster)),
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

    /// Checks the allocation the entry at `entry` names and returns what to
    /// mark: `len` bytes from `first`, contiguous or chained. `None` when
    /// it has no clusters.
    async fn claim(&mut self, entry: u64, first: u32, len: u64, contiguous: bool) -> FsResult<Option<Claim>, D::Error> {
        let cluster_size = self.vol.geometry().cluster_size();
        let needed = len.div_ceil(cluster_size);
        if first == 0 {
            if len > 0 {
                self.once(found(Detail::InvalidCluster).with_location(Location::Cluster(0)), Scope::Entry);
            }
            return Ok(None);
        }
        if !self.vol.geometry().is_cluster(first) {
            self.once(found(Detail::InvalidCluster).with_location(Location::Cluster(first as u64)), Scope::Entry);
            return Ok(None);
        }
        let short = said(Detail::SizeMismatch, "allocation is shorter than its data length needs")
            .with_location(Location::Byte(entry));
        if contiguous {
            let room = (self.vol.geometry().max_cluster() - first + 1) as u64;
            if needed > room {
                self.once(short, Scope::Entry);
            }
            return Ok(Some(Claim { first, contiguous, clusters: needed.min(room) as u32 }));
        }
        let chain = self.analyze(first).await?;
        let (detail, cluster) = match chain.end {
            End::Eoc => (None, 0),
            End::Bad(cluster) => (Some(Detail::BadCluster), cluster),
            End::Broken(cluster) => (Some(Detail::BrokenChain), cluster),
            End::Cycle(cluster) => (Some(Detail::CyclicChain), cluster),
        };
        if let Some(detail) = detail {
            self.once(found(detail).with_location(Location::Cluster(cluster as u64)), Scope::Entry);
        }
        let claim = Claim { first, contiguous, clusters: chain.len };
        if len == u64::MAX {
            return Ok(Some(claim));
        }
        let clusters = chain.len as u64;
        if clusters > needed {
            let long = said(Detail::SizeMismatch, "allocation is longer than its data length needs");
            self.once(long.with_location(Location::Byte(entry)), Scope::Entry);
        } else if clusters < needed && matches!(chain.end, End::Eoc) {
            self.once(short, Scope::Entry);
        }
        Ok(Some(claim))
    }

    /// Claims the clusters of `claim` for the entry being checked.
    async fn mark(&mut self, claim: Claim) -> FsResult<(), D::Error> {
        let mut cluster = claim.first;
        for index in 0..claim.clusters {
            if (self.lo..self.hi).contains(&cluster) {
                let bit = (cluster - self.lo) as usize;
                let (byte, mask) = (bit / 8, 1u8 << (bit % 8));
                if self.bits[byte] & mask != 0 {
                    self.report(found(Detail::CrossLink).with_location(Location::Cluster(cluster as u64)), Scope::Entry);
                } else {
                    self.bits[byte] |= mask;
                }
            }
            if index + 1 < claim.clusters {
                cluster = if claim.contiguous { cluster + 1 } else { self.step(cluster).await? };
            }
        }
        Ok(())
    }

    /// The directory an allocation holds, limited to its checked clusters.
    fn dir(&self, claim: Option<Claim>, len: u64) -> Option<Dir> {
        let claim = claim?;
        let size = len.min(claim.clusters as u64 * self.vol.geometry().cluster_size());
        let extent = if claim.contiguous {
            Extent::contiguous(claim.first, size)
        } else {
            Extent::chain(claim.first, size)
        };
        Some(Dir { walk: DirWalk::new(extent), slot: 0 })
    }

    async fn entry_at(&mut self, dir: &mut Dir, slot: u32) -> FsResult<Option<(u64, RawEntry)>, D::Error> {
        let Some(at) = slot_offset(self.dev, &mut self.block, self.vol.geometry(), &mut dir.walk, slot).await? else {
            return Ok(None);
        };
        let mut entry = [0u8; ENTRY_SIZE];
        self.read(at, &mut entry).await?;
        Ok(Some((at, entry)))
    }

    /// Makes the entry set being checked the directory walked.
    fn descend(&mut self) {
        if self.hidden > 0 || self.name_len == 0 {
            self.hidden += 1;
        } else {
            self.dir_len += self.name_len;
        }
        self.name_len = 0;
    }

    /// Leaves the directory walked for its parent.
    fn ascend(&mut self) {
        if self.hidden > 0 {
            self.hidden -= 1;
        } else {
            self.dir_len = self.path[..self.dir_len].iter().rposition(|&b| b == b'/').unwrap_or(0);
        }
        self.name_len = 0;
    }

    async fn walk_tree(&mut self) -> FsResult<(), D::Error> {
        (self.dir_len, self.name_len, self.hidden) = (0, 0, 0);
        let root = self.vol.geometry().root();
        let Some(claim) = self.claim(ROOT_ENTRY, root, u64::MAX, false).await? else {
            return Ok(());
        };
        self.mark(claim).await?;
        let Some(root) = self.dir(Some(claim), u64::MAX) else {
            return Ok(());
        };
        let mut stack = [root; MAX_DEPTH];
        let mut depth = 1;
        while depth > 0 {
            let mut dir = stack[depth - 1];
            let slot = dir.slot;
            self.name_len = 0;
            let Some((at, entry)) = self.entry_at(&mut dir, slot).await? else {
                depth -= 1;
                if depth > 0 {
                    self.ascend();
                }
                continue;
            };
            dir.slot += 1;
            let is_root = depth == 1;
            let kind = entry[0];
            if kind == raw::ENTRY_END {
                depth -= 1;
                if depth > 0 {
                    self.ascend();
                }
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
                    let one_fat = second_bitmap && self.vol.geometry().mirror_fat().is_none();
                    if !is_root || self.seen[which] || bad_label || one_fat {
                        self.once(found(Detail::RootEntry).with_location(Location::Byte(at)), Scope::Dir);
                    }
                    self.seen[which] = true;
                    if kind != raw::ENTRY_LABEL
                        && let Some(claim) = self.claim(at, le32(&entry, 20), le64(&entry, 24), false).await?
                    {
                        self.mark(claim).await?;
                    }
                }
                raw::ENTRY_FILE => {
                    let (count, child) = self.file_set(&mut dir, slot, at, entry).await?;
                    dir.slot = slot + count;
                    if let Some(child) = child {
                        if depth == MAX_DEPTH {
                            self.once(found(Detail::TooDeep).with_location(Location::Byte(at)), Scope::Entry);
                        } else {
                            stack[depth - 1] = dir;
                            stack[depth] = child;
                            depth += 1;
                            self.descend();
                            continue;
                        }
                    }
                }
                kind if kind & raw::CATEGORY_SECONDARY != 0 => {
                    self.once(found(Detail::EntrySet).with_location(Location::Byte(at)), Scope::Dir);
                }
                kind if kind & raw::IMPORTANCE_BENIGN != 0 => dir.slot = slot + 1 + entry[1] as u32,
                _ => self.once(found(Detail::EntrySet).with_location(Location::Byte(at)), Scope::Dir),
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
        let here = Location::Byte(at);
        if !complete {
            self.once(found(Detail::EntrySet).with_location(here), Scope::Dir);
            return Ok((read as u32, None));
        }
        let mut name = NameUnits::new();
        for index in 0..name_len {
            let entry = &set[2 + index / raw::NAME_UNITS_PER_ENTRY];
            name.push(le16(entry, 2 + (index % raw::NAME_UNITS_PER_ENTRY) * 2));
        }
        self.name_len = match self.hidden {
            0 => put_name(&mut self.path[self.dir_len..], name.as_slice()).unwrap_or(0),
            _ => 0,
        };
        if raw::set_checksum(&set[..count]) != le16(&primary, 2) {
            self.once(found(Detail::SetChecksum).with_location(here), Scope::Entry);
        }
        let units = name.as_slice();
        if units.iter().any(|&unit| !raw::valid_unit(unit)) || units == [0x2E] || units == [0x2E, 0x2E] {
            self.once(found(Detail::BadName).with_location(here), Scope::Entry);
        }
        if self.first_pass {
            let mut upcased = name;
            for unit in upcased.as_mut_slice() {
                *unit = upcase(self.dev, &mut self.block, self.vol.geometry(), &mut self.upcase, *unit).await?;
            }
            if raw::name_hash(upcased.as_slice()) != le16(&stream, 4) {
                self.report(found(Detail::NameHash).with_location(here), Scope::Entry);
            }
        }
        let is_dir = le16(&primary, 4) & raw::ATTR_DIRECTORY != 0;
        let len = le64(&stream, 24);
        let valid = le64(&stream, 8);
        if is_dir {
            if len % self.vol.geometry().cluster_size() != 0 || len > raw::MAX_DIRECTORY_SIZE || valid != len {
                self.once(found(Detail::DirectorySize).with_location(here), Scope::Entry);
            }
        } else if valid > len {
            self.once(found(Detail::ValidDataLength).with_location(here), Scope::Entry);
        }
        let mut child = None;
        if stream[1] & raw::ALLOCATION_POSSIBLE != 0 {
            let claim = self.claim(at, le32(&stream, 20), len, stream[1] & raw::NO_FAT_CHAIN != 0).await?;
            if let Some(claim) = claim {
                self.mark(claim).await?;
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
                self.mark(claim).await?;
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
            let n = bitmap_bytes(self.dev, &mut self.block, &mut self.vol, pos, &mut chunk[..want]).await?;
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
        self.used += 1;
        if claimed || self.fat_entry(cluster).await? == raw::FAT_BAD {
            self.flush_lost();
            return Ok(());
        }
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
            let first = self.lost.0;
            self.lost = (0, 0);
            self.report(found(Detail::LostClusters).with_location(Location::Cluster(first as u64)), Scope::Volume);
        }
    }

    fn flush_free_in_use(&mut self) {
        if self.free_in_use.1 > 0 {
            let first = self.free_in_use.0;
            self.free_in_use = (0, 0);
            let finding = said(Detail::Bitmap, "clusters in use are marked free in the allocation bitmap");
            self.report(finding.with_location(Location::Cluster(first as u64)), Scope::Volume);
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

#[cfg(test)]
mod tests {
    use super::put_name;

    #[test]
    fn names_decode_to_utf8_paths() {
        let mut out = [0u8; 16];
        let len = put_name(&mut out, &[0x61, 0xD800, 0x62, 0xE9]).unwrap();
        assert_eq!(&out[..len], "/a\u{FFFD}b\u{E9}".as_bytes());
        assert_eq!(put_name(&mut out[..4], &[0x61, 0xD800]), None);
        assert_eq!(put_name(&mut [], &[]), None);
    }
}
