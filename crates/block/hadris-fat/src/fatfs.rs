use core::fmt;

use hadris_common::types::endian::Endian;
use hadris_fs::{
    Attributes, Capabilities, CaseSensitivity, Clock, DateTime, DirCursor, DirEntry, Error,
    ErrorKind, FileTimes, FileType, FixedTable, FsResult, FsStats, Metadata, MountError, Name,
    NameBuf, NameCharset, NameError, NewNode, NoClock, NodeId, NodeTable, RenameFlags, SetMetadata,
};
use hadris_storage::BlockIndex;

use super::storage::BlockDevice;
use crate::code_page::{Ascii, CodePage};
use crate::codec::boot::{self, BootError, Geometry, RootDir};
use crate::codec::dirent::{self, ENTRY_SIZE, ShortEntry, Slot};
use crate::codec::entry::FIRST_DATA_CLUSTER;
use crate::codec::lfn::{self, Assembler, Encoded};
use crate::codec::{date, name as names, short_name};
use crate::raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};
use crate::{FatKind, MountOptions, VolumeLabel};

#[path = "check.rs"]
mod fsck;
pub use fsck::{check, check_with};

const ROOT: NodeId = NodeId::new(1);
/// Unpinned ids from here up name the entry at `(id - MOVED_IDS) * 32`. A
/// listing hands them out when a pinned node that has moved holds the
/// natural id.
const MOVED_IDS: u64 = 1 << 62;
/// Ids from here up are handed out when a node's natural id is taken.
const FALLBACK_IDS: u64 = 1 << 63;
/// The largest device block [`FatFs`] can buffer.
pub(super) const MAX_BLOCK_SIZE: usize = 4096;
const BOOT_SECTOR_LEN: usize = 512;
const BPB_LEN: usize = size_of::<RawBpb>();
const FAT32_MIRRORING_DISABLED: u16 = 0x80;
const FAT32_ACTIVE_FAT: u16 = 0x0F;
/// Offset of `FSI_Free_Count` in the FSInfo sector; `FSI_Nxt_Free` follows.
const FSINFO_FREE_COUNT: u64 = 488;
const UNKNOWN_FREE: u32 = u32::MAX;
const MAX_FILE_SIZE: u64 = u32::MAX as u64;
/// The attribute bits [`Attributes`] maps to.
const ATTR_MAPPED: [(u8, Attributes); 4] = [
    (dirent::ATTR_READ_ONLY, Attributes::READ_ONLY),
    (dirent::ATTR_HIDDEN, Attributes::HIDDEN),
    (dirent::ATTR_SYSTEM, Attributes::SYSTEM),
    (dirent::ATTR_ARCHIVE, Attributes::ARCHIVE),
];

fn corrupt(_: BootError) -> ErrorKind {
    ErrorKind::Corrupt
}

/// State of a pinned node.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// Byte offset of the node's short entry on the volume.
    entry: u64,
    first: u32,
    size: u32,
    dir: bool,
    /// A known `(index, cluster)` pair of the chain, so sequential reads do
    /// not walk it from the start. Cluster 0 when unknown.
    hint: (u32, u32),
    /// The directory entry lacks the size and modification time. A dirty
    /// node holds one pin of the driver's own until it is written.
    dirty: bool,
}

impl Node {
    fn new(entry: u64, short: &ShortEntry, kind: FatKind) -> Self {
        Self {
            entry,
            first: short.first_cluster(kind),
            size: short.size,
            dir: short.is_dir(),
            hint: (0, 0),
            dirty: false,
        }
    }
}

/// Where a directory's entries live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirStart {
    /// The FAT12/16 root: `slots` entries from byte `start`.
    Fixed { start: u64, slots: u32 },
    /// A cluster chain starting at the given cluster.
    Chain(u32),
}

/// A position in a directory's cluster chain, kept across slots so a scan
/// walks the chain once.
struct Walk {
    start: DirStart,
    index: u32,
    cluster: u32,
}

impl Walk {
    fn new(start: DirStart) -> Self {
        Self {
            start,
            index: 0,
            cluster: 0,
        }
    }
}

/// A visible short entry found by a directory scan.
struct Found {
    slot: u32,
    offset: u64,
    entry: ShortEntry,
    /// The slot of the last long-name fragment that started a name before
    /// this entry, meaningful only when the name assembled.
    long_start: u32,
}

/// A named entry: its short entry and the slots its name occupies.
struct Located {
    /// The first slot of the entry, its first long-name fragment if it has a
    /// valid long name.
    first: u32,
    slot: u32,
    offset: u64,
    entry: ShortEntry,
    /// Whether the query equals the entry's name exactly.
    exact: bool,
}

/// A name about to be written: its long-name entries and short-name
/// candidates.
struct NewName {
    encoded: Encoded,
    /// Candidates in on-disk form for tails none, `~1` to `~4`.
    candidates: [[u8; 11]; 5],
    /// The name is its own short name up to case.
    lossless: bool,
    /// Set when the name can be stored as a short entry alone with these
    /// `DIR_NTRes` case bits.
    case_bits: Option<u8>,
}

impl NewName {
    fn new(text: &str, code_page: &impl CodePage) -> Result<Self, ErrorKind> {
        if text.encode_utf16().count() > lfn::MAX_UNITS {
            return Err(ErrorKind::NameTooLong);
        }
        if !short_name::is_valid_long_name(text) || text.ends_with(['.', ' ']) {
            return Err(ErrorKind::InvalidInput);
        }
        let encoded = Encoded::new(text).ok_or(ErrorKind::InvalidInput)?;
        let mut candidates = [[0u8; 11]; 5];
        for (suffix, candidate) in candidates.iter_mut().enumerate() {
            if let Some(mut name) =
                short_name::generate(text, suffix as u8, |ch| code_page.encode(ch))
            {
                short_name::to_disk(&mut name);
                *candidate = name;
            }
        }
        let mut shown = [0u8; short_name::DISPLAY_MAX];
        let len = short_name::display(&candidates[0], 0, |byte| code_page.decode(byte), &mut shown);
        let lossless = candidates[0][0] != 0
            && text.is_ascii()
            && text
                .bytes()
                .map(|b| b.to_ascii_uppercase())
                .eq(shown[..len].iter().copied());
        Ok(Self {
            encoded,
            candidates,
            lossless,
            case_bits: short_name::case_bits(text),
        })
    }

    fn short_only(&self) -> bool {
        self.lossless && self.case_bits.is_some()
    }

    /// Directory slots the name needs.
    fn slots(&self) -> u32 {
        if self.short_only() {
            1
        } else {
            self.encoded.entries() as u32 + 1
        }
    }
}

/// Where a new entry goes.
struct Plan {
    /// The first slot of the run.
    start: u32,
    slots: u32,
    /// Clusters to append to the directory first, after `tail`.
    grow: u32,
    tail: u32,
    short: [u8; 11],
    nt_case: u8,
}

/// Entries a new name may take the place of while it is planned.
#[derive(Clone, Copy, Default)]
struct Skip {
    /// A short entry whose name does not count as taken.
    entry: Option<u64>,
    /// Slots, inclusive, that count as free and whose names do not count.
    run: Option<(u32, u32)>,
}

impl Skip {
    fn covers(&self, slot: u32, offset: u64) -> bool {
        self.entry == Some(offset)
            || self
                .run
                .is_some_and(|(first, last)| (first..=last).contains(&slot))
    }
}

/// The raw slots of an entry and its long name, kept to undo its removal.
struct SavedRun {
    first: u32,
    len: u32,
    raw: [[u8; ENTRY_SIZE as usize]; lfn::MAX_ENTRIES + 1],
}

/// Clusters added to a file's chain, to undo on failure.
#[derive(Clone, Copy)]
struct Growth {
    first: u32,
    /// The old last cluster, 0 when the file had none.
    tail: u32,
    /// The first added cluster, 0 when none were added.
    added: u32,
}

/// One device block, the driver's only buffer.
pub(super) struct BlockBuf {
    data: [u8; MAX_BLOCK_SIZE],
    size: usize,
    cached: Option<u64>,
}

impl BlockBuf {
    pub(super) fn new(size: usize) -> Self {
        Self {
            data: [0; MAX_BLOCK_SIZE],
            size,
            cached: None,
        }
    }
}

fn metadata(node: &Node, entry: &ShortEntry) -> Metadata {
    let times = FileTimes::new()
        .with_created(date::decode(
            entry.created_date,
            entry.created_time,
            entry.created_tenths,
        ))
        .with_modified(date::decode(entry.modified_date, entry.modified_time, 0))
        .with_accessed(date::decode(entry.accessed_date, 0, 0));
    let mut attributes = Attributes::empty();
    for (bit, flag) in ATTR_MAPPED {
        if entry.attr & bit != 0 {
            attributes |= flag;
        }
    }
    let (file_type, len) = if node.dir {
        (FileType::Dir, 0)
    } else {
        (FileType::File, node.size as u64)
    };
    Metadata::new(file_type)
        .with_len(len)
        .with_times(times)
        .with_attributes(attributes)
}

/// Whether `query` names the entry, by its long name or its short name,
/// ignoring case.
fn matches(
    query: &str,
    long: Option<&[u16]>,
    entry: &ShortEntry,
    code_page: &impl CodePage,
) -> bool {
    if long.is_some_and(|units| names::eq_ignore_case(query.chars(), names::utf16_chars(units))) {
        return true;
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(
        &entry.name,
        entry.nt_case,
        |byte| code_page.decode(byte),
        &mut short,
    );
    core::str::from_utf8(&short[..len])
        .is_ok_and(|short| names::eq_ignore_case(query.chars(), short.chars()))
}

/// Writes the entry's name into `out`: the long name when it is a valid
/// name, else the short name.
fn write_name(
    out: &mut NameBuf,
    long: Option<&[u16]>,
    entry: &ShortEntry,
    code_page: &impl CodePage,
) -> Result<usize, ErrorKind> {
    if let Some(units) = long
        && out
            .fill(|buf| names::utf16_to_utf8(units, buf).ok_or(NameError::TooLong))
            .is_ok()
    {
        return Ok(out.len());
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(
        &entry.name,
        entry.nt_case,
        |byte| code_page.decode(byte),
        &mut short,
    );
    out.set_bytes(&short[..len]).map_err(|err| match err {
        NameError::TooLong => ErrorKind::LimitExceeded,
        _ => ErrorKind::Corrupt,
    })?;
    Ok(len)
}

/// Whether `query` is exactly the entry's name.
fn is_exact(
    query: &str,
    long: Option<&[u16]>,
    entry: &ShortEntry,
    code_page: &impl CodePage,
) -> bool {
    if let Some(units) = long {
        return names::utf16_chars(units).eq(query.chars());
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(
        &entry.name,
        entry.nt_case,
        |byte| code_page.decode(byte),
        &mut short,
    );
    short[..len] == *query.as_bytes()
}

/// Sets the entry's creation time and the modification and access times.
fn stamp(entry: &mut ShortEntry, created: DateTime, modified: DateTime, accessed: DateTime) {
    (entry.created_date, entry.created_time, entry.created_tenths) = date::encode(created);
    (entry.modified_date, entry.modified_time, _) = date::encode(modified);
    entry.accessed_date = date::encode(accessed).0;
}

fn apply_attributes(entry: &mut ShortEntry, attributes: Attributes) {
    for (bit, flag) in ATTR_MAPPED {
        if attributes.contains(flag) {
            entry.attr |= bit;
        } else {
            entry.attr &= !bit;
        }
    }
}

/// The name a query must be, as a string: not `.` or `..`.
fn entry_name(name: &Name, invalid: ErrorKind) -> Result<&str, ErrorKind> {
    match name.to_str() {
        Ok("." | "..") => Err(ErrorKind::InvalidInput),
        Ok(text) => Ok(text),
        Err(_) => Err(invalid),
    }
}

io_transform! {

async fn load<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, index: u64) -> FsResult<(), D::Error> {
    if block.cached == Some(index) {
        return Ok(());
    }
    block.cached = None;
    dev.read_blocks(BlockIndex(index), &mut block.data[..block.size])
        .await
        .map_err(Error::from_device)?;
    block.cached = Some(index);
    Ok(())
}

/// Reads `out.len()` bytes at byte `offset`. Whole blocks go straight into
/// `out`; partial blocks go through `block`.
async fn read_bytes<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    offset: u64,
    out: &mut [u8],
) -> FsResult<(), D::Error> {
    let size = block.size;
    let mut done = 0;
    while done < out.len() {
        let pos = offset + done as u64;
        let index = pos / size as u64;
        let at = (pos % size as u64) as usize;
        let whole = (out.len() - done) / size * size;
        if at == 0 && whole > 0 {
            dev.read_blocks(BlockIndex(index), &mut out[done..done + whole])
                .await
                .map_err(Error::from_device)?;
            done += whole;
            continue;
        }
        load(dev, block, index).await?;
        let n = (size - at).min(out.len() - done);
        out[done..done + n].copy_from_slice(&block.data[at..at + n]);
        done += n;
    }
    Ok(())
}

/// Writes `len` bytes at byte `offset`: from `data`, or zeros when `data` is
/// `None`. Whole blocks of `data` go straight to the device, whole blocks of
/// zeros go from `block` as many at once as it holds, and partial blocks
/// are read, patched and written through `block`, which is left holding the
/// device's copy or nothing.
pub(super) async fn write_bytes<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    offset: u64,
    data: Option<&[u8]>,
    len: usize,
) -> FsResult<(), D::Error> {
    let size = block.size;
    let mut done = 0;
    while done < len {
        let pos = offset + done as u64;
        let index = pos / size as u64;
        let at = (pos % size as u64) as usize;
        let whole = (len - done) / size * size;
        if data.is_none() && at == 0 && whole > 0 {
            let chunk = whole.min(MAX_BLOCK_SIZE / size * size);
            block.cached = None;
            block.data[..chunk].fill(0);
            dev.write_blocks(BlockIndex(index), &block.data[..chunk]).await?;
            done += chunk;
            continue;
        }
        if let Some(data) = data
            && at == 0
            && whole > 0
        {
            let blocks = index..index + (whole / size) as u64;
            if block.cached.is_some_and(|cached| blocks.contains(&cached)) {
                block.cached = None;
            }
            dev.write_blocks(BlockIndex(index), &data[done..done + whole]).await?;
            done += whole;
            continue;
        }
        let n = (size - at).min(len - done);
        if n < size {
            load(dev, block, index).await?;
        }
        block.cached = None;
        match data {
            Some(data) => block.data[at..at + n].copy_from_slice(&data[done..done + n]),
            None => block.data[at..at + n].fill(0),
        }
        dev.write_blocks(BlockIndex(index), &block.data[..size]).await?;
        block.cached = Some(index);
        done += n;
    }
    Ok(())
}

/// A FAT12, FAT16 or FAT32 volume on a block device.
///
/// `FatFs` is the V3 driver: every node method takes `&mut self`, holds no
/// lock, and needs no allocator. Share it through `hadris_fs` `Volume`, or
/// call the node methods directly; they have the same names and signatures
/// as the `FsDriver` methods, which forward to them.
///
/// Mount with [`open`](FatFs::open), or with [`open_with`](FatFs::open_with)
/// and [`MountOptions`] to choose the type parameters:
///
/// - `T`, the node table. Nodes are identified by the location of their
///   directory entry. `lookup`, `create` and `parent` pin the node they
///   return in the table, and `forget` unpins it. A pinned node keeps its
///   id across `rename`. A full table makes `lookup` and `create` fail with
///   [`ErrorKind::LimitExceeded`] before anything is written; name
///   `HeapTable` or a larger `FixedTable<N>` for more open nodes. Ids from
///   `read_dir_entry` are not pinned and stay valid until that directory
///   changes.
/// - `C`, the [`Clock`] that stamps created and modified entries. The
///   default [`NoClock`] writes 1980-01-01, so images are reproducible;
///   `SystemClock` with `std` writes the current UTC time.
/// - `P`, the [`CodePage`] of short names. The default [`Ascii`] reads
///   short-name bytes above `0x7F` as U+FFFD and generates `_` for
///   non-ASCII characters; `Cp437` maps them.
///
/// Long names are always read and written. Names compare
/// case-insensitively, by the long name or by the short name. A new name that is a valid
/// 8.3 name, in one case per part, is stored as a short entry alone;
/// others get long-name entries and a generated short name with a `~N`
/// tail, as Windows and Linux do.
///
/// The driver keeps one device block, at most 4096 bytes, inline as its
/// buffer, so a `FatFs` is a little over 4 KiB plus the node table. Reads
/// and writes of whole blocks go straight between the device and the
/// caller's buffer. Devices with blocks larger than 4096 bytes are rejected
/// with [`ErrorKind::Unsupported`]; the device block may be smaller or
/// larger than the FAT sector.
///
/// # Writing
///
/// Writes go to the device at once; the driver caches no data. What it
/// defers is the size and modification time in the directory entry of a
/// pinned file: `write_at` and growing `set_len` keep them in the node table,
/// so every handle on the node sees one size, and `sync_node` or `sync`
/// writes them. Until then the node stays in the table under a pin of the
/// driver's own, even after the last `forget`, and counts towards
/// [`open_nodes`](Self::open_nodes) and the table's capacity. Writes through
/// an unpinned id, shrinking `set_len`, and a file's first cluster are
/// written to the entry at once. `sync` also writes the FAT32 FSInfo free
/// count and flushes the device.
///
/// A device that answers a write with `WriteError::ReadOnly` fails that
/// operation with [`ErrorKind::ReadOnly`] and changes nothing, and the
/// volume is read-only from then on. Other failed operations are undone as
/// far as the device allows.
///
/// # Crash and cancellation safety
///
/// Each operation orders its writes so that stopping between any two of
/// them leaves a volume that `fsck` repairs without losing data that was on
/// it before. The same holds when an `async` future is dropped before it
/// completes, and the driver's own state stays consistent with the disk:
///
/// - Clusters are marked in the FAT before anything points to them, and
///   freed only after nothing does. An interrupted operation can leave lost
///   clusters, never a cross-linked or dangling chain.
/// - `write_at` and `set_len` link new clusters before they publish the new
///   size, so an interrupted write leaves a chain longer than the size. A
///   shrinking `set_len` writes the new size before it frees clusters.
/// - `create` prepares a new directory's cluster, then writes the name
///   entries, the short entry last. `remove` clears the short entry first.
/// - `rename` writes the new entry, then the moved directory's `..`, then
///   clears the old entry, so an interruption can leave the node under both
///   names.
/// - FAT copies are written active copy first; the FSInfo free count is a
///   hint and is written by `sync`.
///
/// Data written by an interrupted `write_at` may be partly on disk.
pub struct FatFs<D, T: NodeTable = FixedTable<64>, C: Clock = NoClock, P: CodePage = Ascii> {
    dev: D,
    geo: Geometry,
    active_fat: u8,
    /// Every FAT copy is written, not only the active one.
    mirrored: bool,
    data_end: u64,
    nodes: T::With<Node>,
    next_id: u64,
    block: BlockBuf,
    free_clusters: Option<u32>,
    /// Byte offset of a valid FAT32 FSInfo sector.
    fs_info: Option<u64>,
    fs_info_dirty: bool,
    /// Where the search for a free cluster starts.
    next_free: u32,
    read_only: bool,
    clock: C,
    code_page: P,
}

/// The state a mount reads from the boot and FSInfo sectors.
struct Mount {
    geo: Geometry,
    active_fat: u8,
    mirrored: bool,
    data_end: u64,
    block: BlockBuf,
    free_clusters: Option<u32>,
    fs_info: Option<u64>,
    next_free: u32,
}

impl Mount {
    async fn read<D: BlockDevice>(dev: &mut D) -> FsResult<Self, D::Error> {
        let size = dev.block_size().get() as usize;
        if size > MAX_BLOCK_SIZE {
            return Err(ErrorKind::Unsupported.into());
        }
        let mut block = BlockBuf::new(size);
        let mut sector = [0u8; BOOT_SECTOR_LEN];
        read_bytes(dev, &mut block, 0, &mut sector).await?;
        let bpb: RawBpb = bytemuck::pod_read_unaligned(&sector[..BPB_LEN]);
        boot::check_bpb(&bpb).map_err(corrupt)?;
        let (geo, active_fat, mirrored, fs_info) = if boot::is_fat32(&bpb) {
            let ext: RawBpbExt32 =
                bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt32>()]);
            boot::check_ext32(&bpb, &ext).map_err(corrupt)?;
            let geo = boot::geometry32(&bpb, &ext).map_err(corrupt)?;
            let flags = u16::from_le_bytes(ext.ext_flags);
            let mirrored = flags & FAT32_MIRRORING_DISABLED == 0;
            let active = if mirrored { 0 } else { (flags & FAT32_ACTIVE_FAT) as u8 };
            (geo, active, mirrored, Some(ext.fs_info_sector.get()))
        } else {
            let ext: RawBpbExt16 =
                bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt16>()]);
            boot::check_ext16(&bpb, &ext).map_err(corrupt)?;
            (boot::geometry16(&bpb).map_err(corrupt)?, 0, true, None)
        };
        if geo.max_cluster < FIRST_DATA_CLUSTER
            || active_fat >= geo.fat_count
            || geo.kind.entry_offset(geo.max_cluster as u64) + geo.kind.entry_len() as u64
                > geo.fat_size
        {
            return Err(ErrorKind::Corrupt.into());
        }
        let data_end = geo.data_start + (geo.max_cluster - 1) as u64 * geo.cluster_size as u64;
        let device_len = dev.block_count().saturating_mul(size as u64);
        if data_end > device_len {
            return Err(ErrorKind::Corrupt.into());
        }
        let mut free_clusters = None;
        let mut next_free = FIRST_DATA_CLUSTER;
        let fs_info = match fs_info {
            Some(at) if at != 0 && at < bpb.reserved_sector_count.get() => {
                let mut sector = [0u8; BOOT_SECTOR_LEN];
                let offset = at as u64 * geo.sector_size as u64;
                read_bytes(dev, &mut block, offset, &mut sector).await?;
                let info: RawFsInfo = bytemuck::pod_read_unaligned(&sector);
                let valid = boot::check_fs_info(&info).is_ok();
                let free = info.free_count.get();
                let hint = info.next_free.get();
                if valid && free < geo.max_cluster {
                    free_clusters = Some(free);
                }
                if valid && (FIRST_DATA_CLUSTER..=geo.max_cluster).contains(&hint) {
                    next_free = hint;
                }
                valid.then_some(offset)
            }
            _ => None,
        };
        Ok(Self {
            geo,
            active_fat,
            mirrored,
            data_end,
            block,
            free_clusters,
            fs_info,
            next_free,
        })
    }
}

impl<D, T: NodeTable, C: Clock, P: CodePage> fmt::Debug for FatFs<D, T, C, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FatFs")
            .field("kind", &self.geo.kind)
            .field("cluster_size", &self.geo.cluster_size)
            .field("open_nodes", &(self.nodes.len() + 1))
            .field("free_clusters", &self.free_clusters)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl<D: BlockDevice> FatFs<D> {
    /// Mounts the volume on `dev` with the defaults of [`MountOptions::new`]:
    /// writable, a `FixedTable<64>`, [`NoClock`] and [`Ascii`].
    ///
    /// Fails with [`ErrorKind::Corrupt`] when the boot sector is not a valid
    /// FAT12, FAT16 or FAT32 boot sector or describes a volume larger than
    /// the device, and with [`ErrorKind::Unsupported`] when the device's
    /// blocks are larger than 4096 bytes. The [`MountError`] gives `dev`
    /// back.
    pub async fn open(dev: D) -> Result<Self, MountError<D, D::Error>> {
        Self::open_with(dev, MountOptions::new()).await
    }
}

impl<D: BlockDevice, T: NodeTable, C: Clock, P: CodePage> FatFs<D, T, C, P> {
    /// Mounts the volume on `dev` with `options`, which set the node table,
    /// clock and code page types. Fails as [`open`](FatFs::open) does.
    pub async fn open_with(
        mut dev: D,
        options: MountOptions<T, C, P>,
    ) -> Result<Self, MountError<D, D::Error>> {
        let MountOptions { read_only, table, clock, code_page } = options;
        let mount = match Mount::read(&mut dev).await {
            Ok(mount) => mount,
            Err(error) => return Err(MountError::new(error, dev)),
        };
        Ok(Self {
            dev,
            geo: mount.geo,
            active_fat: mount.active_fat,
            mirrored: mount.mirrored,
            data_end: mount.data_end,
            nodes: table.empty(),
            next_id: FALLBACK_IDS,
            block: mount.block,
            free_clusters: mount.free_clusters,
            fs_info: mount.fs_info,
            fs_info_dirty: false,
            next_free: mount.next_free,
            read_only,
            clock,
            code_page,
        })
    }

    /// Returns the device.
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// Whether the volume was mounted with
    /// [`MountOptions::with_read_only`], or the device has refused a write
    /// since.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The clock that stamps new and modified entries.
    pub fn clock(&self) -> &C {
        &self.clock
    }

    /// The code page of short names.
    pub fn code_page(&self) -> &P {
        &self.code_page
    }

    /// The FAT variant of the volume.
    pub fn kind(&self) -> FatKind {
        self.geo.kind
    }

    /// The volume label: the label entry of the root directory, or `None`
    /// when it has none. The copy in the boot sector is not read.
    pub async fn label(&mut self) -> FsResult<Option<VolumeLabel>, D::Error> {
        let mut walk = Walk::new(self.root_start());
        let mut slot = 0;
        while let Some(offset) = self.slot_offset(&mut walk, slot).await? {
            match self.read_slot(offset).await? {
                Slot::End => break,
                Slot::Short(entry) if entry.is_label() => {
                    return Ok(Some(VolumeLabel::from_disk(entry.name)));
                }
                _ => {}
            }
            slot += 1;
        }
        Ok(None)
    }

    /// Number of nodes in the node table, plus one for the root, which is
    /// always pinned. Nodes whose size is not yet written count until
    /// `sync_node` or `sync`.
    pub fn open_nodes(&self) -> usize {
        self.nodes.len() + 1
    }

    /// What this volume supports: case-insensitive, case-preserving UTF-16
    /// names of up to 255 code units (765 bytes of UTF-8) and two-second
    /// modification times. Writable unless [`is_read_only`](Self::is_read_only).
    pub fn capabilities(&self) -> Capabilities {
        let caps = Capabilities::new()
            .with_case_sensitivity(CaseSensitivity::InsensitivePreserving)
            .with_name_charset(NameCharset::Utf16)
            .with_max_name_len(lfn::MAX_UNITS * 3)
            .with_timestamp_resolution_ns(2_000_000_000);
        if self.read_only { caps } else { caps.with_writable() }
    }

    /// The root directory. Always pinned.
    pub fn root(&self) -> NodeId {
        ROOT
    }

    /// Finds `name` in `dir` and pins the result. Case is ignored, and the
    /// short name of an entry with a long name matches too.
    pub async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        let start = self.dir_start(dir).await?;
        let Ok(query) = name.to_str() else {
            return Err(ErrorKind::NotFound.into());
        };
        let found = self.find_entry(start, query).await?.ok_or(ErrorKind::NotFound)?;
        Ok(self.intern(found.offset, &found.entry)?)
    }

    /// Metadata of a pinned node or of an id just returned by
    /// `read_dir_entry`. Directories have length 0.
    pub async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        if node == ROOT {
            return Ok(Metadata::new(FileType::Dir));
        }
        let (state, entry) = self.node_entry(node).await?;
        Ok(metadata(&state, &entry))
    }

    /// Writes the entry after `cursor` into `name` and advances `cursor`.
    /// `None` at the end. `.`, `..`, the volume label and deleted entries
    /// are skipped. The raw cursor is the index of the next directory slot.
    pub async fn read_dir_entry(
        &mut self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, D::Error> {
        let start = self.dir_start(dir).await?;
        let Ok(mut slot) = u32::try_from(cursor.into_raw()) else {
            return Ok(None);
        };
        let mut walk = Walk::new(start);
        let mut long = Assembler::new();
        let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? else {
            *cursor = DirCursor::from_raw(slot as u64);
            return Ok(None);
        };
        let units = long.finish(lfn::checksum(&found.entry.name));
        let len = write_name(name, units.filter(|units| !units.is_empty()), &found.entry, &self.code_page)?;
        let file_type = if found.entry.is_dir() { FileType::Dir } else { FileType::File };
        let node = self.id_at(found.offset);
        *cursor = DirCursor::from_raw(found.slot as u64 + 1);
        Ok(Some(DirEntry::new(node, file_type, len)))
    }

    /// Reads from a file at `offset`. Returns 0 at or past the end.
    pub async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        if node == ROOT {
            return Err(ErrorKind::IsADirectory.into());
        }
        let state = self.node(node).await?;
        if state.dir {
            return Err(ErrorKind::IsADirectory.into());
        }
        let size = state.size as u64;
        if offset >= size || buf.is_empty() {
            return Ok(0);
        }
        let count = (size - offset).min(buf.len() as u64) as usize;
        let cluster_size = self.geo.cluster_size as u64;
        let (mut index, mut cluster) =
            if state.hint.1 != 0 && state.hint.0 as u64 <= offset / cluster_size {
                state.hint
            } else {
                (0, self.check_cluster(state.first)?)
            };
        let mut done = 0;
        while done < count {
            let pos = offset + done as u64;
            let want = (pos / cluster_size) as u32;
            while index < want {
                cluster = self.next_cluster(cluster).await?.ok_or(ErrorKind::Corrupt)?;
                index += 1;
            }
            let within = pos % cluster_size;
            let n = ((cluster_size - within) as usize).min(count - done);
            let at = self.cluster_at(cluster)? + within;
            read_bytes(&mut self.dev, &mut self.block, at, &mut buf[done..done + n]).await?;
            done += n;
        }
        if let Some(state) = self.nodes.get_mut(node) {
            state.hint = (index, cluster);
        }
        Ok(count)
    }

    /// The directory containing `dir`, pinned. The root is its own parent.
    ///
    /// Found through `..` entries: the parent's first cluster, then the
    /// grandparent, which is scanned for the parent's entry.
    pub async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        if dir == ROOT {
            return Ok(ROOT);
        }
        let DirStart::Chain(first) = self.dir_start(dir).await? else {
            return Ok(ROOT);
        };
        let Some(up) = self.dot_dot(first).await? else {
            return Ok(ROOT);
        };
        let grand = match self.dot_dot(up).await? {
            Some(cluster) => DirStart::Chain(cluster),
            None => self.root_start(),
        };
        let mut walk = Walk::new(grand);
        let mut slot = 0;
        let mut long = Assembler::new();
        while let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? {
            if found.entry.is_dir() && found.entry.first_cluster(self.geo.kind) == up {
                return Ok(self.intern(found.offset, &found.entry)?);
            }
        }
        Err(ErrorKind::Corrupt.into())
    }

    /// Space usage in clusters. Free space comes from the FAT32 FSInfo
    /// sector when it is valid, and otherwise from one scan of the FAT,
    /// which is then kept. The FSInfo count is a hint: allocation scans the
    /// FAT regardless, and one that finds it wrong drops it.
    pub async fn stats(&mut self) -> FsResult<FsStats, D::Error> {
        let free = match self.free_clusters {
            Some(free) => free,
            None => {
                let mut free = 0;
                for cluster in FIRST_DATA_CLUSTER..=self.geo.max_cluster {
                    if self.fat_entry(cluster).await? & self.geo.kind.mask() == 0 {
                        free += 1;
                    }
                }
                self.free_clusters = Some(free);
                free
            }
        };
        let total = self.geo.max_cluster - 1;
        Ok(FsStats::new(total as u64, free as u64, self.geo.cluster_size))
    }

    /// Passes the clusters of the chain of `node` to `visit` in order and
    /// returns how many there were. An empty file and the FAT12/16 root
    /// directory have none. Fails with [`ErrorKind::Corrupt`] when the chain
    /// leaves the data clusters, runs into a bad cluster or is longer than
    /// the volume.
    pub async fn cluster_chain(
        &mut self,
        node: NodeId,
        mut visit: impl FnMut(u32),
    ) -> FsResult<u32, D::Error> {
        let first = if node == ROOT {
            match self.geo.root {
                RootDir::Fixed { .. } => return Ok(0),
                RootDir::Cluster(cluster) => cluster,
            }
        } else {
            self.node(node).await?.first
        };
        if first == 0 {
            return Ok(0);
        }
        let mut cluster = self.check_cluster(first)?;
        let mut count = 0u32;
        loop {
            visit(cluster);
            count += 1;
            if count >= self.geo.max_cluster {
                return Err(ErrorKind::Corrupt.into());
            }
            match self.next_cluster(cluster).await? {
                Some(next) => cluster = next,
                None => return Ok(count),
            }
        }
    }

    /// Unpins a node. Unknown ids and the root are ignored. A node whose
    /// size is not yet written stays in the table until `sync_node` or
    /// `sync`.
    pub fn forget(&mut self, node: NodeId) {
        if node != ROOT && self.user_pins(node) > 0 {
            self.nodes.unpin(node);
        }
    }

    /// Creates `name` in `dir` and pins the new node. Only files and
    /// directories exist on FAT; other kinds fail with
    /// [`ErrorKind::Unsupported`].
    ///
    /// `meta` sets the attributes, which default to archive for a file, and
    /// the creation, modification and access times, which default to now.
    /// Mode and owner are ignored, as for `set_metadata`. Fails with [`ErrorKind::AlreadyExists`]
    /// when a long or short name matches, [`ErrorKind::InvalidInput`] for a
    /// name FAT cannot hold (control characters, `"*/:<>?\|`, or a trailing
    /// dot or space, refused rather than stripped so a created name is the
    /// name listed), and [`ErrorKind::NoSpace`] when a FAT12/16 root
    /// directory is full or a directory would pass 65536 entries.
    pub async fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        meta: &SetMetadata,
    ) -> FsResult<NodeId, D::Error> {
        self.writable()?;
        let is_dir = match kind {
            NewNode::File => false,
            NewNode::Dir => true,
            _ => return Err(ErrorKind::Unsupported.into()),
        };
        let start = self.dir_start(dir).await?;
        let text = entry_name(name, ErrorKind::InvalidInput)?;
        let new = NewName::new(text, &self.code_page)?;
        let plan = self.plan(start, text, true, &new, Skip::default()).await?;
        let reserved = NodeId::new(self.next_id);
        let placeholder = Node {
            entry: u64::MAX,
            first: 0,
            size: 0,
            dir: is_dir,
            hint: (0, 0),
            dirty: false,
        };
        self.nodes
            .insert(reserved, placeholder)
            .map_err(|_| ErrorKind::LimitExceeded)?;
        let node = match self.create_entry(start, &new, &plan, is_dir, meta).await {
            Ok(node) => node,
            Err(err) => {
                self.nodes.remove(reserved);
                return Err(err);
            }
        };
        self.nodes.remove(reserved);
        if let Some(natural) = self.natural_id(node.entry)
            && self.nodes.insert(natural, node).is_ok()
        {
            return Ok(natural);
        }
        self.nodes
            .insert(reserved, node)
            .map_err(|_| ErrorKind::LimitExceeded)?;
        self.next_id += 1;
        Ok(reserved)
    }

    /// Removes the file or empty directory `name` from `dir` and frees its
    /// clusters.
    ///
    /// Fails with [`ErrorKind::Busy`] while the node is pinned, and with
    /// [`ErrorKind::DirectoryNotEmpty`] for a directory with entries.
    pub async fn remove(&mut self, dir: NodeId, name: &Name) -> FsResult<(), D::Error> {
        self.writable()?;
        let start = self.dir_start(dir).await?;
        let query = entry_name(name, ErrorKind::NotFound)?;
        let found = self.find_entry(start, query).await?.ok_or(ErrorKind::NotFound)?;
        let pinned = self.pinned_at(found.offset);
        if pinned.is_some_and(|id| self.user_pins(id) > 0) {
            return Err(ErrorKind::Busy.into());
        }
        let first = match pinned.and_then(|id| self.nodes.get(id)) {
            Some(node) => node.first,
            None => found.entry.first_cluster(self.geo.kind),
        };
        if found.entry.is_dir() && !self.dir_is_empty(self.check_cluster(first)?).await? {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        self.write(found.offset, &[dirent::FREE]).await?;
        if let Some(id) = pinned {
            self.nodes.remove(id);
        }
        self.clear_slots(start, found.first, found.slot).await?;
        if first != 0 {
            self.free_chain(first).await?;
        }
        Ok(())
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`. The moved node keeps
    /// its `NodeId` when it is pinned. A renamed file gets the archive
    /// attribute, as the FAT specification and Windows do; a directory
    /// keeps its attributes.
    ///
    /// An existing `to` is replaced unless `flags` has
    /// [`RenameFlags::NO_REPLACE`] ([`ErrorKind::AlreadyExists`]): a file by
    /// a file, an empty directory by a directory. The result is named `to`
    /// as given, even when `to` matches the target in another case or by its
    /// short alias, and the new name takes the target's slots when it fits
    /// them, so a full FAT12/16 root directory still allows the replace. A
    /// pinned target fails with [`ErrorKind::Busy`].
    /// Moving a directory into itself or below, or a `to` that `create`
    /// would refuse, fails with [`ErrorKind::InvalidInput`], and unknown
    /// flags with
    /// [`ErrorKind::Unsupported`].
    pub async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), D::Error> {
        self.writable()?;
        if flags.bits() & !RenameFlags::NO_REPLACE.bits() != 0 {
            return Err(ErrorKind::Unsupported.into());
        }
        let from_start = self.dir_start(from_dir).await?;
        let to_start = self.dir_start(to_dir).await?;
        let from_text = entry_name(from, ErrorKind::NotFound)?;
        let to_text = entry_name(to, ErrorKind::InvalidInput)?;
        let new = NewName::new(to_text, &self.code_page)?;
        let src = self
            .find_entry(from_start, from_text)
            .await?
            .ok_or(ErrorKind::NotFound)?;
        let src_id = self.pinned_at(src.offset);
        let src_node = match src_id.and_then(|id| self.nodes.get(id)) {
            Some(node) => *node,
            None => Node::new(src.offset, &src.entry, self.geo.kind),
        };
        if src_node.dir {
            self.check_cluster(src_node.first)?;
        }
        if src_node.dir && self.is_within(to_start, src_node.first).await? {
            return Err(ErrorKind::InvalidInput.into());
        }
        let mut moved = src.entry;
        moved.set_first_cluster(self.geo.kind, src_node.first);
        if !src_node.dir {
            moved.size = src_node.size;
            moved.attr |= dirent::ATTR_ARCHIVE;
        }
        let dot_dot = (src_node.dir && self.parent_cluster(from_start) != self.parent_cluster(to_start))
            .then(|| (src_node.first, self.parent_cluster(from_start), self.parent_cluster(to_start)));
        match self.find_entry(to_start, to_text).await? {
            Some(target) if target.offset == src.offset && target.exact => return Ok(()),
            Some(target) if target.offset != src.offset => {
                if flags.contains(RenameFlags::NO_REPLACE) {
                    return Err(ErrorKind::AlreadyExists.into());
                }
                return self
                    .replace_entry(from_start, &src, to_start, &target, to_text, &new, moved, dot_dot, src_id)
                    .await;
            }
            _ => {
                let skip = Skip {
                    entry: (from_start == to_start).then_some(src.offset),
                    run: None,
                };
                let plan = self.plan(to_start, to_text, false, &new, skip).await?;
                self.move_entry(&src, to_start, &new, &plan, moved, dot_dot, src_id, None)
                    .await?;
            }
        }
        self.clear_slots(from_start, src.first, src.slot).await
    }

    /// Writes to a file at `offset`, growing it and zero-filling any gap
    /// past the old end. Returns the bytes written, fewer than `buf.len()`
    /// only at the 4 GiB - 1 FAT size limit; an `offset` at or past it fails
    /// with [`ErrorKind::FileTooLarge`], and a volume without room for the
    /// new clusters fails with [`ErrorKind::NoSpace`] and changes nothing.
    ///
    /// The new size of a pinned file is written by `sync_node` or `sync`,
    /// with the modification time and the archive attribute.
    pub async fn write_at(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, D::Error> {
        self.writable()?;
        let (id, state) = self.file_node(node).await?;
        if buf.is_empty() {
            return Ok(0);
        }
        if offset >= MAX_FILE_SIZE {
            return Err(ErrorKind::FileTooLarge.into());
        }
        let count = (MAX_FILE_SIZE - offset).min(buf.len() as u64) as usize;
        let end = offset + count as u64;
        let growth = self.cover(&state, end).await?;
        let old = state.size as u64;
        let hint = if growth.first == state.first { state.hint } else { (0, 0) };
        let filled = if offset > old {
            self.fill(growth.first, hint, old, None, (offset - old) as usize).await
        } else {
            Ok(hint)
        };
        let written = match filled {
            Ok(hint) => self.fill(growth.first, hint, offset, Some(&buf[..count]), count).await,
            Err(err) => Err(err),
        };
        let hint = match written {
            Ok(hint) => hint,
            Err(err) => {
                self.undo_growth(growth).await;
                return Err(err);
            }
        };
        let size = old.max(end) as u32;
        if let Err(err) = self.publish(id, &state, growth.first, size, hint).await {
            self.undo_growth(growth).await;
            return Err(err);
        }
        Ok(count)
    }

    /// Truncates or extends a file. Growth reads as zeros. Shrinking writes
    /// the new size to the directory entry at once and frees the clusters
    /// past it. A changed size sets the archive attribute; the same size
    /// changes nothing. A length past 4 GiB - 1 fails with
    /// [`ErrorKind::FileTooLarge`].
    pub async fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), D::Error> {
        self.writable()?;
        let (id, state) = self.file_node(node).await?;
        if len > MAX_FILE_SIZE {
            return Err(ErrorKind::FileTooLarge.into());
        }
        let old = state.size as u64;
        if len > old {
            let growth = self.cover(&state, len).await?;
            let hint = if growth.first == state.first { state.hint } else { (0, 0) };
            let published = match self.fill(growth.first, hint, old, None, (len - old) as usize).await {
                Ok(hint) => self.publish(id, &state, growth.first, len as u32, hint).await,
                Err(err) => Err(err),
            };
            if let Err(err) = published {
                self.undo_growth(growth).await;
                return Err(err);
            }
            return Ok(());
        }
        if len == old {
            return Ok(());
        }
        let keep = len.div_ceil(self.geo.cluster_size as u64) as u32;
        let first = if keep == 0 { 0 } else { state.first };
        self.store(id, &state, first, len as u32, (0, 0)).await?;
        if state.first == 0 {
            return Ok(());
        }
        if keep == 0 {
            return self.free_chain(state.first).await;
        }
        let (index, last) = self.walk(state.first, (0, 0), keep - 1).await?;
        if index == keep - 1
            && let Some(next) = self.next_cluster(last).await?
        {
            self.set_fat(last, self.geo.kind.end_of_chain()).await?;
            self.free_chain(next).await?;
        }
        Ok(())
    }

    /// Changes attributes and times. `changed` times, mode and owner are
    /// ignored, since FAT cannot store them, as are changes to the root. A
    /// pending size is written too.
    ///
    /// Ignoring them is the `FsDriver` contract, which `copy_tree` and
    /// `import_from_host` rely on when they copy a mode onto FAT;
    /// [`capabilities`](Self::capabilities) reports neither permissions nor
    /// owners, so a caller that needs them can check first.
    pub async fn set_metadata(&mut self, node: NodeId, changes: &SetMetadata) -> FsResult<(), D::Error> {
        self.writable()?;
        if node == ROOT {
            return Ok(());
        }
        let (id, state) = self.any_node(node).await?;
        let times = changes.times();
        if !state.dirty
            && changes.attributes().is_none()
            && times.created().is_none()
            && times.modified().is_none()
            && times.accessed().is_none()
        {
            return Ok(());
        }
        let mut entry = self.read_short(state.entry).await?;
        if state.dirty {
            self.touch(&mut entry, &state);
        }
        if let Some(attributes) = changes.attributes() {
            apply_attributes(&mut entry, attributes);
        }
        if let Some(time) = times.created() {
            (entry.created_date, entry.created_time, entry.created_tenths) = date::encode(time);
        }
        if let Some(time) = times.modified() {
            (entry.modified_date, entry.modified_time, _) = date::encode(time);
        }
        if let Some(time) = times.accessed() {
            entry.accessed_date = date::encode(time).0;
        }
        self.write(state.entry, &entry.encode()).await?;
        if let Some(id) = id {
            self.clean(id);
        }
        Ok(())
    }

    /// Writes the node's pending size and modification time, then flushes
    /// the device.
    pub async fn sync_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        if node != ROOT {
            let (id, _) = self.any_node(node).await?;
            if let Some(id) = id {
                self.flush_node(id).await?;
            }
        }
        self.flush_device().await
    }

    /// Writes every pending size and modification time and the FAT32
    /// FSInfo free count, then flushes the device.
    pub async fn sync(&mut self) -> FsResult<(), D::Error> {
        while let Some(id) = self.nodes.find(&mut |_, node| node.dirty) {
            self.flush_node(id).await?;
        }
        if self.fs_info_dirty
            && let Some(at) = self.fs_info
        {
            let mut info = [0u8; 8];
            info[..4].copy_from_slice(&self.free_clusters.unwrap_or(UNKNOWN_FREE).to_le_bytes());
            info[4..].copy_from_slice(&self.next_free.to_le_bytes());
            self.write(at + FSINFO_FREE_COUNT, &info).await?;
            self.fs_info_dirty = false;
        }
        self.flush_device().await
    }

    fn now(&self) -> DateTime {
        self.clock.now()
    }

    fn writable(&self) -> Result<(), ErrorKind> {
        if self.read_only { Err(ErrorKind::ReadOnly) } else { Ok(()) }
    }

    /// Pins held by callers, not counting the driver's own pin on a dirty
    /// node.
    fn user_pins(&self, id: NodeId) -> u32 {
        let dirty = self.nodes.get(id).is_some_and(|node| node.dirty);
        self.nodes.pins(id).saturating_sub(dirty as u32)
    }

    fn mark_dirty(&mut self, id: NodeId) {
        let fresh = match self.nodes.get_mut(id) {
            Some(node) if !node.dirty => {
                node.dirty = true;
                true
            }
            _ => false,
        };
        if fresh {
            self.nodes.pin(id);
        }
    }

    fn clean(&mut self, id: NodeId) {
        let was_dirty = match self.nodes.get_mut(id) {
            Some(node) if node.dirty => {
                node.dirty = false;
                true
            }
            _ => false,
        };
        if was_dirty {
            self.nodes.unpin(id);
        }
    }

    /// The parent cluster a `..` entry records for a child of `dir`.
    fn parent_cluster(&self, dir: DirStart) -> u32 {
        match dir {
            DirStart::Chain(cluster) if self.geo.root != RootDir::Cluster(cluster) => cluster,
            _ => 0,
        }
    }

    async fn put(&mut self, offset: u64, data: Option<&[u8]>, len: usize) -> FsResult<(), D::Error> {
        let result = write_bytes(&mut self.dev, &mut self.block, offset, data, len).await;
        if let Err(err) = &result
            && err.kind() == ErrorKind::ReadOnly
        {
            self.read_only = true;
        }
        result
    }

    async fn write(&mut self, offset: u64, data: &[u8]) -> FsResult<(), D::Error> {
        self.put(offset, Some(data), data.len()).await
    }

    async fn flush_device(&mut self) -> FsResult<(), D::Error> {
        if self.read_only {
            return Ok(());
        }
        let result = self.dev.flush().await.map_err(Error::from);
        if let Err(err) = &result
            && err.kind() == ErrorKind::ReadOnly
        {
            self.read_only = true;
        }
        result
    }

    async fn read_short(&mut self, offset: u64) -> FsResult<ShortEntry, D::Error> {
        match self.read_slot(offset).await? {
            Slot::Short(entry) => Ok(entry),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    /// Records the node's size and first cluster in `entry` and marks it
    /// modified now.
    fn touch(&self, entry: &mut ShortEntry, node: &Node) {
        if !node.dir {
            entry.size = node.size;
        }
        entry.set_first_cluster(self.geo.kind, node.first);
        let (date, time, _) = date::encode(self.now());
        entry.modified_date = date;
        entry.modified_time = time;
        entry.accessed_date = date;
        entry.attr |= dirent::ATTR_ARCHIVE;
    }

    /// Writes a file's first cluster, size and modification time to its
    /// entry and updates its table state.
    async fn store(
        &mut self,
        id: Option<NodeId>,
        state: &Node,
        first: u32,
        size: u32,
        hint: (u32, u32),
    ) -> FsResult<(), D::Error> {
        let next = Node { first, size, hint, ..*state };
        let mut entry = self.read_short(state.entry).await?;
        self.touch(&mut entry, &next);
        self.write(state.entry, &entry.encode()).await?;
        if let Some(id) = id {
            if let Some(node) = self.nodes.get_mut(id) {
                node.first = first;
                node.size = size;
                node.hint = hint;
            }
            self.clean(id);
        }
        Ok(())
    }

    /// Records a file's new size: in the table for a pinned node whose first
    /// cluster is unchanged, else in its entry.
    async fn publish(
        &mut self,
        id: Option<NodeId>,
        state: &Node,
        first: u32,
        size: u32,
        hint: (u32, u32),
    ) -> FsResult<(), D::Error> {
        match id {
            Some(id) if first == state.first => {
                if let Some(node) = self.nodes.get_mut(id) {
                    node.size = size;
                    node.hint = hint;
                }
                self.mark_dirty(id);
                Ok(())
            }
            _ => self.store(id, state, first, size, hint).await,
        }
    }

    async fn flush_node(&mut self, id: NodeId) -> FsResult<(), D::Error> {
        match self.nodes.get(id).copied() {
            Some(node) if node.dirty => {
                self.store(Some(id), &node, node.first, node.size, node.hint)
                    .await
            }
            _ => Ok(()),
        }
    }

    /// The table id and state of a node that is not the root.
    async fn any_node(&mut self, id: NodeId) -> FsResult<(Option<NodeId>, Node), D::Error> {
        if let Some(node) = self.nodes.get(id) {
            return Ok((Some(id), *node));
        }
        let (offset, entry) = self.unpinned(id).await?;
        match self.pinned_at(offset) {
            Some(pinned) => Ok((Some(pinned), *self.nodes.get(pinned).ok_or(ErrorKind::Corrupt)?)),
            None => Ok((None, Node::new(offset, &entry, self.geo.kind))),
        }
    }

    async fn file_node(&mut self, id: NodeId) -> FsResult<(Option<NodeId>, Node), D::Error> {
        if id == ROOT {
            return Err(ErrorKind::IsADirectory.into());
        }
        let (id, node) = self.any_node(id).await?;
        if node.dir {
            return Err(ErrorKind::IsADirectory.into());
        }
        Ok((id, node))
    }

    /// Finds the visible entry named `query`, by long or short name.
    async fn find_entry(&mut self, start: DirStart, query: &str) -> FsResult<Option<Located>, D::Error> {
        let mut walk = Walk::new(start);
        let mut slot = 0;
        let mut long = Assembler::new();
        while let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? {
            let units = long.finish(lfn::checksum(&found.entry.name));
            let named = units.is_some();
            let units = units.filter(|units| !units.is_empty());
            if matches(query, units, &found.entry, &self.code_page) {
                return Ok(Some(Located {
                    first: if named { found.long_start } else { found.slot },
                    slot: found.slot,
                    offset: found.offset,
                    exact: is_exact(query, units, &found.entry, &self.code_page),
                    entry: found.entry,
                }));
            }
        }
        Ok(None)
    }

    async fn dir_is_empty(&mut self, first: u32) -> FsResult<bool, D::Error> {
        let mut walk = Walk::new(DirStart::Chain(first));
        let mut slot = 0;
        let mut long = Assembler::new();
        Ok(self.next_visible(&mut walk, &mut slot, &mut long).await?.is_none())
    }

    /// Whether `dir` is the directory starting at `ancestor` or below it.
    async fn is_within(&mut self, dir: DirStart, ancestor: u32) -> FsResult<bool, D::Error> {
        let DirStart::Chain(mut cluster) = dir else {
            return Ok(false);
        };
        for _ in 0..=self.geo.max_cluster {
            if cluster == ancestor {
                return Ok(true);
            }
            if self.geo.root == RootDir::Cluster(cluster) {
                return Ok(false);
            }
            match self.dot_dot(cluster).await? {
                Some(up) => cluster = up,
                None => return Ok(false),
            }
        }
        Err(ErrorKind::Corrupt.into())
    }

    /// Finds room for `new` in `dir` and picks its short name. With
    /// `check_exists`, an entry matching `text` fails with
    /// [`ErrorKind::AlreadyExists`]. Entries `skip` covers are ignored, and
    /// its run counts as free.
    async fn plan(
        &mut self,
        dir: DirStart,
        text: &str,
        check_exists: bool,
        new: &NewName,
        skip: Skip,
    ) -> FsResult<Plan, D::Error> {
        let needed = new.slots();
        let mut walk = Walk::new(dir);
        let mut long = Assembler::new();
        let mut slot = 0;
        let mut end = false;
        let mut taken = 0u8;
        let (mut run_start, mut run_len) = (0, 0);
        let mut found = None;
        while let Some(offset) = self.slot_offset(&mut walk, slot).await? {
            let reused = skip.run.is_some_and(|(first, last)| (first..=last).contains(&slot));
            if reused {
                long.reset();
            }
            let free = end
                || reused
                || match self.read_slot(offset).await? {
                    Slot::End => {
                        end = true;
                        true
                    }
                    Slot::Free => {
                        long.reset();
                        true
                    }
                    Slot::Long(part) => {
                        long.push(part.sequence, part.checksum, &part.name1, &part.name2, &part.name3);
                        false
                    }
                    Slot::Short(entry) => {
                        let units = long
                            .finish(lfn::checksum(&entry.name))
                            .filter(|units| !units.is_empty());
                        if !skip.covers(slot, offset) {
                            if check_exists && entry.is_visible() && matches(text, units, &entry, &self.code_page) {
                                return Err(ErrorKind::AlreadyExists.into());
                            }
                            for (bit, candidate) in new.candidates.iter().enumerate() {
                                if entry.name == *candidate {
                                    taken |= 1 << bit;
                                }
                            }
                        }
                        false
                    }
                };
            if free {
                if run_len == 0 {
                    run_start = slot;
                }
                run_len += 1;
                if run_len == needed && found.is_none() {
                    found = Some(run_start);
                }
            } else {
                run_len = 0;
            }
            slot += 1;
            if end && found.is_some() {
                break;
            }
        }
        let (start, grow, tail) = match found {
            Some(start) => (start, 0, 0),
            None => {
                let start = if run_len == 0 { slot } else { run_start };
                if matches!(dir, DirStart::Fixed { .. })
                    || start as u64 + needed as u64 > dirent::MAX_ENTRIES as u64
                {
                    return Err(ErrorKind::NoSpace.into());
                }
                let per_cluster = self.geo.cluster_size / ENTRY_SIZE as u32;
                (start, (needed - run_len).div_ceil(per_cluster), walk.cluster)
            }
        };
        let short = if new.lossless && taken & 1 == 0 {
            new.candidates[0]
        } else if new.short_only() {
            return Err(ErrorKind::AlreadyExists.into());
        } else if let Some(bit) = (1..new.candidates.len()).find(|bit| taken & (1 << bit) == 0) {
            new.candidates[bit]
        } else {
            self.hashed_short(dir, text, skip).await?
        };
        Ok(Plan {
            start,
            slots: needed,
            grow,
            tail,
            short,
            nt_case: if new.short_only() { new.case_bits.unwrap_or(0) } else { 0 },
        })
    }

    /// A short name with a hashed `HHHH~N` tail that no entry of `dir` has.
    async fn hashed_short(&mut self, dir: DirStart, text: &str, skip: Skip) -> FsResult<[u8; 11], D::Error> {
        for suffix in 5..=u8::MAX {
            let Some(mut candidate) = short_name::generate(text, suffix, |ch| self.code_page.encode(ch)) else {
                continue;
            };
            short_name::to_disk(&mut candidate);
            if !self.short_taken(dir, &candidate, skip).await? {
                return Ok(candidate);
            }
        }
        Err(ErrorKind::AlreadyExists.into())
    }

    async fn short_taken(&mut self, dir: DirStart, name: &[u8; 11], skip: Skip) -> FsResult<bool, D::Error> {
        let mut walk = Walk::new(dir);
        let mut slot = 0;
        while let Some(offset) = self.slot_offset(&mut walk, slot).await? {
            match self.read_slot(offset).await? {
                Slot::End => break,
                Slot::Short(entry) if entry.name == *name && !skip.covers(slot, offset) => {
                    return Ok(true);
                }
                _ => {}
            }
            slot += 1;
        }
        Ok(false)
    }

    /// Writes `entry` and the long name of `new` into the run `plan` found,
    /// growing the directory first when needed. Returns the short entry's
    /// offset.
    async fn insert_entry(
        &mut self,
        dir: DirStart,
        new: &NewName,
        plan: &Plan,
        entry: &ShortEntry,
    ) -> FsResult<u64, D::Error> {
        let mut grown = 0;
        if plan.grow > 0 {
            grown = self.allocate_chain(plan.grow, true).await?;
            if let Err(err) = self.set_fat(plan.tail, grown).await {
                let _ = self.free_chain(grown).await;
                return Err(err);
            }
        }
        let checksum = lfn::checksum(&entry.name);
        let mut walk = Walk::new(dir);
        let mut offset = 0;
        let mut written = 0;
        let mut failed = None;
        while written < plan.slots {
            let raw = if written + 1 == plan.slots {
                entry.encode()
            } else {
                let (sequence, units) = new.encoded.entry(written as usize);
                dirent::encode_long(sequence, checksum, &units)
            };
            let result = match self.slot_offset(&mut walk, plan.start + written).await {
                Ok(Some(at)) => {
                    offset = at;
                    self.write(at, &raw).await
                }
                Ok(None) => Err(ErrorKind::Corrupt.into()),
                Err(err) => Err(err),
            };
            if let Err(err) = result {
                failed = Some(err);
                break;
            }
            written += 1;
        }
        let Some(err) = failed else {
            return Ok(offset);
        };
        let _ = self.clear_slots(dir, plan.start, plan.start + written).await;
        if grown != 0 && self.set_fat(plan.tail, self.geo.kind.end_of_chain()).await.is_ok() {
            let _ = self.free_chain(grown).await;
        }
        Err(err)
    }

    /// Marks slots `from..to` of `dir` deleted.
    async fn clear_slots(&mut self, dir: DirStart, from: u32, to: u32) -> FsResult<(), D::Error> {
        let mut walk = Walk::new(dir);
        for slot in from..to {
            let at = self.slot_offset(&mut walk, slot).await?.ok_or(ErrorKind::Corrupt)?;
            self.write(at, &[dirent::FREE]).await?;
        }
        Ok(())
    }

    /// Writes a new node's entries: a directory's cluster with `.` and `..`
    /// first, then the name.
    async fn create_entry(
        &mut self,
        dir: DirStart,
        new: &NewName,
        plan: &Plan,
        is_dir: bool,
        meta: &SetMetadata,
    ) -> FsResult<Node, D::Error> {
        let kind = self.geo.kind;
        let now = self.now();
        let times = meta.times();
        let attr = if is_dir { dirent::ATTR_DIRECTORY } else { dirent::ATTR_ARCHIVE };
        let mut entry = ShortEntry::new(plan.short, attr);
        entry.nt_case = plan.nt_case;
        if let Some(attributes) = meta.attributes() {
            apply_attributes(&mut entry, attributes);
        }
        stamp(
            &mut entry,
            times.created().unwrap_or(now),
            times.modified().unwrap_or(now),
            times.accessed().unwrap_or(now),
        );
        let mut first = 0;
        if is_dir {
            first = self.allocate_chain(1, true).await?;
            entry.set_first_cluster(kind, first);
            let mut dot = entry;
            dot.name = *b".          ";
            dot.nt_case = 0;
            dot.attr = dirent::ATTR_DIRECTORY;
            let mut dot_dot = dot;
            dot_dot.name = *b"..         ";
            dot_dot.set_first_cluster(kind, self.parent_cluster(dir));
            let mut raw = [0u8; 2 * ENTRY_SIZE as usize];
            raw[..32].copy_from_slice(&dot.encode());
            raw[32..].copy_from_slice(&dot_dot.encode());
            let written = match self.cluster_at(first) {
                Ok(at) => self.write(at, &raw).await,
                Err(kind) => Err(kind.into()),
            };
            if let Err(err) = written {
                let _ = self.free_chain(first).await;
                return Err(err);
            }
        }
        match self.insert_entry(dir, new, plan, &entry).await {
            Ok(offset) => Ok(Node {
                entry: offset,
                first,
                size: 0,
                dir: is_dir,
                hint: (0, 0),
                dirty: false,
            }),
            Err(err) => {
                if first != 0 {
                    let _ = self.free_chain(first).await;
                }
                Err(err)
            }
        }
    }

    async fn set_dot_dot(&mut self, dir: u32, parent: u32) -> FsResult<(), D::Error> {
        let at = self.cluster_at(dir)? + ENTRY_SIZE;
        let mut entry = self.read_short(at).await?;
        if !entry.is_dot_dot() {
            return Err(ErrorKind::Corrupt.into());
        }
        entry.set_first_cluster(self.geo.kind, parent);
        self.write(at, &entry.encode()).await
    }

    /// Writes `moved` under the name `new` where `plan` found room in `to`,
    /// then frees `src`'s short entry. On failure the new entry is cleared
    /// and `saved` written back. `src`'s long-name slots are left to the
    /// caller.
    #[allow(clippy::too_many_arguments)]
    async fn move_entry(
        &mut self,
        src: &Located,
        to: DirStart,
        new: &NewName,
        plan: &Plan,
        moved: ShortEntry,
        dot_dot: Option<(u32, u32, u32)>,
        src_id: Option<NodeId>,
        saved: Option<&SavedRun>,
    ) -> FsResult<(), D::Error> {
        let mut entry = moved;
        entry.name = plan.short;
        entry.nt_case = plan.nt_case;
        let offset = match self.insert_entry(to, new, plan, &entry).await {
            Ok(offset) => offset,
            Err(err) => {
                self.restore_run(to, saved).await;
                return Err(err);
            }
        };
        let end = plan.start + plan.slots;
        if let Some((dir, _, parent)) = dot_dot
            && let Err(err) = self.set_dot_dot(dir, parent).await
        {
            let _ = self.clear_slots(to, plan.start, end).await;
            self.restore_run(to, saved).await;
            return Err(err);
        }
        if let Err(err) = self.write(src.offset, &[dirent::FREE]).await {
            if let Some((dir, old, _)) = dot_dot {
                let _ = self.set_dot_dot(dir, old).await;
            }
            let _ = self.clear_slots(to, plan.start, end).await;
            self.restore_run(to, saved).await;
            return Err(err);
        }
        if let Some(node) = src_id.and_then(|id| self.nodes.get_mut(id)) {
            node.entry = offset;
        }
        Ok(())
    }

    /// Renames onto an existing entry: `target` and its long name are
    /// cleared and `src` is written under the name `new`, in `target`'s
    /// slots when it fits there, then `target`'s clusters are freed.
    #[allow(clippy::too_many_arguments)]
    async fn replace_entry(
        &mut self,
        from: DirStart,
        src: &Located,
        to: DirStart,
        target: &Located,
        text: &str,
        new: &NewName,
        moved: ShortEntry,
        dot_dot: Option<(u32, u32, u32)>,
        src_id: Option<NodeId>,
    ) -> FsResult<(), D::Error> {
        let target_id = self.pinned_at(target.offset);
        if target_id.is_some_and(|id| self.user_pins(id) > 0) {
            return Err(ErrorKind::Busy.into());
        }
        match (moved.is_dir(), target.entry.is_dir()) {
            (true, false) => return Err(ErrorKind::NotADirectory.into()),
            (false, true) => return Err(ErrorKind::IsADirectory.into()),
            _ => {}
        }
        let target_first = match target_id.and_then(|id| self.nodes.get(id)) {
            Some(node) => node.first,
            None => target.entry.first_cluster(self.geo.kind),
        };
        if target.entry.is_dir() && !self.dir_is_empty(self.check_cluster(target_first)?).await? {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        let skip = Skip {
            entry: (from == to).then_some(src.offset),
            run: Some((target.first, target.slot)),
        };
        let plan = self.plan(to, text, false, new, skip).await?;
        let saved = self.save_run(to, target.first, target.slot).await?;
        if let Err(err) = self.clear_slots(to, target.first, target.slot + 1).await {
            self.restore_run(to, Some(&saved)).await;
            return Err(err);
        }
        self.move_entry(src, to, new, &plan, moved, dot_dot, src_id, Some(&saved))
            .await?;
        if let Some(id) = target_id {
            self.nodes.remove(id);
        }
        self.clear_slots(from, src.first, src.slot).await?;
        if target_first != 0 {
            self.free_chain(target_first).await?;
        }
        Ok(())
    }

    /// Reads slots `first..=last` of `dir`, at most one long name and its
    /// short entry.
    async fn save_run(&mut self, dir: DirStart, first: u32, last: u32) -> FsResult<SavedRun, D::Error> {
        let mut saved = SavedRun {
            first,
            len: last - first + 1,
            raw: [[0; ENTRY_SIZE as usize]; lfn::MAX_ENTRIES + 1],
        };
        if saved.len as usize > saved.raw.len() {
            return Err(ErrorKind::Corrupt.into());
        }
        let mut walk = Walk::new(dir);
        for (slot, raw) in (first..=last).zip(saved.raw.iter_mut()) {
            let at = self.slot_offset(&mut walk, slot).await?.ok_or(ErrorKind::Corrupt)?;
            read_bytes(&mut self.dev, &mut self.block, at, raw).await?;
        }
        Ok(saved)
    }

    /// Writes back the slots `save_run` read, as far as the device allows.
    async fn restore_run(&mut self, dir: DirStart, saved: Option<&SavedRun>) {
        let Some(saved) = saved else {
            return;
        };
        let mut walk = Walk::new(dir);
        for (slot, raw) in (saved.first..).zip(&saved.raw[..saved.len as usize]) {
            if let Ok(Some(at)) = self.slot_offset(&mut walk, slot).await {
                let _ = self.write(at, raw).await;
            }
        }
    }

    /// Stores `value` as the FAT entry of `cluster`, in the active copy
    /// first, then in the mirrors.
    async fn set_fat(&mut self, cluster: u32, value: u32) -> FsResult<(), D::Error> {
        let kind = self.geo.kind;
        let len = kind.entry_len();
        let within = kind.entry_offset(cluster as u64);
        let copies = if self.mirrored { self.geo.fat_count } else { 1 };
        for step in 0..copies {
            let copy = (self.active_fat + step) % self.geo.fat_count;
            let at = self.geo.fat_start + copy as u64 * self.geo.fat_size + within;
            let mut bytes = [0u8; 4];
            read_bytes(&mut self.dev, &mut self.block, at, &mut bytes[..len]).await?;
            kind.encode(cluster as u64, value, &mut bytes);
            self.write(at, &bytes[..len]).await?;
        }
        Ok(())
    }

    /// Takes a free cluster and marks it as the end of a chain. The FAT is
    /// scanned whatever the free count says, since FSInfo is only a hint.
    async fn allocate(&mut self) -> FsResult<u32, D::Error> {
        let max = self.geo.max_cluster;
        let count = max - FIRST_DATA_CLUSTER + 1;
        let from = self.next_free.clamp(FIRST_DATA_CLUSTER, max) - FIRST_DATA_CLUSTER;
        for step in 0..count {
            let cluster = FIRST_DATA_CLUSTER + (from + step) % count;
            if self.fat_entry(cluster).await? & self.geo.kind.mask() != 0 {
                continue;
            }
            self.set_fat(cluster, self.geo.kind.end_of_chain()).await?;
            self.free_clusters = self.free_clusters.and_then(|free| free.checked_sub(1));
            self.next_free = if cluster == max { FIRST_DATA_CLUSTER } else { cluster + 1 };
            self.fs_info_dirty = true;
            return Ok(cluster);
        }
        if self.free_clusters != Some(0) {
            self.free_clusters = Some(0);
            self.fs_info_dirty = true;
        }
        Err(ErrorKind::NoSpace.into())
    }

    async fn release(&mut self, cluster: u32) -> FsResult<(), D::Error> {
        self.set_fat(cluster, 0).await?;
        let total = self.geo.max_cluster - 1;
        self.free_clusters = self.free_clusters.map(|free| (free + 1).min(total));
        self.fs_info_dirty = true;
        Ok(())
    }

    /// Allocates a chain of `count` clusters, zeroed when `zero` is set, and
    /// returns its first cluster. Nothing is left allocated on failure.
    async fn allocate_chain(&mut self, count: u32, zero: bool) -> FsResult<u32, D::Error> {
        let (mut first, mut last) = (0, 0);
        for _ in 0..count {
            match self.append_cluster(last, zero).await {
                Ok(cluster) => {
                    if first == 0 {
                        first = cluster;
                    }
                    last = cluster;
                }
                Err(err) => {
                    if first != 0 {
                        let _ = self.free_chain(first).await;
                    }
                    return Err(err);
                }
            }
        }
        Ok(first)
    }

    async fn append_cluster(&mut self, prev: u32, zero: bool) -> FsResult<u32, D::Error> {
        let cluster = self.allocate().await?;
        let mut result = Ok(());
        if zero {
            result = match self.cluster_at(cluster) {
                Ok(at) => self.put(at, None, self.geo.cluster_size as usize).await,
                Err(kind) => Err(kind.into()),
            };
        }
        if result.is_ok() && prev != 0 {
            result = self.set_fat(prev, cluster).await;
        }
        if let Err(err) = result {
            let _ = self.release(cluster).await;
            return Err(err);
        }
        Ok(cluster)
    }

    /// Frees the chain starting at `first`. A freed cluster reads as free,
    /// so a cyclic chain ends with [`ErrorKind::Corrupt`].
    async fn free_chain(&mut self, first: u32) -> FsResult<(), D::Error> {
        let mut cluster = self.check_cluster(first)?;
        loop {
            let next = self.next_cluster(cluster).await?;
            self.release(cluster).await?;
            match next {
                Some(next) => cluster = next,
                None => return Ok(()),
            }
        }
    }

    /// Walks a chain towards cluster index `want` from `hint` or `first`.
    /// Returns the `(index, cluster)` reached, short of `want` when the
    /// chain ends first.
    async fn walk(&mut self, first: u32, hint: (u32, u32), want: u32) -> FsResult<(u32, u32), D::Error> {
        let (mut index, mut cluster) = if hint.1 != 0 && hint.0 <= want {
            hint
        } else {
            (0, self.check_cluster(first)?)
        };
        while index < want {
            match self.next_cluster(cluster).await? {
                Some(next) => {
                    cluster = next;
                    index += 1;
                }
                None => break,
            }
        }
        Ok((index, cluster))
    }

    /// Extends a file's chain to hold `end` bytes. The new clusters are
    /// linked but not zeroed.
    async fn cover(&mut self, state: &Node, end: u64) -> FsResult<Growth, D::Error> {
        let need = end.div_ceil(self.geo.cluster_size as u64) as u32;
        let none = Growth { first: state.first, tail: 0, added: 0 };
        if need == 0 {
            return Ok(none);
        }
        if state.first == 0 {
            let first = self.allocate_chain(need, false).await?;
            return Ok(Growth { first, tail: 0, added: first });
        }
        let (index, tail) = self.walk(state.first, state.hint, need - 1).await?;
        if index + 1 >= need {
            return Ok(none);
        }
        let added = self.allocate_chain(need - 1 - index, false).await?;
        if let Err(err) = self.set_fat(tail, added).await {
            let _ = self.free_chain(added).await;
            return Err(err);
        }
        Ok(Growth { first: state.first, tail, added })
    }

    /// Frees what [`cover`](Self::cover) added. Best effort: on failure
    /// the clusters stay allocated.
    async fn undo_growth(&mut self, growth: Growth) {
        if growth.added == 0 {
            return;
        }
        if growth.tail != 0
            && self
                .set_fat(growth.tail, self.geo.kind.end_of_chain())
                .await
                .is_err()
        {
            return;
        }
        let _ = self.free_chain(growth.added).await;
    }

    /// Writes `len` bytes of `data`, or zeros, at byte `pos` of the chain at
    /// `first`, and returns a hint for the last cluster written.
    async fn fill(
        &mut self,
        first: u32,
        hint: (u32, u32),
        pos: u64,
        data: Option<&[u8]>,
        len: usize,
    ) -> FsResult<(u32, u32), D::Error> {
        if len == 0 {
            return Ok(hint);
        }
        let cluster_size = self.geo.cluster_size as u64;
        let (mut index, mut cluster) = self.walk(first, hint, (pos / cluster_size) as u32).await?;
        let mut done = 0;
        while done < len {
            let at = pos + done as u64;
            let want = (at / cluster_size) as u32;
            while index < want {
                cluster = self.next_cluster(cluster).await?.ok_or(ErrorKind::Corrupt)?;
                index += 1;
            }
            let within = at % cluster_size;
            let n = ((cluster_size - within) as usize).min(len - done);
            let offset = self.cluster_at(cluster)? + within;
            self.put(offset, data.map(|data| &data[done..done + n]), n).await?;
            done += n;
        }
        Ok((index, cluster))
    }

    fn root_start(&self) -> DirStart {
        match self.geo.root {
            RootDir::Fixed { start, size } => DirStart::Fixed {
                start,
                slots: (size / ENTRY_SIZE) as u32,
            },
            RootDir::Cluster(cluster) => DirStart::Chain(cluster),
        }
    }

    fn check_cluster(&self, cluster: u32) -> Result<u32, ErrorKind> {
        if (FIRST_DATA_CLUSTER..=self.geo.max_cluster).contains(&cluster) {
            Ok(cluster)
        } else {
            Err(ErrorKind::Corrupt)
        }
    }

    /// Byte offset of a data cluster; [`ErrorKind::Corrupt`] for any other.
    fn cluster_at(&self, cluster: u32) -> Result<u64, ErrorKind> {
        self.geo.cluster_offset(cluster).ok_or(ErrorKind::Corrupt)
    }

    fn pinned_at(&self, offset: u64) -> Option<NodeId> {
        self.nodes.find(&mut |_, node| node.entry == offset)
    }

    /// The id derived from the location of the entry at `offset`, unless a
    /// node that has moved away from it holds that id.
    fn natural_id(&self, offset: u64) -> Option<NodeId> {
        let id = NodeId::new(offset / ENTRY_SIZE);
        self.nodes.get(id).is_none().then_some(id)
    }

    /// The id of the entry at `offset` without pinning it: its pinned id,
    /// else its natural id, else its id in the moved range.
    fn id_at(&self, offset: u64) -> NodeId {
        self.pinned_at(offset)
            .or_else(|| self.natural_id(offset))
            .unwrap_or(NodeId::new(MOVED_IDS + offset / ENTRY_SIZE))
    }

    /// Pins the node whose short entry is at `offset`.
    fn intern(&mut self, offset: u64, entry: &ShortEntry) -> Result<NodeId, ErrorKind> {
        if let Some(id) = self.pinned_at(offset) {
            self.nodes.pin(id);
            return Ok(id);
        }
        let natural = self.natural_id(offset);
        let fallback = natural.is_none();
        let id = natural.unwrap_or(NodeId::new(self.next_id));
        self.nodes
            .insert(id, Node::new(offset, entry, self.geo.kind))
            .map_err(|_| ErrorKind::LimitExceeded)?;
        if fallback {
            self.next_id += 1;
        }
        Ok(id)
    }

    async fn read_slot(&mut self, offset: u64) -> FsResult<Slot, D::Error> {
        let mut raw = [0u8; ENTRY_SIZE as usize];
        read_bytes(&mut self.dev, &mut self.block, offset, &mut raw).await?;
        Ok(Slot::parse(&raw))
    }

    /// The entry of an id that is not pinned, decoded from its location.
    async fn unpinned(&mut self, id: NodeId) -> FsResult<(u64, ShortEntry), D::Error> {
        let mut raw = id.get();
        if raw == ROOT.get() || raw >= FALLBACK_IDS {
            return Err(ErrorKind::InvalidHandle.into());
        }
        if raw >= MOVED_IDS {
            raw -= MOVED_IDS;
        }
        let offset = raw.checked_mul(ENTRY_SIZE).ok_or(ErrorKind::InvalidHandle)?;
        let in_root = matches!(
            self.geo.root,
            RootDir::Fixed { start, size } if (start..start + size).contains(&offset)
        );
        if !in_root && !(self.geo.data_start..self.data_end).contains(&offset) {
            return Err(ErrorKind::InvalidHandle.into());
        }
        match self.read_slot(offset).await? {
            Slot::Short(entry) if entry.is_visible() => Ok((offset, entry)),
            _ => Err(ErrorKind::InvalidHandle.into()),
        }
    }

    /// The state of a pinned node, or of an unpinned id decoded from disk.
    async fn node(&mut self, id: NodeId) -> FsResult<Node, D::Error> {
        Ok(self.any_node(id).await?.1)
    }

    /// The node's state and its short entry.
    async fn node_entry(&mut self, id: NodeId) -> FsResult<(Node, ShortEntry), D::Error> {
        let node = self.node(id).await?;
        match self.read_slot(node.entry).await? {
            Slot::Short(entry) => Ok((node, entry)),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    async fn dir_start(&mut self, dir: NodeId) -> FsResult<DirStart, D::Error> {
        if dir == ROOT {
            return Ok(self.root_start());
        }
        let node = self.node(dir).await?;
        if !node.dir {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok(DirStart::Chain(self.check_cluster(node.first)?))
    }

    /// The stored FAT entry of `cluster`, which must be a data cluster.
    async fn fat_entry(&mut self, cluster: u32) -> FsResult<u32, D::Error> {
        let kind = self.geo.kind;
        let fat = self.geo.fat_start + self.active_fat as u64 * self.geo.fat_size;
        let mut bytes = [0u8; 4];
        let at = fat + kind.entry_offset(cluster as u64);
        read_bytes(&mut self.dev, &mut self.block, at, &mut bytes[..kind.entry_len()]).await?;
        Ok(kind.decode(cluster as u64, &bytes))
    }

    async fn next_cluster(&mut self, cluster: u32) -> FsResult<Option<u32>, D::Error> {
        let stored = self.fat_entry(cluster).await?;
        Ok(self.geo.kind.next(stored, self.geo.max_cluster).map_err(|_| ErrorKind::Corrupt)?)
    }

    /// Byte offset of directory slot `slot`, or `None` past the end.
    async fn slot_offset(&mut self, walk: &mut Walk, slot: u32) -> FsResult<Option<u64>, D::Error> {
        if slot >= dirent::MAX_ENTRIES {
            return Ok(None);
        }
        match walk.start {
            DirStart::Fixed { start, slots } => {
                Ok((slot < slots).then(|| start + slot as u64 * ENTRY_SIZE))
            }
            DirStart::Chain(first) => {
                let per_cluster = self.geo.cluster_size / ENTRY_SIZE as u32;
                let want = slot / per_cluster;
                if walk.cluster == 0 || want < walk.index {
                    walk.index = 0;
                    walk.cluster = first;
                }
                while walk.index < want {
                    match self.next_cluster(walk.cluster).await? {
                        Some(next) => {
                            walk.cluster = next;
                            walk.index += 1;
                        }
                        None => return Ok(None),
                    }
                }
                let within = (slot % per_cluster) as u64 * ENTRY_SIZE;
                Ok(Some(self.cluster_at(walk.cluster)? + within))
            }
        }
    }

    /// Scans from `slot` to the next visible short entry, feeding long-name
    /// fragments to `long`, and leaves `slot` after it.
    async fn next_visible(
        &mut self,
        walk: &mut Walk,
        slot: &mut u32,
        long: &mut Assembler,
    ) -> FsResult<Option<Found>, D::Error> {
        let mut long_start = *slot;
        while let Some(offset) = self.slot_offset(walk, *slot).await? {
            let at = *slot;
            match self.read_slot(offset).await? {
                Slot::End => break,
                Slot::Free => long.reset(),
                Slot::Long(part) => {
                    if part.sequence & lfn::LAST_ENTRY != 0 {
                        long_start = at;
                    }
                    long.push(part.sequence, part.checksum, &part.name1, &part.name2, &part.name3)
                }
                Slot::Short(entry) if entry.is_visible() => {
                    *slot = at + 1;
                    return Ok(Some(Found { slot: at, offset, entry, long_start }));
                }
                Slot::Short(_) => long.reset(),
            }
            *slot = at + 1;
        }
        Ok(None)
    }

    /// The parent cluster recorded in the `..` entry of the directory at
    /// `first`, or `None` for the root.
    async fn dot_dot(&mut self, first: u32) -> FsResult<Option<u32>, D::Error> {
        let offset = self.cluster_at(first)? + ENTRY_SIZE;
        let Slot::Short(entry) = self.read_slot(offset).await? else {
            return Err(ErrorKind::Corrupt.into());
        };
        if !entry.is_dot_dot() {
            return Err(ErrorKind::Corrupt.into());
        }
        let cluster = entry.first_cluster(self.geo.kind);
        if cluster == 0 || self.geo.root == RootDir::Cluster(cluster) {
            return Ok(None);
        }
        Ok(Some(self.check_cluster(cluster)?))
    }
}

}

impl_fat_driver!(impl[D: BlockDevice, T: NodeTable, C: Clock, P: CodePage] FatFs<D, T, C, P>, error = D::Error; also = [parent]);
