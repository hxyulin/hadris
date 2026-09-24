use core::fmt;

use hadris_fs::{
    Attributes, Capabilities, CaseSensitivity, Clock, DateTime, DirCursor, DirEntry, ErrorKind,
    FileTimes, FileType, FixedTable, FsResult, FsStats, Metadata, MountError, Name, NameBuf,
    NameCharset, NameError, NewNode, NoClock, NodeId, NodeTable, RemoveKind, RenameFlags,
    SetMetadata,
};

use super::block_io::{BlockBuf, ClusterGroup, MAX_BLOCK_SIZE, load, read_bytes, write_bytes};
use super::storage::BlockDevice;
use crate::codec::cycle::Hint;
use crate::codec::name as names;
use crate::exfat::codec::{
    self, Geometry, MAX_SET, PageStart, RawEntry, Units, UpcaseDecoder, le16, le32, le64,
};
use crate::exfat::raw::{self, ENTRY_SIZE};
use crate::exfat::{MountOptions, VolumeLabel};

#[path = "check.rs"]
mod fsck;
pub use fsck::{check, check_with};

const ROOT: NodeId = NodeId::new(1);
/// A node id holds the slot of its File entry, the entry's byte offset
/// divided by 32, in its low `SLOT_BITS` bits, and a tier above, as in
/// `FatFs`. An exFAT heap ends below `2^58` bytes, so slots stay below
/// `2^53`.
const SLOT_BITS: u32 = 53;
const SLOT_MASK: u64 = (1 << SLOT_BITS) - 1;
const MAX_TIER: u64 = (1 << (63 - SLOT_BITS)) - 1;
/// The id of the table slot `create` reserves before it writes anything.
const RESERVED: NodeId = NodeId::new(1 << 63);
/// The entry offset that stands for the root directory, which has none.
const ROOT_ENTRY: u64 = 0;
/// A parent not yet known.
const UNKNOWN_PARENT: u64 = u64::MAX;
/// The deepest directory the tree searches enter.
pub(super) const MAX_DEPTH: usize = 64;
const BOOT_SECTOR_LEN: usize = 512;
/// Offset of `VolumeFlags` in the boot sector.
const VOLUME_FLAGS_AT: u64 = 106;
/// Offset of `PercentInUse` in the boot sector.
const PERCENT_AT: u64 = 112;
/// The largest up-case table: 65536 mappings, uncompressed.
const MAX_UPCASE_LEN: u64 = 0x2_0000;
/// Decoded up-case pages kept besides page 0.
const UPCASE_CACHE: usize = 2;
/// The attribute bits [`Attributes`] maps to.
const ATTR_MAPPED: [(u16, Attributes); 4] = [
    (raw::ATTR_READ_ONLY, Attributes::READ_ONLY),
    (raw::ATTR_HIDDEN, Attributes::HIDDEN),
    (raw::ATTR_SYSTEM, Attributes::SYSTEM),
    (raw::ATTR_ARCHIVE, Attributes::ARCHIVE),
];

/// State of a pinned node.
#[derive(Debug, Clone, Copy)]
pub(super) struct Node {
    /// Byte offset of the node's File entry.
    entry: u64,
    /// Byte offset of the File entry of the directory holding it,
    /// [`ROOT_ENTRY`] or [`UNKNOWN_PARENT`].
    parent: u64,
    first: u32,
    /// `DataLength`.
    len: u64,
    /// `ValidDataLength`.
    valid: u64,
    /// `NoFatChain`: the allocation is contiguous and has no FAT chain.
    contiguous: bool,
    dir: bool,
    /// A known position in the allocation.
    hint: Hint,
    /// The entry set lacks the sizes and modification time. A dirty node
    /// holds one pin of the driver's own until it is written.
    dirty: bool,
    opens: u32,
    /// Removed while pinned: every method but `forget` and `close_node`
    /// answers `NotFound`.
    unlinked: bool,
}

impl Node {
    fn from_set(set: &Set, parent: u64) -> Self {
        Self {
            entry: set.offset,
            parent,
            first: set.first(),
            len: set.data_len(),
            valid: set.valid_len().min(set.data_len()),
            contiguous: set.flags() & raw::NO_FAT_CHAIN != 0,
            dir: set.is_dir(),
            hint: Hint::NONE,
            dirty: false,
            opens: 0,
            unlinked: false,
        }
    }

    fn alloc(&self) -> Alloc {
        Alloc {
            first: self.first,
            contiguous: self.contiguous,
        }
    }
}

/// Where a file's or directory's clusters are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Alloc {
    /// 0 when nothing is allocated.
    first: u32,
    contiguous: bool,
}

/// Where a directory's entries live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DirStart {
    pub(super) alloc: Alloc,
    /// `DataLength`; `u64::MAX` for the root, which ends with its chain.
    pub(super) size: u64,
}

/// A position in a directory's clusters, kept across slots so a scan
/// walks the chain once.
#[derive(Clone, Copy)]
pub(super) struct Walk {
    start: DirStart,
    at: Hint,
    /// The last cluster reached.
    last: u32,
}

impl Walk {
    pub(super) fn new(start: DirStart) -> Self {
        Self {
            start,
            at: Hint::NONE,
            last: 0,
        }
    }
}

/// A File entry set read from a directory.
#[derive(Clone, Copy)]
pub(super) struct Set {
    /// Byte offset of the File entry.
    pub(super) offset: u64,
    pub(super) count: usize,
    pub(super) raw: [RawEntry; MAX_SET],
    /// Byte offset of each entry.
    pub(super) at: [u64; MAX_SET],
}

impl Set {
    /// An empty set for a read to fill.
    pub(super) fn new() -> Self {
        Self {
            offset: 0,
            count: 0,
            raw: [[0; ENTRY_SIZE]; MAX_SET],
            at: [0; MAX_SET],
        }
    }

    pub(super) fn attributes(&self) -> u16 {
        le16(&self.raw[0], 4)
    }

    pub(super) fn is_dir(&self) -> bool {
        self.attributes() & raw::ATTR_DIRECTORY != 0
    }

    pub(super) fn flags(&self) -> u8 {
        self.raw[1][1]
    }

    pub(super) fn first(&self) -> u32 {
        le32(&self.raw[1], 20)
    }

    pub(super) fn data_len(&self) -> u64 {
        le64(&self.raw[1], 24)
    }

    pub(super) fn valid_len(&self) -> u64 {
        le64(&self.raw[1], 8)
    }

    fn alloc(&self) -> Alloc {
        Alloc {
            first: self.first(),
            contiguous: self.flags() & raw::NO_FAT_CHAIN != 0,
        }
    }

    /// The name's length in code units, from the Stream Extension entry.
    fn name_len(&self) -> usize {
        self.raw[1][3] as usize
    }

    /// The name's code units, read from its File Name entries.
    fn name_units(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.name_len()).map(|at| {
            let entry = &self.raw[2 + at / raw::NAME_UNITS_PER_ENTRY];
            le16(entry, 2 + at % raw::NAME_UNITS_PER_ENTRY * 2)
        })
    }

    /// Benign secondary entries after the name.
    fn extras(&self) -> &[RawEntry] {
        &self.raw[2 + self.name_len().div_ceil(raw::NAME_UNITS_PER_ENTRY)..self.count]
    }

    /// The allocations and lengths that benign secondary entries, such as
    /// Vendor Allocation entries, hold.
    fn extra_allocs(&self) -> impl Iterator<Item = (Alloc, u64)> + '_ {
        self.extras()
            .iter()
            .filter(|entry| {
                entry[0] & raw::IMPORTANCE_BENIGN != 0 && entry[1] & raw::ALLOCATION_POSSIBLE != 0
            })
            .map(|entry| {
                let alloc = Alloc {
                    first: le32(entry, 20),
                    contiguous: entry[1] & raw::NO_FAT_CHAIN != 0,
                };
                (alloc, le64(entry, 24))
            })
    }
}

/// Whether the entries form a File entry set with a Stream Extension, the
/// File Name entries its name length needs, and a valid checksum.
pub(super) fn parse_set(entries: &[RawEntry]) -> bool {
    let Some(primary) = entries.first() else {
        return false;
    };
    if primary[0] != raw::ENTRY_FILE || !(2..MAX_SET).contains(&(primary[1] as usize)) {
        return false;
    }
    let count = 1 + primary[1] as usize;
    let Some(set) = entries.get(..count) else {
        return false;
    };
    let stream = &set[1];
    let len = stream[3] as usize;
    if stream[0] != raw::ENTRY_STREAM || len == 0 {
        return false;
    }
    let entries_needed = len.div_ceil(raw::NAME_UNITS_PER_ENTRY);
    if 2 + entries_needed > count {
        return false;
    }
    if set[2..2 + entries_needed]
        .iter()
        .any(|entry| entry[0] != raw::ENTRY_NAME)
    {
        return false;
    }
    if set[2 + entries_needed..]
        .iter()
        .any(|entry| entry[0] & raw::CATEGORY_SECONDARY == 0 || entry[0] & raw::IN_USE == 0)
    {
        return false;
    }
    codec::set_checksum(set) == le16(primary, 2)
}

/// A named entry set found by a directory scan.
struct Located {
    set: Set,
    slot: u32,
    /// Whether the query equals the entry's name exactly.
    exact: bool,
}

impl Located {
    fn new() -> Self {
        Self {
            set: Set::new(),
            slot: 0,
            exact: false,
        }
    }
}

/// Where a new entry set goes.
struct Plan {
    start: u32,
    /// Clusters to append to the directory first.
    grow: u32,
}

/// Clusters added to a file's allocation, to undo on failure.
#[derive(Clone, Copy)]
struct Growth {
    alloc: Alloc,
    /// The first added cluster, 0 when none were added.
    added: u32,
}

/// What links the chain of a [`Pending`] into the volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// Nothing does.
    None,
    /// The FAT entry of this cluster.
    Cluster(u32),
    /// The FAT entry of this cluster, for a file whose new size is not yet
    /// recorded: recovery cuts the chain there.
    Tail(u32),
    /// The entry set at this byte offset, of a node that stays.
    Entry(u64),
    /// The entry set at this byte offset, of a node being removed.
    Removed(u64),
}

/// Clusters an unfinished operation holds: allocated but not yet linked,
/// or unlinked but not yet freed. The next writing operation frees them
/// unless `owner` links `head` on disk.
#[derive(Debug, Clone, Copy)]
struct Pending {
    /// The chain, 0 for none.
    head: u32,
    /// One more chain, allocated or being freed, that `head` does not
    /// reach: a single cluster, or one that leads into `head`. 0 for none.
    extra: u32,
    owner: Owner,
}

impl Pending {
    const fn chain(head: u32, owner: Owner) -> Self {
        Self {
            head,
            extra: 0,
            owner,
        }
    }
}

/// How the next writing operation finishes an interrupted write of an
/// entry set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetWrite {
    /// A new set: removed unless all of it landed.
    Insert,
    /// Changes to a set: its name hash and checksum are made to match the
    /// entries that landed.
    Update,
    /// A removal: completed.
    Clear,
}

/// The entries of a set being written.
#[derive(Debug, Clone, Copy)]
struct SetPending {
    at: [u64; MAX_SET],
    count: u8,
    kind: SetWrite,
}

impl SetPending {
    fn new(at: &[u64], kind: SetWrite) -> Self {
        let mut all = [0u64; MAX_SET];
        all[..at.len()].copy_from_slice(at);
        Self {
            at: all,
            count: at.len() as u8,
            kind,
        }
    }
}

/// Whether the driver has marked the volume dirty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dirty {
    /// `VolumeDirty` is clear on disk.
    Clean,
    /// This driver set `VolumeDirty` and clears it in `sync`.
    Marked,
    /// `VolumeDirty` was set at mount; it is left for a checker to clear.
    Inherited,
}

/// A chain of clusters holding metadata: the bitmap or the up-case table.
#[derive(Debug, Clone, Copy)]
pub(super) struct Extent {
    pub(super) first: u32,
    pub(super) len: u64,
    pub(super) contiguous: bool,
}

/// The up-case table: an index of where each 256-unit page starts, page 0
/// decoded, and a few more pages cached.
pub(super) struct Upcase {
    pub(super) extent: Extent,
    pub(super) checksum: u32,
    pub(super) stored_checksum: u32,
    starts: [PageStart; 256],
    identity: [u8; 32],
    page0: [u16; 256],
    cache: [(u16, [u16; 256]); UPCASE_CACHE],
    victim: usize,
}

impl Upcase {
    /// An empty table for `index_upcase` to fill.
    fn new() -> Self {
        Self {
            extent: Extent {
                first: 0,
                len: 0,
                contiguous: true,
            },
            checksum: 0,
            stored_checksum: 0,
            starts: [PageStart::default(); 256],
            identity: [0xFF; 32],
            page0: [0; 256],
            cache: [(0, [0; 256]); UPCASE_CACHE],
            victim: 0,
        }
    }

    fn is_identity(&self, page: usize) -> bool {
        self.identity[page / 8] & (1 << (page % 8)) != 0
    }

    /// Whether the table matches its checksum and maps the first 128 code
    /// points as the specification requires.
    pub(super) fn is_valid(&self) -> bool {
        self.checksum == self.stored_checksum
            && (0..128u16).all(|code| self.page0[code as usize] == codec::mandatory_upcase(code))
    }
}

fn metadata(node: &Node, set: &Set) -> Metadata {
    let primary = &set.raw[0];
    let times = FileTimes::new()
        .with_created(codec::decode_time(
            le32(primary, 8),
            primary[20],
            primary[22],
        ))
        .with_modified(codec::decode_time(
            le32(primary, 12),
            primary[21],
            primary[23],
        ))
        .with_accessed(codec::decode_time(le32(primary, 16), 0, primary[24]));
    let mut attributes = Attributes::empty();
    for (bit, flag) in ATTR_MAPPED {
        if set.attributes() & bit != 0 {
            attributes |= flag;
        }
    }
    let (file_type, len) = if node.dir {
        (FileType::Dir, 0)
    } else {
        (FileType::File, node.len)
    };
    Metadata::new(file_type)
        .with_len(len)
        .with_times(times)
        .with_attributes(attributes)
}

fn set_attributes(primary: &mut RawEntry, attributes: Attributes) {
    let mut value = le16(primary, 4);
    for (bit, flag) in ATTR_MAPPED {
        if attributes.contains(flag) {
            value |= bit;
        } else {
            value &= !bit;
        }
    }
    primary[4..6].copy_from_slice(&value.to_le_bytes());
}

fn stamp(primary: &mut RawEntry, which: usize, time: DateTime) {
    let (stamp, increment, offset) = codec::encode_time(time);
    let (at, increment_at, offset_at) =
        [(8, Some(20), 22), (12, Some(21), 23), (16, None, 24)][which];
    primary[at..at + 4].copy_from_slice(&stamp.to_le_bytes());
    if let Some(increment_at) = increment_at {
        primary[increment_at] = increment;
    }
    primary[offset_at] = offset;
}

const CREATED: usize = 0;
const MODIFIED: usize = 1;
const ACCESSED: usize = 2;

fn set_stream(stream: &mut RawEntry, alloc: Alloc, len: u64, valid: u64) {
    let mut flags = stream[1] | raw::ALLOCATION_POSSIBLE;
    if alloc.contiguous {
        flags |= raw::NO_FAT_CHAIN;
    } else {
        flags &= !raw::NO_FAT_CHAIN;
    }
    stream[1] = flags;
    stream[8..16].copy_from_slice(&valid.to_le_bytes());
    stream[20..24].copy_from_slice(&alloc.first.to_le_bytes());
    stream[24..32].copy_from_slice(&len.to_le_bytes());
}

/// Builds a File entry set for `name` from a primary and stream template
/// and extra secondary entries into the front of `set`, and returns how many
/// entries it has. `hash` is the name's `NameHash`.
fn build_set(
    primary: &RawEntry,
    stream: &RawEntry,
    name: &Units,
    hash: u16,
    extras: &[RawEntry],
    set: &mut [RawEntry; MAX_SET],
) -> Result<usize, ErrorKind> {
    let count = 2 + name.entries() + extras.len();
    if count > MAX_SET {
        return Err(ErrorKind::NameTooLong);
    }
    set[..count].fill([0; ENTRY_SIZE]);
    set[0] = *primary;
    set[0][0] = raw::ENTRY_FILE;
    set[0][1] = (count - 1) as u8;
    set[1] = *stream;
    set[1][0] = raw::ENTRY_STREAM;
    set[1][3] = name.len as u8;
    set[1][4..6].copy_from_slice(&hash.to_le_bytes());
    for (index, chunk) in name
        .as_slice()
        .chunks(raw::NAME_UNITS_PER_ENTRY)
        .enumerate()
    {
        let entry = &mut set[2 + index];
        entry[0] = raw::ENTRY_NAME;
        for (unit, &value) in chunk.iter().enumerate() {
            entry[2 + unit * 2..4 + unit * 2].copy_from_slice(&value.to_le_bytes());
        }
    }
    set[2 + name.entries()..count].copy_from_slice(extras);
    codec::seal(&mut set[..count]);
    Ok(count)
}

/// The name a query must be, as a string: not `.` or `..`.
fn entry_name(name: &Name, invalid: ErrorKind) -> Result<&str, ErrorKind> {
    match name.to_str() {
        Ok("." | "..") => Err(ErrorKind::InvalidInput),
        Ok(text) => Ok(text),
        Err(_) => Err(invalid),
    }
}

/// An exFAT volume on a block device.
///
/// `ExFatFs` is the sibling of `FatFs` for exFAT: every node method takes
/// `&mut self`, holds no lock and needs no allocator, and the `FsDriver`
/// methods forward to inherent methods of the same names. Share it through
/// `hadris_fs` `Volume`, or call the node methods directly.
///
/// Mount with [`open`](ExFatFs::open), or with
/// [`open_with`](ExFatFs::open_with) and [`MountOptions`] to choose the
/// node table `T` and the [`Clock`] `C`, as for `FatFs`. Nodes are
/// identified by the location of their File entry; `lookup`, `create` and
/// `parent` pin the node they return, `forget` unpins it, and a pinned node
/// keeps its id across `rename`. Ids from `read_dir_entry` are not pinned
/// and stay valid until that directory changes.
///
/// Names are UTF-16, up to 255 code units, and compare through the
/// volume's up-case table, so lookups ignore case as the volume defines
/// it. A lookup passes over entry sets whose `NameHash` differs from the
/// name's, so a set with a wrong `NameHash`, which `check` reports, is not
/// found by name. New names may not hold control characters or `"*/:<>?\|`, be `.`
/// or `..`, or end in a dot or space. `parent` finds the directory of a
/// node from the node table, or by searching the tree from the root when
/// it is not known; exFAT has no `..` entries.
///
/// The driver keeps one device block of at most 4096 bytes, an index of
/// the up-case table and three decoded pages of it, a little over 8 KiB
/// plus the node table. Runs of clusters that follow one another on disk
/// are read and written in one device call, and growing or freeing an
/// allocation writes the FAT and the bitmap a device block at a time.
///
/// # Writing
///
/// Writes go to the device at once. As in `FatFs`, `write_at` and
/// `set_len` keep a pinned file's new sizes and modification time in the
/// node table until `publish_node`, `sync_node` or `sync`, unless its
/// first cluster changes. New allocations always have a FAT chain; a
/// contiguous (`NoFatChain`) allocation made by another implementation is
/// given a chain when it grows. `set_len` grows a file by raising its
/// `DataLength` alone, and the bytes past `ValidDataLength` read as zeros,
/// so growing writes no data. Directories grow a cluster at a time, up to
/// 256 MiB.
///
/// The first write after mounting or after `sync` sets `VolumeDirty`, and
/// `sync` clears it again, as the specification recommends; a volume that
/// was dirty at mount stays dirty. `sync` also records `PercentInUse`.
///
/// A device that answers a write with `WriteError::ReadOnly` fails that
/// operation with [`ErrorKind::ReadOnly`], and the volume is read-only from
/// then on.
///
/// # Crash and cancellation safety
///
/// Clusters are marked in the FAT and the bitmap before an entry set names
/// them, and freed only after the entry set is gone, so an interrupted
/// operation never leaves an allocation that another entry can take. The
/// entries of a set that lie in one device block are written with one
/// device write, so a set that does not cross a block boundary is written
/// or cleared whole. Across a boundary a new set is written secondary
/// entries first and its File entry last, and a removed set loses its File
/// entry first. `rename` writes the new set before it removes the old one,
/// so an interruption can leave the node under both names.
///
/// The driver remembers what an unfinished operation leaves: clusters
/// allocated but not yet linked or unlinked but not yet freed, and an entry
/// set it was writing. The next `create`, `remove`, `rename`, `write_at`,
/// `set_len`, `set_metadata`, `set_label` or `sync` frees the clusters
/// unless the interrupted write linked them, removes a new set that did
/// not land whole, completes a removal, reseals an updated set so its
/// checksum and name hash match, and cuts back a chain a dropped write had
/// grown past the file's size. Only a process that stops, or a driver that
/// is dropped, in between leaves lost clusters, or secondary entries and a
/// set checksum that `check` reports. What cannot be finished because the
/// volume turns out to be corrupt is dropped.
pub struct ExFatFs<D, T: NodeTable = FixedTable<64>, C: Clock = NoClock> {
    pub(super) dev: D,
    pub(super) geo: Geometry,
    nodes: T::With<Node>,
    pub(super) block: BlockBuf,
    pub(super) bitmap: Extent,
    bitmap_hint: Hint,
    /// The Allocation Bitmap of the FAT that is not active, on a TexFAT
    /// volume that has one.
    mirror_bitmap: Option<Extent>,
    pub(super) upcase: Upcase,
    free_clusters: Option<u32>,
    next_free: u32,
    allocation_changed: bool,
    dirty: Dirty,
    /// Clusters an interrupted operation left to free.
    pending: Option<Pending>,
    /// An entry set an interrupted operation was writing.
    pending_set: Option<SetPending>,
    read_only: bool,
    clock: C,
    /// Some pinned node is not at the slot its id names, because it was
    /// renamed or removed. Until the table empties, `pinned_at` searches it.
    moved: bool,
    /// A known `(index, cluster)` of the root directory's chain.
    root_hint: Hint,
}

impl<D, T: NodeTable, C: Clock> fmt::Debug for ExFatFs<D, T, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExFatFs")
            .field("cluster_size", &self.geo.cluster_size())
            .field("clusters", &self.geo.cluster_count)
            .field("open_nodes", &(self.nodes.len() + 1))
            .field("free_clusters", &self.free_clusters)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

io_transform! {

/// The geometry of the boot region at `base`, if its boot sector is valid
/// and its checksum matches.
async fn boot_region<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf, base: u64) -> FsResult<Option<Geometry>, D::Error> {
    let mut sector = [0u8; BOOT_SECTOR_LEN];
    read_bytes(dev, block, base, &mut sector).await?;
    let boot: raw::BootSector = bytemuck::pod_read_unaligned(&sector);
    let Ok(geo) = codec::parse_boot(&boot) else {
        return Ok(None);
    };
    let size = geo.sector_size();
    let mut chunk = [0u8; 128];
    let mut sum = 0u32;
    let mut at = 0;
    while at < (raw::BOOT_REGION_SECTORS - 1) * size {
        read_bytes(dev, block, base + at, &mut chunk).await?;
        sum = codec::boot_checksum(sum, at.min(1), &chunk);
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
/// damaged, through `block`, which the caller sized to the device's blocks.
/// The flag is set when the backup was used.
async fn boot<D: BlockDevice>(dev: &mut D, block: &mut BlockBuf) -> FsResult<(Geometry, bool), D::Error> {
    let size = block.size;
    if size > MAX_BLOCK_SIZE {
        return Err(ErrorKind::Unsupported.into());
    }
    let mut backup = false;
    let mut geo = boot_region(dev, block, 0).await?;
    for shift in 9..=12u8 {
        if geo.is_some() {
            break;
        }
        let base = raw::BOOT_REGION_SECTORS << shift;
        if base + (raw::BOOT_REGION_SECTORS << shift) > dev.block_count().saturating_mul(size as u64) {
            break;
        }
        geo = boot_region(dev, block, base).await?.filter(|geo| geo.sector_shift == shift);
        backup = geo.is_some();
    }
    let Some(geo) = geo else {
        let mut head = [0u8; 11];
        read_bytes(dev, block, 0, &mut head).await?;
        return Err(match head[3..] == raw::FILE_SYSTEM_NAME {
            true => ErrorKind::Corrupt.into(),
            false => ErrorKind::NotRecognized.into(),
        });
    };
    let device_len = dev.block_count().saturating_mul(size as u64);
    let heap_end = geo.heap_start + ((geo.cluster_count as u64) << geo.cluster_shift);
    if geo.volume_len > device_len || heap_end > geo.volume_len {
        return Err(ErrorKind::Corrupt.into());
    }
    Ok((geo, backup))
}

/// Finds the root's bitmap and up-case entries through `block` and indexes
/// the up-case table into the empty `upcase`.
async fn system_structures<D: BlockDevice>(
    dev: &mut D,
    block: &mut BlockBuf,
    geo: Geometry,
    upcase: &mut Upcase,
) -> FsResult<[Option<Extent>; 2], D::Error> {
    let mut probe = Probe { dev, block, geo };
    let (entries, upcase_entry) = probe.system_entries().await?;
    let mut bitmaps = [None; 2];
    for (entry, bitmap) in entries.into_iter().zip(&mut bitmaps) {
        let Some((first, len)) = entry else { continue };
        let extent = probe.extent(first, len).await?;
        if extent.len < (geo.cluster_count as u64).div_ceil(8) {
            return Err(ErrorKind::Corrupt.into());
        }
        *bitmap = Some(extent);
    }
    let (first, len, stored_checksum) = upcase_entry;
    if !(2..=MAX_UPCASE_LEN).contains(&len) {
        return Err(ErrorKind::Corrupt.into());
    }
    let extent = probe.extent(first, len).await?;
    probe.index_upcase(extent, stored_checksum, upcase).await?;
    Ok(bitmaps)
}

}

/// The device and geometry during a mount, before the driver exists.
struct Probe<'a, D> {
    dev: &'a mut D,
    block: &'a mut BlockBuf,
    geo: Geometry,
}

io_transform! {

impl<D: BlockDevice> Probe<'_, D> {
    async fn fat(&mut self, cluster: u32) -> FsResult<u32, D::Error> {
        let mut bytes = [0u8; 4];
        read_bytes(self.dev, self.block, self.geo.fat_start + cluster as u64 * 4, &mut bytes).await?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// The first Allocation Bitmap entry of the active FAT, that of the
    /// other FAT of a TexFAT volume, and the first Up-case Table entry of
    /// the root directory.
    #[allow(clippy::type_complexity)]
    async fn system_entries(&mut self) -> FsResult<([Option<(u32, u64)>; 2], (u32, u64, u32)), D::Error> {
        let cluster_size = self.geo.cluster_size();
        let mut cluster = self.geo.root;
        let mut bitmap = None;
        let mut mirror = None;
        let mut upcase = None;
        for _ in 0..self.geo.cluster_count {
            let base = self.geo.cluster_offset(cluster).ok_or(ErrorKind::Corrupt)?;
            let mut at = 0;
            while at < cluster_size {
                let mut entry = [0u8; ENTRY_SIZE];
                read_bytes(self.dev, self.block, base + at, &mut entry).await?;
                match entry[0] {
                    raw::ENTRY_END => break,
                    raw::ENTRY_BITMAP if entry[1] & 1 == self.geo.active && bitmap.is_none() => {
                        bitmap = Some((le32(&entry, 20), le64(&entry, 24)));
                    }
                    raw::ENTRY_BITMAP if self.geo.mirror_fat.is_some() && entry[1] & 1 != self.geo.active && mirror.is_none() => {
                        mirror = Some((le32(&entry, 20), le64(&entry, 24)));
                    }
                    raw::ENTRY_UPCASE if upcase.is_none() => {
                        upcase = Some((le32(&entry, 20), le64(&entry, 24), le32(&entry, 4)));
                    }
                    _ => {}
                }
                if let (Some(_), Some(upcase), true) = (bitmap, upcase, mirror.is_some() || self.geo.mirror_fat.is_none()) {
                    return Ok(([bitmap, mirror], upcase));
                }
                at += ENTRY_SIZE as u64;
            }
            if at < cluster_size {
                break;
            }
            match self.fat(cluster).await? {
                raw::FAT_END => break,
                next if self.geo.is_cluster(next) => cluster = next,
                _ => return Err(ErrorKind::Corrupt.into()),
            }
        }
        match (bitmap, upcase) {
            (Some(_), Some(upcase)) => Ok(([bitmap, mirror], upcase)),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    /// Checks that the chain at `first` holds `len` bytes, and whether it
    /// is contiguous.
    async fn extent(&mut self, first: u32, len: u64) -> FsResult<Extent, D::Error> {
        let clusters = len.div_ceil(self.geo.cluster_size());
        if !self.geo.is_cluster(first) || clusters == 0 || clusters > self.geo.cluster_count as u64 {
            return Err(ErrorKind::Corrupt.into());
        }
        let mut contiguous = true;
        let mut cluster = first;
        for _ in 1..clusters {
            match self.fat(cluster).await? {
                next if self.geo.is_cluster(next) => {
                    contiguous &= next == cluster + 1;
                    cluster = next;
                }
                _ => return Err(ErrorKind::Corrupt.into()),
            }
        }
        Ok(Extent { first, len, contiguous })
    }

    /// Reads the up-case table once into the empty `upcase`: its checksum,
    /// where each page starts, which pages map every code point to itself,
    /// and page 0.
    async fn index_upcase(&mut self, extent: Extent, stored_checksum: u32, upcase: &mut Upcase) -> FsResult<(), D::Error> {
        upcase.extent = extent;
        upcase.stored_checksum = stored_checksum;
        for (code, slot) in upcase.page0.iter_mut().enumerate() {
            *slot = code as u16;
        }
        let cluster_size = self.geo.cluster_size();
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
                    self.fat(cluster).await?
                };
            }
            let base = self.geo.cluster_offset(cluster).ok_or(ErrorKind::Corrupt)?;
            let n = (chunk.len() as u64).min(cluster_size - within).min(extent.len - pos) as usize;
            read_bytes(self.dev, self.block, base + within, &mut chunk[..n]).await?;
            upcase.checksum = codec::table_checksum(upcase.checksum, &chunk[..n]);
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
}

}

io_transform! {

impl<D: BlockDevice> ExFatFs<D> {
    /// Mounts the volume on `dev` with the defaults of
    /// [`MountOptions::new`]: writable, a `FixedTable<64>` and [`NoClock`].
    ///
    /// A main boot region whose boot sector or checksum is bad is replaced by
    /// a valid backup boot region, and the volume is then mounted read-only,
    /// as it is when the up-case table does not match its `TableChecksum`
    /// or maps the first 128 code points wrongly; [`check`](super::check)
    /// reports both.
    ///
    /// On a TexFAT volume with two FATs the FAT and Allocation Bitmap that
    /// `ActiveFat` selects are read, and every change is written to both
    /// FATs and both bitmaps.
    ///
    /// Fails with [`ErrorKind::NotRecognized`] when the first sector does not
    /// name exFAT, and with [`ErrorKind::Corrupt`] when neither boot region
    /// holds a valid exFAT boot sector and checksum, the boot sector describes a
    /// volume larger than the device, or the root directory lacks an
    /// Allocation Bitmap or Up-case Table entry whose chain holds it; and
    /// with [`ErrorKind::Unsupported`] when the device's blocks are larger
    /// than 4096 bytes. The [`MountError`] gives `dev` back.
    pub async fn open(dev: D) -> Result<Self, MountError<D, D::Error>> {
        Self::open_with(dev, MountOptions::new()).await
    }
}

impl<D: BlockDevice, T: NodeTable, C: Clock> ExFatFs<D, T, C> {
    /// Mounts the volume on `dev` with `options`, which set the node table
    /// and clock types. Fails as [`open`](ExFatFs::open) does.
    pub async fn open_with(mut dev: D, options: MountOptions<T, C>) -> Result<Self, MountError<D, D::Error>> {
        let MountOptions { read_only, table, clock } = options;
        let mut block = BlockBuf::new(dev.block_size().get() as usize);
        let (geo, backup) = match boot(&mut dev, &mut block).await {
            Ok(found) => found,
            Err(error) => return Err(MountError::new(error, dev)),
        };
        let mut upcase = Upcase::new();
        let [bitmap, mirror_bitmap] = match system_structures(&mut dev, &mut block, geo, &mut upcase).await {
            Ok(bitmaps) => bitmaps,
            Err(error) => return Err(MountError::new(error, dev)),
        };
        let Some(bitmap) = bitmap else {
            return Err(MountError::new(ErrorKind::Corrupt.into(), dev));
        };
        let upcase_valid = upcase.is_valid();
        let dirty = if geo.flags & raw::VOLUME_DIRTY != 0 { Dirty::Inherited } else { Dirty::Clean };
        Ok(Self {
            dev,
            geo,
            nodes: table.empty(),
            block,
            bitmap,
            bitmap_hint: Hint::NONE,
            mirror_bitmap,
            upcase,
            free_clusters: None,
            next_free: raw::FIRST_CLUSTER,
            allocation_changed: false,
            dirty,
            pending: None,
            pending_set: None,
            read_only: read_only || backup || !upcase_valid,
            clock,
            moved: false,
            root_hint: Hint::NONE,
        })
    }

    /// Returns the device.
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// Whether the volume was mounted with
    /// [`MountOptions::with_read_only`], from its backup boot region or with
    /// an up-case table that fails its checksum, or the device has refused a
    /// write since.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The clock that stamps new and modified entries.
    pub fn clock(&self) -> &C {
        &self.clock
    }

    /// Number of nodes in the node table, plus one for the root, which is
    /// always pinned.
    pub fn open_nodes(&self) -> usize {
        self.nodes.len() + 1
    }

    /// The volume serial number.
    pub fn volume_id(&self) -> u32 {
        self.geo.serial
    }

    /// The cluster size in bytes.
    pub fn cluster_size(&self) -> u32 {
        self.geo.cluster_size() as u32
    }

    /// What this volume supports: case-insensitive, case-preserving UTF-16
    /// names of up to 255 code units (765 bytes of UTF-8) and modification
    /// times in 10 ms steps. Writable unless
    /// [`is_read_only`](Self::is_read_only).
    pub fn capabilities(&self) -> Capabilities {
        let caps = Capabilities::new()
            .with_case_sensitivity(CaseSensitivity::InsensitivePreserving)
            .with_name_charset(NameCharset::Utf16)
            .with_max_name_len(raw::MAX_NAME_UNITS * 3)
            .with_timestamp_resolution_ns(10_000_000);
        if self.read_only { caps } else { caps.with_writable() }
    }

    /// The root directory. Always pinned.
    pub fn root(&self) -> NodeId {
        ROOT
    }

    /// The volume label: the Volume Label entry of the root directory, or
    /// `None` when there is none or it is empty.
    pub async fn label(&mut self) -> FsResult<Option<VolumeLabel>, D::Error> {
        Ok(self.find_label().await?.and_then(|(_, entry)| {
            let count = entry[1] as usize;
            if count == 0 || count > raw::MAX_LABEL_UNITS {
                return None;
            }
            let mut units = [0u16; raw::MAX_LABEL_UNITS];
            for (index, unit) in units.iter_mut().enumerate().take(count) {
                *unit = le16(&entry, 2 + index * 2);
            }
            Some(VolumeLabel::from_disk(units, count as u8))
        }))
    }

    /// Sets the volume label, or empties it with `None`. The root
    /// directory's Volume Label entry is rewritten, or created in a free
    /// slot of the root directory, which grows when it has none.
    pub async fn set_label(&mut self, label: Option<VolumeLabel>) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let mut entry = [0u8; ENTRY_SIZE];
        entry[0] = raw::ENTRY_LABEL;
        if let Some(label) = &label {
            entry[1] = label.as_utf16().len() as u8;
            for (index, unit) in label.as_utf16().iter().enumerate() {
                entry[2 + index * 2..4 + index * 2].copy_from_slice(&unit.to_le_bytes());
            }
        }
        if let Some((offset, _)) = self.find_label().await? {
            return self.write(offset, &entry).await;
        }
        if label.is_none() {
            return Ok(());
        }
        let start = self.root_start();
        let plan = self.plan(start, None, 1, None).await?;
        let mut set = [[0u8; ENTRY_SIZE]; MAX_SET];
        set[0] = entry;
        self.insert(start, ROOT_ENTRY, &plan, &set[..1]).await?;
        Ok(())
    }

    /// Finds `name` in `dir` and pins the result. Names compare through
    /// the up-case table.
    pub async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        let (start, dir_entry) = self.dir_info(dir).await?;
        let Some(query) = name.to_str().ok().and_then(Units::query) else {
            return Err(ErrorKind::NotFound.into());
        };
        let mut found = Located::new();
        if !self.find(start, &query, &mut found).await? {
            return Err(ErrorKind::NotFound.into());
        }
        Ok(self.intern(&found.set, dir_entry)?)
    }

    /// Metadata of a pinned node or of an id just returned by
    /// `read_dir_entry`. Directories have length 0.
    pub async fn node_metadata(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        if node == ROOT {
            return Ok(Metadata::new(FileType::Dir));
        }
        let (_, state) = self.any_node(node).await?;
        let mut set = Set::new();
        self.set_at(state.entry, &mut set).await?;
        Ok(metadata(&state, &set))
    }

    /// Writes the entry after `cursor` into `name` and advances `cursor`.
    /// `None` at the end. The raw cursor is the index of the next directory
    /// entry. A directory whose cluster chain loops fails with
    /// [`ErrorKind::Corrupt`] once the walk comes back around.
    pub async fn read_dir_entry(
        &mut self,
        dir: NodeId,
        cursor: &mut DirCursor,
        name: &mut NameBuf,
    ) -> FsResult<Option<DirEntry>, D::Error> {
        let (start, _) = self.dir_info(dir).await?;
        let Ok(mut slot) = u32::try_from(cursor.into_raw()) else {
            return Ok(None);
        };
        let mut walk = Walk::new(start);
        if !start.alloc.contiguous {
            walk.at = self.dir_hint(dir);
        }
        let mut set = Set::new();
        let next = self.next_set(&mut walk, &mut slot, &mut set).await?;
        if walk.at.cluster != 0 {
            self.set_dir_hint(dir, walk.at);
        }
        let Some(at) = next else {
            *cursor = DirCursor::from_raw(slot as u64);
            return Ok(None);
        };
        name.fill(|buf| names::utf16_to_utf8(set.name_units(), buf).ok_or(NameError::TooLong))
            .map_err(|_| ErrorKind::LimitExceeded)?;
        let file_type = if set.is_dir() { FileType::Dir } else { FileType::File };
        let node = self.id_at(set.offset);
        *cursor = DirCursor::from_raw(slot as u64);
        let _ = at;
        Ok(Some(DirEntry::new(node, file_type, name.len())))
    }

    /// Reads from a file at `offset`. Returns 0 at or past the end. Bytes
    /// past `ValidDataLength` read as zeros. A chain that ends before the
    /// file's size or loops fails with [`ErrorKind::Corrupt`].
    pub async fn read_at(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
        let (_, state) = self.file_node(node).await?;
        if offset >= state.len || buf.is_empty() {
            return Ok(0);
        }
        let count = (state.len - offset).min(buf.len() as u64) as usize;
        let cluster_size = self.geo.cluster_size();
        let mut index = Hint::NONE;
        let mut hint = state.hint;
        let mut done = 0;
        while done < count {
            let pos = offset + done as u64;
            let within = pos % cluster_size;
            let n = ((cluster_size - within) as usize).min(count - done);
            let out = &mut buf[done..done + n];
            if pos >= state.valid {
                out.fill(0);
                done += n;
            } else {
                let valid = (state.valid - pos).min((count - done) as u64) as usize;
                index = self.locate_cluster(state.alloc(), hint, (pos / cluster_size) as u32).await?;
                let at = self.cluster_at(index.cluster)? + within;
                let n = self.run(state.alloc(), &mut index, n.min(valid), valid).await?;
                hint = index;
                read_bytes(&mut self.dev, &mut self.block, at, &mut buf[done..done + n]).await?;
                done += n;
            }
        }
        if index.cluster != 0
            && let Some(state) = self.nodes.get_mut(node)
        {
            state.hint = index;
        }
        Ok(count)
    }

    /// The directory containing `dir`, pinned. The root is its own parent.
    ///
    /// The node table remembers the directory a node was found in; when it
    /// does not, the tree is searched from the root, to a depth of 64.
    pub async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
        if dir == ROOT {
            return Ok(ROOT);
        }
        let (id, state) = self.any_node(dir).await?;
        if !state.dir {
            return Err(ErrorKind::NotADirectory.into());
        }
        let parent = match state.parent {
            UNKNOWN_PARENT => self.search(Target::ParentOf(state.entry)).await?.ok_or(ErrorKind::Corrupt)?,
            parent => parent,
        };
        if let Some(node) = id.and_then(|id| self.nodes.get_mut(id)) {
            node.parent = parent;
        }
        if parent == ROOT_ENTRY {
            return Ok(ROOT);
        }
        if let Some(id) = self.pinned_at(parent) {
            self.nodes.pin(id);
            return Ok(id);
        }
        let mut set = Set::new();
        self.set_at(parent, &mut set).await?;
        Ok(self.intern(&set, UNKNOWN_PARENT)?)
    }

    /// Space usage in clusters. Free space comes from one scan of the
    /// allocation bitmap, which is then kept up to date.
    pub async fn stats(&mut self) -> FsResult<FsStats, D::Error> {
        let free = self.free_count().await?;
        Ok(FsStats::new(self.geo.cluster_count as u64, free as u64, self.geo.cluster_size() as u32))
    }

    /// Passes the clusters of `node`'s allocation to `visit` in order and
    /// returns how many there were. An empty file has none. Fails with
    /// [`ErrorKind::Corrupt`] when a chain leaves the heap, runs into a bad
    /// cluster or is longer than the volume.
    pub async fn cluster_chain(&mut self, node: NodeId, mut visit: impl FnMut(u32)) -> FsResult<u32, D::Error> {
        let (alloc, len) = if node == ROOT {
            (self.root_start().alloc, u64::MAX)
        } else {
            let state = self.node(node).await?;
            (state.alloc(), state.len)
        };
        if alloc.first == 0 {
            return Ok(0);
        }
        let mut cluster = self.check_cluster(alloc.first)?;
        if alloc.contiguous {
            let count = len.div_ceil(self.geo.cluster_size()).min(self.geo.cluster_count as u64) as u32;
            for step in 0..count {
                visit(self.check_cluster(cluster + step)?);
            }
            return Ok(count);
        }
        let mut count = 0u32;
        loop {
            visit(cluster);
            count += 1;
            if count > self.geo.cluster_count {
                return Err(ErrorKind::Corrupt.into());
            }
            match self.next_cluster(cluster).await? {
                Some(next) => cluster = next,
                None => return Ok(count),
            }
        }
    }

    /// Unpins a node. Unknown ids and the root are ignored.
    pub fn forget(&mut self, node: NodeId) {
        if node == ROOT || self.user_pins(node) == 0 {
            return;
        }
        if self.nodes.unpin(node) == Some(0) && self.nodes.get(node).is_some_and(|n| n.unlinked) {
            self.nodes.remove(node);
        }
        if self.nodes.is_empty() {
            self.moved = false;
        }
    }

    /// Where the last listing of `dir` left its chain, [`Hint::NONE`] when
    /// unknown.
    fn dir_hint(&self, dir: NodeId) -> Hint {
        if dir == ROOT {
            return self.root_hint;
        }
        self.nodes.get(dir).map_or(Hint::NONE, |node| node.hint)
    }

    /// Records a position in the chain of `dir`, which only ever grows, for
    /// the next `read_dir_entry` to start from.
    fn set_dir_hint(&mut self, dir: NodeId, hint: Hint) {
        if dir == ROOT {
            self.root_hint = hint;
        } else if let Some(node) = self.nodes.get_mut(dir)
            && node.dir
        {
            node.hint = hint;
        }
    }

    /// Marks a pinned node as open: until the matching
    /// [`close_node`](Self::close_node), `remove` and a replacing `rename`
    /// of it fail with [`ErrorKind::Busy`].
    pub async fn open_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        if node == ROOT {
            return Ok(());
        }
        match self.nodes.get_mut(node) {
            Some(state) if state.unlinked => Err(ErrorKind::NotFound.into()),
            Some(state) => {
                state.opens = state.opens.saturating_add(1);
                Ok(())
            }
            None => Err(ErrorKind::InvalidHandle.into()),
        }
    }

    /// Ends one [`open_node`](Self::open_node). Unknown ids are ignored.
    pub fn close_node(&mut self, node: NodeId) {
        if let Some(state) = self.nodes.get_mut(node) {
            state.opens = state.opens.saturating_sub(1);
        }
    }

    /// Creates `name` in `dir` and pins the new node. Only files and
    /// directories exist on exFAT; other kinds fail with
    /// [`ErrorKind::Unsupported`].
    ///
    /// `meta` sets the attributes, which default to archive for a file, and
    /// the creation, modification and access times, which default to now.
    /// A new directory gets one zeroed cluster. Fails with
    /// [`ErrorKind::AlreadyExists`] when a name matches through the up-case
    /// table, [`ErrorKind::InvalidInput`] or [`ErrorKind::NameTooLong`] for
    /// a name exFAT cannot hold, and [`ErrorKind::NoSpace`] when the volume
    /// or a 256 MiB directory is full.
    pub async fn create(
        &mut self,
        dir: NodeId,
        name: &Name,
        kind: NewNode<'_>,
        meta: &SetMetadata,
    ) -> FsResult<NodeId, D::Error> {
        self.prepare().await?;
        let is_dir = match kind {
            NewNode::File => false,
            NewNode::Dir => true,
            _ => return Err(ErrorKind::Unsupported.into()),
        };
        let (start, dir_entry) = self.dir_info(dir).await?;
        let text = entry_name(name, ErrorKind::InvalidInput)?;
        let units = Units::encode(text)?;
        let hash = self.name_hash(&units).await?;
        let plan = self.plan(start, Some((&units, hash)), 2 + units.entries() as u32, None).await?;
        let placeholder = Node {
            entry: u64::MAX,
            parent: dir_entry,
            first: 0,
            len: 0,
            valid: 0,
            contiguous: false,
            dir: is_dir,
            hint: Hint::NONE,
            dirty: false,
            opens: 0,
            unlinked: false,
        };
        self.nodes.insert(RESERVED, placeholder).map_err(|_| ErrorKind::LimitExceeded)?;
        let created = self.create_set(start, dir_entry, &units, hash, &plan, is_dir, meta).await;
        self.nodes.remove(RESERVED);
        let node = created?;
        let id = self.free_id(node.entry).ok_or(ErrorKind::LimitExceeded)?;
        self.nodes.insert(id, node).map_err(|_| ErrorKind::LimitExceeded)?;
        Ok(id)
    }

    /// Removes the file or empty directory `name` from `dir` and frees its
    /// clusters, with those its Vendor Allocation entries hold. `kind` says
    /// which of the two is expected.
    ///
    /// Fails with [`ErrorKind::IsADirectory`] or
    /// [`ErrorKind::NotADirectory`] when the entry is not of `kind`, with
    /// [`ErrorKind::Busy`] while the node is open, and with
    /// [`ErrorKind::DirectoryNotEmpty`] for a directory with entries. A
    /// pinned node that is not open is removed; its id then answers
    /// [`ErrorKind::NotFound`] until its last `forget`.
    pub async fn remove(&mut self, dir: NodeId, name: &Name, kind: RemoveKind) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let (start, _) = self.dir_info(dir).await?;
        let query = entry_name(name, ErrorKind::NotFound)?;
        let query = Units::query(query).ok_or(ErrorKind::NotFound)?;
        let mut found = Located::new();
        if !self.find(start, &query, &mut found).await? {
            return Err(ErrorKind::NotFound.into());
        }
        let file_type = if found.set.is_dir() { FileType::Dir } else { FileType::File };
        kind.check(file_type)?;
        let pinned = self.pinned_at(found.set.offset);
        if pinned.is_some_and(|id| self.is_open(id)) {
            return Err(ErrorKind::Busy.into());
        }
        let state = match pinned.and_then(|id| self.nodes.get(id)) {
            Some(node) => *node,
            None => Node::from_set(&found.set, UNKNOWN_PARENT),
        };
        if state.dir && !self.dir_is_empty(state.alloc(), state.len).await? {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        if state.first != 0 && !state.contiguous {
            self.pending = Some(Pending::chain(state.first, Owner::Removed(found.set.offset)));
        }
        self.clear_set(&found.set).await?;
        if let Some(id) = pinned {
            self.unlink(id);
        }
        self.free_alloc(state.alloc(), state.len).await?;
        self.free_extras(&found.set).await
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`. The moved node keeps
    /// its `NodeId` when it is pinned. A renamed file gets the archive
    /// attribute; a directory keeps its attributes.
    ///
    /// An existing `to` is replaced unless `flags` has
    /// [`RenameFlags::NO_REPLACE`] ([`ErrorKind::AlreadyExists`]): a file by
    /// a file, an empty directory by a directory; the replaced node's
    /// clusters are freed as by `remove`; an open target fails with
    /// [`ErrorKind::Busy`]. The result is named `to` as given, and takes
    /// the target's entries when it fits them. A rename that changes only
    /// the case rewrites the entry set in place. Moving a directory into
    /// itself or below, or a `to` that `create` would refuse, fails with
    /// [`ErrorKind::InvalidInput`], and unknown flags with
    /// [`ErrorKind::Unsupported`].
    pub async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        flags: RenameFlags,
    ) -> FsResult<(), D::Error> {
        self.prepare().await?;
        if flags.bits() & !RenameFlags::NO_REPLACE.bits() != 0 {
            return Err(ErrorKind::Unsupported.into());
        }
        let (from_start, _) = self.dir_info(from_dir).await?;
        let (to_start, to_entry) = self.dir_info(to_dir).await?;
        let from_text = entry_name(from, ErrorKind::NotFound)?;
        let to_text = entry_name(to, ErrorKind::InvalidInput)?;
        let units = Units::encode(to_text)?;
        let hash = self.name_hash(&units).await?;
        let from_query = Units::query(from_text).ok_or(ErrorKind::NotFound)?;
        let mut src = Located::new();
        if !self.find(from_start, &from_query, &mut src).await? {
            return Err(ErrorKind::NotFound.into());
        }
        let src_id = self.pinned_at(src.set.offset);
        let src_node = match src_id.and_then(|id| self.nodes.get(id)) {
            Some(node) => *node,
            None => Node::from_set(&src.set, UNKNOWN_PARENT),
        };
        if src_node.dir && self.is_within(to_start, src_node.alloc(), src_node.len).await? {
            return Err(ErrorKind::InvalidInput.into());
        }
        let mut target = Located::new();
        let exists = self.find(to_start, &units, &mut target).await?;
        match exists.then_some(&target) {
            Some(target) if target.set.offset == src.set.offset => {
                if target.exact {
                    return Ok(());
                }
                self.rewrite_name(&src.set, &src_node, src_id, &units, hash).await
            }
            Some(target) => {
                if flags.contains(RenameFlags::NO_REPLACE) {
                    return Err(ErrorKind::AlreadyExists.into());
                }
                self.replace(&src, &src_node, src_id, to_start, to_entry, target, &units, hash).await
            }
            None => {
                let plan = self.plan(to_start, None, 2 + units.entries() as u32 + src.set.extras().len() as u32, None).await?;
                self.move_set(&src.set, &src_node, src_id, to_start, to_entry, &plan, &units, hash).await
            }
        }
    }

    /// Writes to a file at `offset`, growing it. Bytes between the old
    /// `ValidDataLength` and `offset` are zeroed first. A volume without
    /// room for the new clusters fails with [`ErrorKind::NoSpace`] and
    /// changes nothing, and a write that would end past `u64::MAX` with
    /// [`ErrorKind::FileTooLarge`].
    ///
    /// The new sizes of a pinned file are written by `publish_node`,
    /// `sync_node` or `sync`, with the modification time and the archive
    /// attribute.
    pub async fn write_at(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, D::Error> {
        self.prepare().await?;
        let (id, state) = self.file_node(node).await?;
        if buf.is_empty() {
            return Ok(0);
        }
        let end = offset.checked_add(buf.len() as u64).ok_or(ErrorKind::FileTooLarge)?;
        let growth = self.cover(&state, end).await?;
        let hint = if growth.alloc == state.alloc() { state.hint } else { Hint::NONE };
        let filled = if offset > state.valid {
            self.fill(growth.alloc, hint, state.valid, None, offset - state.valid).await
        } else {
            Ok(hint)
        };
        let written = match filled {
            Ok(hint) => self.fill(growth.alloc, hint, offset, Some(buf), buf.len() as u64).await,
            Err(err) => Err(err),
        };
        let hint = match written {
            Ok(hint) => hint,
            Err(err) => {
                self.undo_growth(growth).await;
                return Err(err);
            }
        };
        let len = state.len.max(end);
        let valid = state.valid.max(end);
        if let Err(err) = self.publish(id, &state, growth.alloc, len, valid, hint).await {
            self.undo_growth(growth).await;
            return Err(err);
        }
        self.pending = None;
        Ok(buf.len())
    }

    /// Truncates or extends a file. Growth allocates clusters and raises
    /// `DataLength` only, so it reads as zeros without writing them.
    /// Shrinking writes the new sizes at once and frees the clusters past
    /// them. A changed size sets the archive attribute.
    pub async fn set_len(&mut self, node: NodeId, len: u64) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let (id, state) = self.file_node(node).await?;
        if len > state.len {
            let growth = self.cover(&state, len).await?;
            let hint = if growth.alloc == state.alloc() { state.hint } else { Hint::NONE };
            if let Err(err) = self.publish(id, &state, growth.alloc, len, state.valid, hint).await {
                self.undo_growth(growth).await;
                return Err(err);
            }
            self.pending = None;
            return Ok(());
        }
        if len == state.len {
            return Ok(());
        }
        let cluster_size = self.geo.cluster_size();
        let keep = len.div_ceil(cluster_size) as u32;
        let had = state.len.div_ceil(cluster_size) as u32;
        let alloc = if keep == 0 { Alloc { first: 0, contiguous: false } } else { state.alloc() };
        if keep == 0 && state.first != 0 && !state.contiguous {
            self.pending = Some(Pending::chain(state.first, Owner::Entry(state.entry)));
        }
        self.store(id, &state, alloc, len, state.valid.min(len), Hint::NONE).await?;
        if state.first == 0 || keep == had {
            return Ok(());
        }
        if keep == 0 {
            return self.free_alloc(state.alloc(), state.len).await;
        }
        if state.contiguous {
            for cluster in state.first + keep..state.first + had {
                self.set_bit(cluster, false).await?;
            }
            return Ok(());
        }
        let last = self.locate_cluster(state.alloc(), Hint::NONE, keep - 1).await?.cluster;
        if let Some(next) = self.next_cluster(last).await? {
            self.pending = Some(Pending::chain(next, Owner::Cluster(last)));
            self.set_fat(last, raw::FAT_END).await?;
            self.free_chain(next).await?;
        }
        Ok(())
    }

    /// Changes attributes and times. `changed` times, mode and owner are
    /// ignored, since exFAT cannot store them, as are changes to the root.
    /// A pending size is written too.
    pub async fn set_metadata(&mut self, node: NodeId, changes: &SetMetadata) -> FsResult<(), D::Error> {
        self.prepare().await?;
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
        let mut set = Set::new();
        self.set_at(state.entry, &mut set).await?;
        if state.dirty {
            self.touch(&mut set, &state, state.alloc(), state.len, state.valid);
        }
        if let Some(attributes) = changes.attributes() {
            set_attributes(&mut set.raw[0], attributes);
        }
        for (which, time) in [(CREATED, times.created()), (MODIFIED, times.modified()), (ACCESSED, times.accessed())] {
            if let Some(time) = time {
                stamp(&mut set.raw[0], which, time);
            }
        }
        codec::seal(&mut set.raw[..set.count]);
        self.write_set(&set, 2, SetWrite::Update).await?;
        if let Some(id) = id {
            self.clean(id);
        }
        Ok(())
    }

    /// Writes the node's pending sizes and modification time, then flushes
    /// the device.
    pub async fn sync_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        self.publish_node(node).await?;
        self.flush_device().await
    }

    /// Writes the node's pending sizes and modification time to its entry
    /// set, without flushing the device.
    pub async fn publish_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        if node != ROOT {
            let (id, _) = self.any_node(node).await?;
            if let Some(id) = id {
                self.flush_node(id).await?;
            }
        }
        Ok(())
    }

    /// Writes every pending size and modification time and `PercentInUse`,
    /// clears the `VolumeDirty` flag this driver set, then flushes the
    /// device.
    ///
    /// A node whose entry set cannot be read any more does not stop the
    /// others: its pending sizes are dropped, the rest is written and the
    /// device flushed, `VolumeDirty` stays set, and `sync` then fails with
    /// [`ErrorKind::Corrupt`].
    pub async fn sync(&mut self) -> FsResult<(), D::Error> {
        if !self.read_only {
            self.recover().await?;
        }
        let mut corrupt = None;
        while let Some(id) = self.nodes.find(&mut |_, node| node.dirty) {
            match self.flush_node(id).await {
                Err(err) if err.kind() == ErrorKind::Corrupt => {
                    self.clean(id);
                    corrupt = Some(err);
                }
                other => other?,
            }
        }
        if let Some(err) = corrupt {
            self.flush_device().await?;
            return Err(err);
        }
        if self.allocation_changed {
            let free = self.free_count().await?;
            let used = (self.geo.cluster_count - free) as u64;
            let percent = (used * 100 / self.geo.cluster_count as u64) as u8;
            self.write(PERCENT_AT, &[percent]).await?;
            self.geo.percent_in_use = percent;
            self.allocation_changed = false;
        }
        if self.dirty == Dirty::Marked {
            self.write_flags(self.geo.flags & !raw::VOLUME_DIRTY).await?;
            self.dirty = Dirty::Clean;
        }
        self.flush_device().await
    }

    fn now(&self) -> DateTime {
        self.clock.now()
    }

    fn writable(&self) -> Result<(), ErrorKind> {
        if self.read_only { Err(ErrorKind::ReadOnly) } else { Ok(()) }
    }

    /// Checks that the volume is writable and finishes what an interrupted
    /// operation left.
    async fn prepare(&mut self) -> FsResult<(), D::Error> {
        self.writable()?;
        self.nodes.remove(RESERVED);
        self.recover().await
    }

    /// Finishes an entry set write and frees the clusters an interrupted
    /// operation held, unless it linked them. What cannot be finished
    /// because the volume is corrupt is dropped.
    async fn recover(&mut self) -> FsResult<(), D::Error> {
        let result = match self.finish_set().await {
            Ok(()) => self.finish_pending().await,
            Err(err) => Err(err),
        };
        match result {
            Err(err) if err.kind() == ErrorKind::Corrupt => {
                self.pending_set = None;
                self.pending = None;
                Ok(())
            }
            other => other,
        }
    }

    async fn finish_set(&mut self) -> FsResult<(), D::Error> {
        let Some((count, kind)) = self.pending_set.as_ref().map(|set| (set.count as usize, set.kind)) else {
            return Ok(());
        };
        let mut primary = [0u8; ENTRY_SIZE];
        let mut stream = [0u8; ENTRY_SIZE];
        self.read(self.pending_at(0), &mut primary).await?;
        if count > 1 {
            self.read(self.pending_at(1), &mut stream).await?;
        }
        let whole = primary[0] == raw::ENTRY_FILE && 1 + primary[1] as usize == count;
        let hash = if whole { self.pending_hash(count, &stream).await? } else { None };
        match kind {
            SetWrite::Insert if hash.is_some() && self.pending_checksum(count, &primary, &stream).await? == le16(&primary, 2) => {}
            SetWrite::Insert | SetWrite::Clear => {
                for index in 0..count {
                    let at = self.pending_at(index);
                    let mut kind = [0u8; 1];
                    self.read(at, &mut kind).await?;
                    if kind[0] & raw::IN_USE != 0 {
                        self.write(at, &[kind[0] & !raw::IN_USE]).await?;
                    }
                }
                if let SetWrite::Clear = kind
                    && let Some(id) = self.pinned_at(self.pending_at(0))
                {
                    self.unlink(id);
                }
            }
            SetWrite::Update => {
                if let Some(hash) = hash {
                    stream[4..6].copy_from_slice(&hash.to_le_bytes());
                    let sum = self.pending_checksum(count, &primary, &stream).await?;
                    primary[2..4].copy_from_slice(&sum.to_le_bytes());
                    let at = [self.pending_at(0), self.pending_at(1)];
                    self.put_entries(&at, &[primary, stream], true).await?;
                    self.reload_dir(at[0], &stream);
                }
            }
        }
        self.pending_set = None;
        Ok(())
    }

    /// Offset of entry `index` of the set being written.
    fn pending_at(&self, index: usize) -> u64 {
        self.pending_set.as_ref().map_or(0, |set| set.at[index])
    }

    /// The `NameHash` of the set being written, whose `count` entries
    /// start with a File entry, or `None` when they do not form a File
    /// entry set.
    async fn pending_hash(&mut self, count: usize, stream: &RawEntry) -> FsResult<Option<u16>, D::Error> {
        let len = stream[3] as usize;
        let names = len.div_ceil(raw::NAME_UNITS_PER_ENTRY);
        if !(3..=MAX_SET).contains(&count) || stream[0] != raw::ENTRY_STREAM || len == 0 || 2 + names > count {
            return Ok(None);
        }
        let mut hash = 0;
        for index in 2..count {
            let mut entry = [0u8; ENTRY_SIZE];
            self.read(self.pending_at(index), &mut entry).await?;
            if index < 2 + names {
                if entry[0] != raw::ENTRY_NAME {
                    return Ok(None);
                }
                let units = (len - (index - 2) * raw::NAME_UNITS_PER_ENTRY).min(raw::NAME_UNITS_PER_ENTRY);
                for unit in 0..units {
                    hash = codec::hash_unit(hash, self.upcase_unit(le16(&entry, 2 + unit * 2)).await?);
                }
            } else if entry[0] & raw::CATEGORY_SECONDARY == 0 || entry[0] & raw::IN_USE == 0 {
                return Ok(None);
            }
        }
        Ok(Some(hash))
    }

    /// The checksum of the set being written, with `primary` and `stream`
    /// in place of its first two entries.
    async fn pending_checksum(&mut self, count: usize, primary: &RawEntry, stream: &RawEntry) -> FsResult<u16, D::Error> {
        let mut sum = codec::set_checksum(&[*primary, *stream]);
        for index in 2..count {
            let mut entry = [0u8; ENTRY_SIZE];
            self.read(self.pending_at(index), &mut entry).await?;
            sum = entry.iter().fold(sum, |sum, &byte| sum.rotate_right(1).wrapping_add(byte as u16));
        }
        Ok(sum)
    }

    /// Takes a pinned directory's allocation and size from its stream
    /// entry, after an interrupted write to it.
    fn reload_dir(&mut self, offset: u64, stream: &RawEntry) {
        if let Some(id) = self.pinned_at(offset)
            && let Some(node) = self.nodes.get_mut(id)
            && node.dir
        {
            node.first = le32(stream, 20);
            node.contiguous = stream[1] & raw::NO_FAT_CHAIN != 0;
            node.len = le64(stream, 24);
            node.valid = le64(stream, 8).min(node.len);
            node.hint = Hint::NONE;
        }
    }

    async fn finish_pending(&mut self) -> FsResult<(), D::Error> {
        let Some(pending) = self.pending else {
            return Ok(());
        };
        let mut owned = pending.head != 0 && self.links(pending.owner, pending.head).await?;
        if let Owner::Tail(tail) = pending.owner
            && owned
        {
            self.set_fat(tail, raw::FAT_END).await?;
            owned = false;
        }
        match pending.owner {
            Owner::Entry(offset) => self.reconcile(offset).await?,
            Owner::Removed(offset) if !owned => {
                if let Some(id) = self.pinned_at(offset)
                    && self.nodes.get(id).is_some_and(|node| node.first == pending.head)
                {
                    self.unlink(id);
                }
            }
            _ => {}
        }
        if !owned {
            self.pending = Some(Pending {
                owner: Owner::None,
                ..pending
            });
            if pending.extra != 0 {
                self.reclaim(pending.extra, pending.head).await?;
            }
            if pending.head != 0 {
                self.reclaim(pending.head, 0).await?;
            }
        }
        self.pending = None;
        Ok(())
    }

    /// Whether `owner` links the chain at `head` on disk.
    async fn links(&mut self, owner: Owner, head: u32) -> FsResult<bool, D::Error> {
        Ok(match owner {
            Owner::None => false,
            Owner::Cluster(prev) | Owner::Tail(prev) => {
                self.geo.is_cluster(prev) && self.fat_entry(prev).await? == head
            }
            Owner::Entry(offset) | Owner::Removed(offset) => {
                let mut set = Set::new();
                match self.set_at(offset, &mut set).await {
                    Ok(()) => set.first() == head,
                    Err(err) if err.kind() == ErrorKind::Corrupt => false,
                    Err(err) => return Err(err),
                }
            }
        })
    }

    /// Brings a pinned node at `offset` in line with its entry set after an
    /// interrupted write to it.
    async fn reconcile(&mut self, offset: u64) -> FsResult<(), D::Error> {
        let Some(id) = self.pinned_at(offset) else {
            return Ok(());
        };
        let mut set = Set::new();
        match self.set_at(offset, &mut set).await {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::Corrupt => return Ok(()),
            Err(err) => return Err(err),
        }
        let stale = self.nodes.get(id).is_some_and(|node| node.first != set.first());
        if stale {
            self.clean(id);
            if let Some(node) = self.nodes.get_mut(id) {
                node.first = set.first();
                node.contiguous = set.flags() & raw::NO_FAT_CHAIN != 0;
                node.len = set.data_len();
                node.valid = set.valid_len().min(set.data_len());
                node.hint = Hint::NONE;
            }
        }
        Ok(())
    }

    /// Frees the clusters of a chain from `head` for as long as they are
    /// allocated, stopping at a free cluster, the end, or a link out of
    /// range. `keep` stays pending once the chain ends.
    async fn reclaim(&mut self, head: u32, keep: u32) -> FsResult<(), D::Error> {
        let mut cluster = head;
        for _ in 0..self.geo.cluster_count {
            if !self.geo.is_cluster(cluster) || !self.bit(cluster).await? {
                break;
            }
            let next = Some(self.fat_entry(cluster).await?).filter(|&next| self.geo.is_cluster(next));
            self.pending = Some(Pending {
                head: next.unwrap_or(keep),
                extra: cluster,
                owner: Owner::None,
            });
            self.set_bit(cluster, false).await?;
            match next {
                Some(next) => cluster = next,
                None => break,
            }
        }
        Ok(())
    }

    fn user_pins(&self, id: NodeId) -> u32 {
        let dirty = self.nodes.get(id).is_some_and(|node| node.dirty);
        self.nodes.pins(id).saturating_sub(dirty as u32)
    }

    fn is_open(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| node.opens > 0)
    }

    fn unlink(&mut self, id: NodeId) {
        self.clean(id);
        if self.nodes.pins(id) == 0 {
            self.nodes.remove(id);
        } else if let Some(node) = self.nodes.get_mut(id) {
            self.moved = true;
            node.unlinked = true;
            node.entry = u64::MAX;
            node.first = 0;
            node.len = 0;
            node.valid = 0;
        }
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

    async fn write_flags(&mut self, flags: u16) -> FsResult<(), D::Error> {
        let result = write_bytes(&mut self.dev, &mut self.block, VOLUME_FLAGS_AT, Some(&flags.to_le_bytes()), 2).await;
        self.note_refusal(&result);
        result?;
        self.geo.flags = flags;
        Ok(())
    }

    fn note_refusal<V>(&mut self, result: &FsResult<V, D::Error>) {
        if let Err(err) = result
            && err.kind() == ErrorKind::ReadOnly
        {
            self.read_only = true;
        }
    }

    pub(super) async fn put(&mut self, offset: u64, data: Option<&[u8]>, len: u64) -> FsResult<(), D::Error> {
        self.begin_write().await?;
        let len = usize::try_from(len).map_err(|_| ErrorKind::LimitExceeded)?;
        let result = write_bytes(&mut self.dev, &mut self.block, offset, data, len).await;
        self.note_refusal(&result);
        result
    }

    /// Fails on a read-only volume, and sets `VolumeDirty` before the first
    /// write.
    async fn begin_write(&mut self) -> FsResult<(), D::Error> {
        self.writable()?;
        if self.dirty == Dirty::Clean {
            self.write_flags((self.geo.flags | raw::VOLUME_DIRTY) & !raw::VOLUME_CLEAR_TO_ZERO).await?;
            self.dirty = Dirty::Marked;
        }
        Ok(())
    }

    /// Writes the buffered block back as device block `index`. Call
    /// [`begin_write`](Self::begin_write) before loading it.
    async fn write_block(&mut self, index: u64) -> FsResult<(), D::Error> {
        self.block.cached = None;
        let result = self
            .dev
            .write_blocks(hadris_storage::BlockIndex::new(index), &self.block.data[..self.block.size])
            .await;
        self.note_refusal(&result);
        result?;
        self.block.cached = Some(index);
        Ok(())
    }

    async fn write(&mut self, offset: u64, data: &[u8]) -> FsResult<(), D::Error> {
        self.put(offset, Some(data), data.len() as u64).await
    }

    async fn flush_device(&mut self) -> FsResult<(), D::Error> {
        if self.read_only {
            return Ok(());
        }
        let result = self.dev.flush().await;
        self.note_refusal(&result);
        result
    }

    pub(super) async fn read(&mut self, offset: u64, out: &mut [u8]) -> FsResult<(), D::Error> {
        read_bytes(&mut self.dev, &mut self.block, offset, out).await
    }

    /// Records a node's allocation and sizes in `set` and marks it modified
    /// now.
    fn touch(&self, set: &mut Set, node: &Node, alloc: Alloc, len: u64, valid: u64) {
        set_stream(&mut set.raw[1], alloc, len, valid);
        let now = self.now();
        stamp(&mut set.raw[0], MODIFIED, now);
        stamp(&mut set.raw[0], ACCESSED, now);
        if !node.dir {
            let attributes = le16(&set.raw[0], 4) | raw::ATTR_ARCHIVE;
            set.raw[0][4..6].copy_from_slice(&attributes.to_le_bytes());
        }
        codec::seal(&mut set.raw[..set.count]);
    }

    /// Writes a file's allocation, sizes and modification time to its entry
    /// set and updates its table state.
    async fn store(
        &mut self,
        id: Option<NodeId>,
        state: &Node,
        alloc: Alloc,
        len: u64,
        valid: u64,
        hint: Hint,
    ) -> FsResult<(), D::Error> {
        let mut set = Set::new();
        self.set_at(state.entry, &mut set).await?;
        self.touch(&mut set, state, alloc, len, valid);
        self.write_set(&set, 2, SetWrite::Update).await?;
        if let Some(id) = id {
            if let Some(node) = self.nodes.get_mut(id) {
                node.first = alloc.first;
                node.contiguous = alloc.contiguous;
                node.len = len;
                node.valid = valid;
                node.hint = hint;
            }
            self.clean(id);
        }
        Ok(())
    }

    /// Records a file's new sizes: in the table for a pinned node whose
    /// allocation starts where it did, else in its entry set.
    async fn publish(
        &mut self,
        id: Option<NodeId>,
        state: &Node,
        alloc: Alloc,
        len: u64,
        valid: u64,
        hint: Hint,
    ) -> FsResult<(), D::Error> {
        match id {
            Some(id) if alloc == state.alloc() => {
                if let Some(node) = self.nodes.get_mut(id) {
                    node.len = len;
                    node.valid = valid;
                    node.hint = hint;
                }
                self.mark_dirty(id);
                Ok(())
            }
            _ => self.store(id, state, alloc, len, valid, hint).await,
        }
    }

    async fn flush_node(&mut self, id: NodeId) -> FsResult<(), D::Error> {
        match self.nodes.get(id).copied() {
            Some(node) if node.dirty => {
                self.store(Some(id), &node, node.alloc(), node.len, node.valid, node.hint).await
            }
            _ => Ok(()),
        }
    }

    /// Writes the first `count` entries of `set` back, the File entry last.
    async fn write_set(&mut self, set: &Set, count: usize, kind: SetWrite) -> FsResult<(), D::Error> {
        self.pending_set = Some(SetPending::new(&set.at[..set.count], kind));
        let count = count.min(set.count);
        self.put_entries(&set.at[..count], &set.raw[..count], true).await?;
        self.pending_set = None;
        Ok(())
    }

    /// Marks every entry of `set` unused, the File entry first.
    async fn clear_set(&mut self, set: &Set) -> FsResult<(), D::Error> {
        self.pending_set = Some(SetPending::new(&set.at[..set.count], SetWrite::Clear));
        let mut entries = set.raw;
        for entry in &mut entries[..set.count] {
            entry[0] &= !raw::IN_USE;
        }
        self.put_entries(&set.at[..set.count], &entries[..set.count], false).await?;
        self.pending_set = None;
        Ok(())
    }

    /// Writes `entries` at the offsets `at`, one device write for each run
    /// of entries that follow each other within one device block, so a set
    /// inside one block is written at once. With `primary_last` the run
    /// holding the first entry is written last, else first.
    async fn put_entries(&mut self, at: &[u64], entries: &[RawEntry], primary_last: bool) -> FsResult<(), D::Error> {
        let block = self.block.size as u64;
        let joined = |index: usize| {
            index > 0 && at[index] == at[index - 1] + ENTRY_SIZE as u64 && at[index] / block == at[index - 1] / block
        };
        let count = at.len();
        let mut done = 0;
        while done < count {
            let (from, to) = if primary_last {
                let to = count - done;
                let mut from = to - 1;
                while joined(from) {
                    from -= 1;
                }
                (from, to)
            } else {
                let from = done;
                let mut to = from + 1;
                while to < count && joined(to) {
                    to += 1;
                }
                (from, to)
            };
            self.write(at[from], entries[from..to].as_flattened()).await?;
            done += to - from;
        }
        Ok(())
    }

    /// The table id and state of a node that is not the root.
    async fn any_node(&mut self, id: NodeId) -> FsResult<(Option<NodeId>, Node), D::Error> {
        if id == ROOT {
            return Err(ErrorKind::InvalidHandle.into());
        }
        if let Some(node) = self.nodes.get(id) {
            if node.unlinked {
                return Err(ErrorKind::NotFound.into());
            }
            return Ok((Some(id), *node));
        }
        let mut set = Set::new();
        self.unpinned(id, &mut set).await?;
        match self.pinned_at(set.offset) {
            Some(pinned) => Ok((Some(pinned), *self.nodes.get(pinned).ok_or(ErrorKind::Corrupt)?)),
            None => Ok((None, Node::from_set(&set, UNKNOWN_PARENT))),
        }
    }

    async fn node(&mut self, id: NodeId) -> FsResult<Node, D::Error> {
        Ok(self.any_node(id).await?.1)
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

    pub(super) fn root_start(&self) -> DirStart {
        DirStart {
            alloc: Alloc { first: self.geo.root, contiguous: false },
            size: u64::MAX,
        }
    }

    /// Where a directory's entries are, and the offset of its File entry.
    async fn dir_info(&mut self, dir: NodeId) -> FsResult<(DirStart, u64), D::Error> {
        if dir == ROOT {
            return Ok((self.root_start(), ROOT_ENTRY));
        }
        let node = self.node(dir).await?;
        if !node.dir {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok((DirStart { alloc: node.alloc(), size: node.len }, node.entry))
    }

    pub(super) fn check_cluster(&self, cluster: u32) -> Result<u32, ErrorKind> {
        if self.geo.is_cluster(cluster) { Ok(cluster) } else { Err(ErrorKind::Corrupt) }
    }

    pub(super) fn cluster_at(&self, cluster: u32) -> Result<u64, ErrorKind> {
        self.geo.cluster_offset(cluster).ok_or(ErrorKind::Corrupt)
    }

    /// The pinned node whose File entry is at `offset`. Unless a pinned
    /// node has moved, that is the tier-0 id of the slot, found by key.
    fn pinned_at(&self, offset: u64) -> Option<NodeId> {
        if !self.moved {
            let id = NodeId::new(offset / ENTRY_SIZE as u64);
            return self.nodes.get(id).is_some_and(|node| node.entry == offset).then_some(id);
        }
        self.nodes.find(&mut |_, node| node.entry == offset)
    }

    fn free_id(&self, offset: u64) -> Option<NodeId> {
        let slot = offset / ENTRY_SIZE as u64;
        (0..=MAX_TIER)
            .map(|tier| NodeId::new(tier << SLOT_BITS | slot))
            .find(|&id| self.nodes.get(id).is_none())
    }

    fn id_at(&self, offset: u64) -> NodeId {
        self.pinned_at(offset)
            .or_else(|| self.free_id(offset))
            .unwrap_or(NodeId::new(offset / ENTRY_SIZE as u64))
    }

    /// Pins the node whose entry set is `set`, found in the directory whose
    /// File entry is at `parent`.
    fn intern(&mut self, set: &Set, parent: u64) -> Result<NodeId, ErrorKind> {
        if let Some(id) = self.pinned_at(set.offset) {
            self.nodes.pin(id);
            if let Some(node) = self.nodes.get_mut(id)
                && node.parent == UNKNOWN_PARENT
            {
                node.parent = parent;
            }
            return Ok(id);
        }
        let id = self.free_id(set.offset).ok_or(ErrorKind::LimitExceeded)?;
        self.nodes
            .insert(id, Node::from_set(set, parent))
            .map_err(|_| ErrorKind::LimitExceeded)?;
        Ok(id)
    }

    /// Reads the entry set of an id that is not pinned, decoded from its
    /// location, into `set`.
    async fn unpinned(&mut self, id: NodeId, set: &mut Set) -> FsResult<(), D::Error> {
        let raw_id = id.get();
        if raw_id == ROOT.get() || raw_id >= RESERVED.get() {
            return Err(ErrorKind::InvalidHandle.into());
        }
        let offset = (raw_id & SLOT_MASK) * ENTRY_SIZE as u64;
        if self.geo.cluster_of(offset).is_none() {
            return Err(ErrorKind::InvalidHandle.into());
        }
        match self.set_at(offset, set).await {
            Err(err) if err.kind() == ErrorKind::Corrupt => Err(ErrorKind::InvalidHandle.into()),
            other => other,
        }
    }

    /// Reads the entry set whose File entry is at `offset` into `set`.
    ///
    /// The directory holding it is not known, so where the set crosses
    /// into another cluster both the FAT's next cluster and the one after
    /// are tried, and the one whose entries make a valid set wins.
    pub(super) async fn set_at(&mut self, offset: u64, set: &mut Set) -> FsResult<(), D::Error> {
        let mut primary = [0u8; ENTRY_SIZE];
        self.read(offset, &mut primary).await?;
        if primary[0] != raw::ENTRY_FILE || !(2..MAX_SET).contains(&(primary[1] as usize)) {
            return Err(ErrorKind::Corrupt.into());
        }
        let count = 1 + primary[1] as usize;
        for contiguous in [false, true] {
            set.offset = offset;
            set.count = count;
            set.raw[0] = primary;
            set.at[0] = offset;
            let mut at = offset;
            let mut ok = true;
            for index in 1..count {
                at += ENTRY_SIZE as u64;
                if at % self.geo.cluster_size() == self.geo.heap_start % self.geo.cluster_size() {
                    let cluster = self.geo.cluster_of(at - 1).ok_or(ErrorKind::Corrupt)?;
                    let next = if contiguous {
                        Some(cluster + 1).filter(|&next| self.geo.is_cluster(next))
                    } else {
                        let value = self.fat_entry(cluster).await?;
                        self.geo.is_cluster(value).then_some(value)
                    };
                    let Some(next) = next else {
                        ok = false;
                        break;
                    };
                    at = self.cluster_at(next)?;
                }
                set.at[index] = at;
                self.read(at, &mut set.raw[index]).await?;
            }
            if ok && parse_set(&set.raw[..count]) {
                return Ok(());
            }
            if set.at[1..count].windows(2).all(|pair| pair[1] == pair[0] + ENTRY_SIZE as u64)
                && set.at[1] == offset + ENTRY_SIZE as u64
            {
                break;
            }
        }
        Err(ErrorKind::Corrupt.into())
    }

    /// Byte offset of directory slot `slot`, or `None` past the end.
    pub(super) async fn slot_offset(&mut self, walk: &mut Walk, slot: u32) -> FsResult<Option<u64>, D::Error> {
        let bytes = slot as u64 * ENTRY_SIZE as u64;
        let start = walk.start;
        if start.alloc.first == 0 || bytes >= start.size || bytes >= raw::MAX_DIRECTORY_SIZE {
            return Ok(None);
        }
        let want = (bytes >> self.geo.cluster_shift) as u32;
        let within = bytes & (self.geo.cluster_size() - 1);
        let cluster = if start.alloc.contiguous {
            self.check_cluster(start.alloc.first.checked_add(want).ok_or(ErrorKind::Corrupt)?)?
        } else {
            if walk.at.cluster == 0 || want < walk.at.index {
                walk.at = Hint::start(self.check_cluster(start.alloc.first)?);
            }
            while walk.at.index < want {
                match self.next_cluster(walk.at.cluster).await? {
                    Some(next) if walk.at.advance(next) => {}
                    Some(_) => return Err(ErrorKind::Corrupt.into()),
                    None => {
                        walk.last = walk.at.cluster;
                        return Ok(None);
                    }
                }
            }
            walk.at.cluster
        };
        walk.last = cluster;
        Ok(Some(self.cluster_at(cluster)? + within))
    }

    /// Reads the entry set whose File entry is directory slot `slot` into
    /// `set`. False when the entries there are not a valid set.
    async fn read_set(&mut self, walk: &mut Walk, slot: u32, primary: RawEntry, offset: u64, set: &mut Set) -> FsResult<bool, D::Error> {
        let count = 1 + primary[1] as usize;
        if !(3..=MAX_SET).contains(&count) {
            return Ok(false);
        }
        set.offset = offset;
        set.count = count;
        set.raw[0] = primary;
        set.at[0] = offset;
        for index in 1..count {
            let Some(at) = self.slot_offset(walk, slot + index as u32).await? else {
                return Ok(false);
            };
            set.at[index] = at;
            self.read(at, &mut set.raw[index]).await?;
        }
        Ok(parse_set(&set.raw[..count]))
    }

    /// Scans from `slot` to the next valid File entry set, reads it into
    /// `set`, and leaves `slot` after it. Returns the set's slot.
    async fn next_set(&mut self, walk: &mut Walk, slot: &mut u32, set: &mut Set) -> FsResult<Option<u32>, D::Error> {
        while let Some(offset) = self.slot_offset(walk, *slot).await? {
            let at = *slot;
            let mut entry = [0u8; ENTRY_SIZE];
            self.read(offset, &mut entry).await?;
            match entry[0] {
                raw::ENTRY_END => return Ok(None),
                raw::ENTRY_FILE => {
                    if self.read_set(walk, at, entry, offset, set).await? {
                        *slot = at + set.count as u32;
                        return Ok(Some(at));
                    }
                }
                _ => {}
            }
            *slot = at + 1;
        }
        Ok(None)
    }

    /// Up-cases one code unit through the volume's table.
    pub(super) async fn upcase_unit(&mut self, unit: u16) -> FsResult<u16, D::Error> {
        let page = (unit >> 8) as usize;
        let low = (unit & 0xFF) as usize;
        if page == 0 {
            return Ok(self.upcase.page0[low]);
        }
        if self.upcase.is_identity(page) {
            return Ok(unit);
        }
        if let Some((_, units)) = self.upcase.cache.iter().find(|(cached, _)| *cached as usize == page) {
            return Ok(units[low]);
        }
        let slot = self.upcase.victim;
        self.upcase.victim = (slot + 1) % UPCASE_CACHE;
        self.upcase.cache[slot].0 = 0;
        let mut units = [0u16; 256];
        for (index, value) in units.iter_mut().enumerate() {
            *value = (page << 8 | index) as u16;
        }
        let code = (page as u32) << 8;
        let mut decoder = UpcaseDecoder::resume(self.upcase.starts[page], code);
        let mut emit = |at: u32, upper: u16, _| {
            if at >> 8 == page as u32 {
                units[(at & 0xFF) as usize] = upper;
            }
        };
        decoder.drain(&mut emit);
        let extent = self.upcase.extent;
        let mut hint = Hint::NONE;
        while decoder.code < code + 256 && (decoder.next as u64) * 2 + 1 < extent.len {
            let pos = decoder.next as u64 * 2;
            let alloc = Alloc { first: extent.first, contiguous: extent.contiguous };
            hint = self.locate_cluster(alloc, hint, (pos >> self.geo.cluster_shift) as u32).await?;
            let at = self.cluster_at(hint.cluster)? + (pos & (self.geo.cluster_size() - 1));
            let mut pair = [0u8; 2];
            self.read(at, &mut pair).await?;
            decoder.feed(u16::from_le_bytes(pair), &mut emit);
        }
        self.upcase.cache[slot] = (page as u16, units);
        Ok(units[low])
    }

    /// The `NameHash` of `name`, up-cased through the volume's table.
    async fn name_hash(&mut self, name: &Units) -> FsResult<u16, D::Error> {
        let mut hash = 0;
        for &unit in name.as_slice() {
            hash = codec::hash_unit(hash, self.upcase_unit(unit).await?);
        }
        Ok(hash)
    }

    /// Whether `set` is named `name` up to case.
    async fn named(&mut self, set: &Set, name: &Units) -> FsResult<bool, D::Error> {
        if set.name_len() != name.len {
            return Ok(false);
        }
        for (unit, &other) in set.name_units().zip(name.as_slice()) {
            if unit != other && self.upcase_unit(unit).await? != self.upcase_unit(other).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Finds the entry set named `query`, up to case, fills `found` and
    /// returns true when there is one. Sets whose stream entry records
    /// another name length or hash are passed over unread.
    async fn find(&mut self, start: DirStart, query: &Units, found: &mut Located) -> FsResult<bool, D::Error> {
        let hash = self.name_hash(query).await?;
        let mut walk = Walk::new(start);
        let mut slot = 0;
        while let Some(offset) = self.slot_offset(&mut walk, slot).await? {
            let at = slot;
            slot += 1;
            let mut entry = [0u8; ENTRY_SIZE];
            self.read(offset, &mut entry).await?;
            match entry[0] {
                raw::ENTRY_END => break,
                raw::ENTRY_FILE if self.may_be_named(&mut walk, at, query, hash).await? => {
                    if !self.read_set(&mut walk, at, entry, offset, &mut found.set).await? {
                        continue;
                    }
                    if self.named(&found.set, query).await? {
                        found.slot = at;
                        found.exact = found.set.name_units().eq(query.as_slice().iter().copied());
                        return Ok(true);
                    }
                    slot = at + found.set.count as u32;
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Whether the set whose File entry is slot `slot` may be named
    /// `name`: false only when its stream entry records another name
    /// length or name hash. `hash` is the `NameHash` of `name`.
    async fn may_be_named(&mut self, walk: &mut Walk, slot: u32, name: &Units, hash: u16) -> FsResult<bool, D::Error> {
        let Some(at) = self.slot_offset(walk, slot + 1).await? else {
            return Ok(true);
        };
        let mut stream = [0u8; ENTRY_SIZE];
        self.read(at, &mut stream).await?;
        Ok(stream[0] != raw::ENTRY_STREAM || (stream[3] as usize == name.len && le16(&stream, 4) == hash))
    }

    async fn find_label(&mut self) -> FsResult<Option<(u64, RawEntry)>, D::Error> {
        let mut walk = Walk::new(self.root_start());
        let mut slot = 0;
        while let Some(offset) = self.slot_offset(&mut walk, slot).await? {
            let mut entry = [0u8; ENTRY_SIZE];
            self.read(offset, &mut entry).await?;
            match entry[0] {
                raw::ENTRY_END => break,
                raw::ENTRY_LABEL => return Ok(Some((offset, entry))),
                _ => {}
            }
            slot += 1;
        }
        Ok(None)
    }

    async fn dir_is_empty(&mut self, alloc: Alloc, len: u64) -> FsResult<bool, D::Error> {
        let mut walk = Walk::new(DirStart { alloc, size: len });
        let mut slot = 0;
        while let Some(offset) = self.slot_offset(&mut walk, slot).await? {
            let mut entry = [0u8; ENTRY_SIZE];
            self.read(offset, &mut entry).await?;
            match entry[0] {
                raw::ENTRY_END => break,
                kind if kind & raw::IN_USE != 0 && kind & raw::CATEGORY_SECONDARY == 0 => return Ok(false),
                _ => {}
            }
            slot += 1;
        }
        Ok(true)
    }

    /// Searches the tree from the root, to a depth of [`MAX_DEPTH`].
    async fn search(&mut self, target: Target) -> FsResult<Option<u64>, D::Error> {
        let (start, entry) = match target {
            Target::Below(start, _) => (start, u64::MAX),
            Target::ParentOf(_) => (self.root_start(), ROOT_ENTRY),
        };
        let mut stack = [(start, entry, 0u32); MAX_DEPTH];
        let mut depth = 1;
        let mut deeper = false;
        let mut set = Set::new();
        while depth > 0 {
            let (dir, dir_entry, slot) = stack[depth - 1];
            let mut walk = Walk::new(dir);
            let mut next = slot;
            if self.next_set(&mut walk, &mut next, &mut set).await?.is_none() {
                depth -= 1;
                continue;
            }
            stack[depth - 1].2 = next;
            match target {
                Target::ParentOf(offset) if set.offset == offset => return Ok(Some(dir_entry)),
                Target::Below(_, first) if set.is_dir() && set.first() == first => return Ok(Some(set.offset)),
                _ => {}
            }
            if set.is_dir() && set.first() != 0 {
                if depth == MAX_DEPTH {
                    deeper = true;
                } else {
                    stack[depth] = (DirStart { alloc: set.alloc(), size: set.data_len() }, set.offset, 0);
                    depth += 1;
                }
            }
        }
        if deeper {
            return Err(ErrorKind::LimitExceeded.into());
        }
        Ok(None)
    }

    /// Whether the directory at `dir` is the one at `ancestor` or below it.
    async fn is_within(&mut self, dir: DirStart, ancestor: Alloc, len: u64) -> FsResult<bool, D::Error> {
        if dir.alloc.first == 0 || ancestor.first == 0 {
            return Ok(false);
        }
        if dir.alloc.first == ancestor.first {
            return Ok(true);
        }
        let below = DirStart { alloc: ancestor, size: len };
        Ok(self.search(Target::Below(below, dir.alloc.first)).await?.is_some())
    }

    /// Finds room for an entry set of `needed` entries in `dir`. With
    /// `check`, a name and its `NameHash`, an entry set with that name fails with
    /// [`ErrorKind::AlreadyExists`]. The entries of `reuse` count as free.
    async fn plan(
        &mut self,
        dir: DirStart,
        check: Option<(&Units, u16)>,
        needed: u32,
        reuse: Option<&Set>,
    ) -> FsResult<Plan, D::Error> {
        if needed as usize > MAX_SET {
            return Err(ErrorKind::NameTooLong.into());
        }
        let mut walk = Walk::new(dir);
        let mut slot = 0u32;
        let mut end = false;
        let (mut run_start, mut run_len) = (0, 0);
        let mut found = None;
        let mut scanned = Set::new();
        while let Some(offset) = self.slot_offset(&mut walk, slot).await? {
            let reused = reuse.is_some_and(|set| set.at[..set.count].contains(&offset));
            let mut entry = [0u8; ENTRY_SIZE];
            if !end && !reused {
                self.read(offset, &mut entry).await?;
            }
            let mut advance = 1;
            let free = if end || reused {
                true
            } else if entry[0] == raw::ENTRY_END {
                end = true;
                true
            } else if entry[0] == raw::ENTRY_FILE {
                if let Some((query, hash)) = check
                    && self.may_be_named(&mut walk, slot, query, hash).await?
                    && self.read_set(&mut walk, slot, entry, offset, &mut scanned).await?
                {
                    if self.named(&scanned, query).await? {
                        return Err(ErrorKind::AlreadyExists.into());
                    }
                    advance = scanned.count as u32;
                }
                false
            } else {
                entry[0] & raw::IN_USE == 0
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
            slot += advance;
            if found.is_some() && (end || check.is_none()) {
                break;
            }
        }
        if let Some(start) = found {
            return Ok(Plan { start, grow: 0 });
        }
        let start = if run_len == 0 { slot } else { run_start };
        let per_cluster = (self.geo.cluster_size() / ENTRY_SIZE as u64) as u32;
        let grow = (needed - run_len).div_ceil(per_cluster);
        let size_after = ((start as u64 + needed as u64) * ENTRY_SIZE as u64).div_ceil(self.geo.cluster_size())
            * self.geo.cluster_size();
        if size_after > raw::MAX_DIRECTORY_SIZE {
            return Err(ErrorKind::NoSpace.into());
        }
        Ok(Plan { start, grow })
    }

    /// Writes `entries` into the run `plan` found in `dir`, growing it
    /// first when needed; secondary entries go first. Returns the offset
    /// of the first entry.
    /// A pending allocation is owned by the new set from its write on.
    async fn insert(&mut self, dir: DirStart, dir_entry: u64, plan: &Plan, entries: &[RawEntry]) -> FsResult<u64, D::Error> {
        let dir = if plan.grow > 0 { self.grow_dir(dir, dir_entry, plan.grow).await? } else { dir };
        let mut walk = Walk::new(dir);
        let mut at = [0u64; MAX_SET];
        for (index, slot) in at.iter_mut().enumerate().take(entries.len()) {
            *slot = self
                .slot_offset(&mut walk, plan.start + index as u32)
                .await?
                .ok_or(ErrorKind::Corrupt)?;
        }
        let at = &at[..entries.len()];
        if let Some(pending) = self.pending.as_mut()
            && pending.owner == Owner::None
        {
            pending.owner = Owner::Entry(at[0]);
        }
        if entries.len() > 1 {
            self.pending_set = Some(SetPending::new(at, SetWrite::Insert));
        }
        if let Err(err) = self.put_entries(at, entries, true).await {
            let _ = self.finish_set().await;
            return Err(err);
        }
        self.pending_set = None;
        Ok(at[0])
    }

    /// Grows `dir` by what `plan` needs, so the entries can then be written
    /// with no other allocation pending. Returns the directory and the plan
    /// without growth.
    async fn grow_first(&mut self, dir: DirStart, dir_entry: u64, plan: &Plan) -> FsResult<(DirStart, Plan), D::Error> {
        let dir = if plan.grow > 0 { self.grow_dir(dir, dir_entry, plan.grow).await? } else { dir };
        Ok((dir, Plan { start: plan.start, grow: 0 }))
    }

    /// Appends `count` zeroed clusters to a directory and records its new
    /// size in its entry set. Returns where its entries now are.
    async fn grow_dir(&mut self, dir: DirStart, dir_entry: u64, count: u32) -> FsResult<DirStart, D::Error> {
        let cluster_size = self.geo.cluster_size();
        let old_len = if dir_entry == ROOT_ENTRY {
            0
        } else {
            dir.size
        };
        let old = Node {
            entry: dir_entry,
            parent: UNKNOWN_PARENT,
            first: dir.alloc.first,
            len: if dir_entry == ROOT_ENTRY { u64::MAX } else { old_len },
            valid: old_len,
            contiguous: dir.alloc.contiguous,
            dir: true,
            hint: Hint::NONE,
            dirty: false,
            opens: 0,
            unlinked: false,
        };
        let owner = if old.first == 0 { Owner::Entry(dir_entry) } else { Owner::None };
        let added = self.allocate_chain(count, true, owner).await?;
        let linked = self.link(&old, added).await;
        let alloc = match linked {
            Ok(alloc) => alloc,
            Err(err) => {
                let _ = self.recover().await;
                return Err(err);
            }
        };
        if old.first != 0 {
            self.pending = None;
        }
        if dir_entry == ROOT_ENTRY {
            return Ok(DirStart { alloc, size: u64::MAX });
        }
        let len = old_len.div_ceil(cluster_size) * cluster_size + count as u64 * cluster_size;
        let mut set = Set::new();
        self.set_at(dir_entry, &mut set).await?;
        set_stream(&mut set.raw[1], alloc, len, len);
        codec::seal(&mut set.raw[..set.count]);
        self.write_set(&set, 2, SetWrite::Update).await?;
        self.pending = None;
        if let Some(id) = self.pinned_at(dir_entry)
            && let Some(node) = self.nodes.get_mut(id)
        {
            node.first = alloc.first;
            node.contiguous = alloc.contiguous;
            node.len = len;
            node.valid = len;
        }
        Ok(DirStart { alloc, size: len })
    }

    /// Links the chain at `added` after the allocation of `node`, giving a
    /// contiguous allocation a FAT chain first. Returns the new allocation.
    async fn link(&mut self, node: &Node, added: u32) -> FsResult<Alloc, D::Error> {
        if node.first == 0 {
            return Ok(Alloc { first: added, contiguous: false });
        }
        let tail = if node.contiguous {
            let clusters = node.len.div_ceil(self.geo.cluster_size()) as u32;
            for step in 0..clusters.saturating_sub(1) {
                self.set_fat(node.first + step, node.first + step + 1).await?;
            }
            let tail = node.first + clusters.max(1) - 1;
            self.set_fat(tail, raw::FAT_END).await?;
            tail
        } else {
            let mut cluster = self.check_cluster(node.first)?;
            let mut steps = 0;
            while let Some(next) = self.next_cluster(cluster).await? {
                cluster = next;
                steps += 1;
                if steps > self.geo.cluster_count {
                    return Err(ErrorKind::Corrupt.into());
                }
            }
            cluster
        };
        if let Some(pending) = self.pending.as_mut()
            && pending.owner == Owner::None
        {
            pending.owner = Owner::Cluster(tail);
        }
        self.set_fat(tail, added).await?;
        Ok(Alloc { first: node.first, contiguous: false })
    }

    /// Writes a new node's entry set: a directory's cluster first, then
    /// the set.
    #[allow(clippy::too_many_arguments)]
    async fn create_set(
        &mut self,
        dir: DirStart,
        dir_entry: u64,
        name: &Units,
        hash: u16,
        plan: &Plan,
        is_dir: bool,
        meta: &SetMetadata,
    ) -> FsResult<Node, D::Error> {
        let now = self.now();
        let times = meta.times();
        let mut primary = [0u8; ENTRY_SIZE];
        let attributes = if is_dir { raw::ATTR_DIRECTORY } else { raw::ATTR_ARCHIVE };
        primary[4..6].copy_from_slice(&attributes.to_le_bytes());
        if let Some(attributes) = meta.attributes() {
            set_attributes(&mut primary, attributes);
        }
        stamp(&mut primary, CREATED, times.created().unwrap_or(now));
        stamp(&mut primary, MODIFIED, times.modified().unwrap_or(now));
        stamp(&mut primary, ACCESSED, times.accessed().unwrap_or(now));
        let mut stream = [0u8; ENTRY_SIZE];
        let mut alloc = Alloc { first: 0, contiguous: false };
        let mut len = 0;
        let (dir, plan) = self.grow_first(dir, dir_entry, plan).await?;
        if is_dir {
            alloc.first = self.allocate_chain(1, true, Owner::None).await?;
            len = self.geo.cluster_size();
        }
        set_stream(&mut stream, alloc, len, len);
        let mut set = [[0; ENTRY_SIZE]; MAX_SET];
        let inserted = match build_set(&primary, &stream, name, hash, &[], &mut set) {
            Ok(count) => self.insert(dir, dir_entry, &plan, &set[..count]).await,
            Err(kind) => Err(kind.into()),
        };
        match inserted {
            Ok(entry) => {
                self.pending = None;
                Ok(Node {
                entry,
                parent: dir_entry,
                first: alloc.first,
                len,
                valid: len,
                contiguous: false,
                dir: is_dir,
                hint: Hint::NONE,
                dirty: false,
                opens: 0,
                unlinked: false,
                })
            }
            Err(err) => {
                let _ = self.recover().await;
                Err(err)
            }
        }
    }

    /// The primary and stream entries a moved node gets: its pending
    /// sizes, and the archive attribute for a file.
    fn moved_entries(&self, set: &Set, node: &Node) -> (RawEntry, RawEntry) {
        let mut primary = set.raw[0];
        let mut stream = set.raw[1];
        set_stream(&mut stream, node.alloc(), node.len, node.valid);
        if !node.dir {
            let attributes = le16(&primary, 4) | raw::ATTR_ARCHIVE;
            primary[4..6].copy_from_slice(&attributes.to_le_bytes());
        }
        if node.dirty {
            let now = self.now();
            stamp(&mut primary, MODIFIED, now);
            stamp(&mut primary, ACCESSED, now);
        }
        (primary, stream)
    }

    /// Renames in place to a name that differs only in case.
    async fn rewrite_name(&mut self, set: &Set, node: &Node, id: Option<NodeId>, name: &Units, hash: u16) -> FsResult<(), D::Error> {
        let (primary, stream) = self.moved_entries(set, node);
        let mut new = *set;
        let count = build_set(&primary, &stream, name, hash, set.extras(), &mut new.raw)?;
        if count != set.count {
            return Err(ErrorKind::Corrupt.into());
        }
        self.write_set(&new, count, SetWrite::Update).await?;
        if let Some(id) = id {
            self.clean(id);
        }
        Ok(())
    }

    /// Writes `set` under `name` where `plan` found room in `to`, then
    /// removes the old set.
    #[allow(clippy::too_many_arguments)]
    async fn move_set(
        &mut self,
        set: &Set,
        node: &Node,
        id: Option<NodeId>,
        to: DirStart,
        to_entry: u64,
        plan: &Plan,
        name: &Units,
        hash: u16,
    ) -> FsResult<(), D::Error> {
        let (primary, stream) = self.moved_entries(set, node);
        let mut raw = [[0; ENTRY_SIZE]; MAX_SET];
        let count = build_set(&primary, &stream, name, hash, set.extras(), &mut raw)?;
        let offset = self.insert(to, to_entry, plan, &raw[..count]).await?;
        let old = set.offset;
        self.repoint(id, old, offset, to_entry);
        if let Err(err) = self.clear_set(set).await {
            self.repoint(id, offset, old, node.parent);
            let _ = self.write_set(set, set.count, SetWrite::Insert).await;
            let _ = self.clear_new(to, plan, count).await;
            return Err(err);
        }
        if let Some(id) = id {
            self.clean(id);
        }
        Ok(())
    }

    /// Moves the pinned node `id` and the parent of its pinned children
    /// from the set at `from` to the set at `to`, in the directory whose
    /// set is at `parent`.
    fn repoint(&mut self, id: Option<NodeId>, from: u64, to: u64, parent: u64) {
        self.nodes.for_each_mut(&mut |_, child| {
            if child.parent == from {
                child.parent = to;
            }
        });
        if let Some(id) = id
            && let Some(state) = self.nodes.get_mut(id)
        {
            state.entry = to;
            state.parent = parent;
            self.moved = true;
        }
    }

    /// Clears the entries a failed move wrote.
    async fn clear_new(&mut self, dir: DirStart, plan: &Plan, count: usize) -> FsResult<(), D::Error> {
        let mut walk = Walk::new(dir);
        for slot in plan.start..plan.start + count as u32 {
            if let Some(at) = self.slot_offset(&mut walk, slot).await? {
                let mut entry = [0u8; 1];
                self.read(at, &mut entry).await?;
                self.write(at, &[entry[0] & !raw::IN_USE]).await?;
            }
        }
        Ok(())
    }

    /// Renames onto an existing entry set: the target is removed and the
    /// source written under `name`, in the target's entries when it fits
    /// them, then the target's clusters are freed.
    #[allow(clippy::too_many_arguments)]
    async fn replace(
        &mut self,
        src: &Located,
        src_node: &Node,
        src_id: Option<NodeId>,
        to: DirStart,
        to_entry: u64,
        target: &Located,
        name: &Units,
        hash: u16,
    ) -> FsResult<(), D::Error> {
        let target_id = self.pinned_at(target.set.offset);
        if target_id.is_some_and(|id| self.is_open(id)) {
            return Err(ErrorKind::Busy.into());
        }
        match (src_node.dir, target.set.is_dir()) {
            (true, false) => return Err(ErrorKind::NotADirectory.into()),
            (false, true) => return Err(ErrorKind::IsADirectory.into()),
            _ => {}
        }
        let target_node = match target_id.and_then(|id| self.nodes.get(id)) {
            Some(node) => *node,
            None => Node::from_set(&target.set, UNKNOWN_PARENT),
        };
        if target_node.dir && !self.dir_is_empty(target_node.alloc(), target_node.len).await? {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        let needed = 2 + name.entries() as u32 + src.set.extras().len() as u32;
        let plan = self.plan(to, None, needed, Some(&target.set)).await?;
        let (to, plan) = self.grow_first(to, to_entry, &plan).await?;
        let held = (target_node.first != 0 && !target_node.contiguous)
            .then(|| Pending::chain(target_node.first, Owner::Removed(target.set.offset)));
        self.pending = held;
        let moved = match self.clear_set(&target.set).await {
            Ok(()) => self.move_set(&src.set, src_node, src_id, to, to_entry, &plan, name, hash).await,
            Err(err) => Err(err),
        };
        if let Err(err) = moved {
            let _ = self.write_set(&target.set, target.set.count, SetWrite::Insert).await;
            self.pending = held;
            let _ = self.recover().await;
            return Err(err);
        }
        self.pending = held;
        if let Some(id) = target_id {
            self.unlink(id);
        }
        let _ = target.slot;
        self.free_alloc(target_node.alloc(), target_node.len).await?;
        self.free_extras(&target.set).await
    }

    /// Stores `value` as the FAT entry of `cluster`.
    pub(super) async fn set_fat(&mut self, cluster: u32, value: u32) -> FsResult<(), D::Error> {
        self.check_cluster(cluster)?;
        if let Some(mirror) = self.geo.mirror_fat {
            self.write(mirror + cluster as u64 * 4, &value.to_le_bytes()).await?;
        }
        self.write(self.geo.fat_start + cluster as u64 * 4, &value.to_le_bytes()).await
    }

    /// The stored FAT entry of `cluster`.
    pub(super) async fn fat_entry(&mut self, cluster: u32) -> FsResult<u32, D::Error> {
        let mut bytes = [0u8; 4];
        self.read(self.geo.fat_start + cluster as u64 * 4, &mut bytes).await?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub(super) async fn next_cluster(&mut self, cluster: u32) -> FsResult<Option<u32>, D::Error> {
        match self.fat_entry(cluster).await? {
            raw::FAT_END => Ok(None),
            next if self.geo.is_cluster(next) => Ok(Some(next)),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    /// The position of cluster `want` of an allocation, walked from `hint`
    /// when it is not past `want`. A chain that ends first or loops fails
    /// with [`ErrorKind::Corrupt`].
    async fn locate_cluster(&mut self, alloc: Alloc, hint: Hint, want: u32) -> FsResult<Hint, D::Error> {
        if alloc.contiguous {
            let cluster = alloc.first.checked_add(want).ok_or(ErrorKind::Corrupt)?;
            let mut at = Hint::start(alloc.first);
            at.index = want;
            at.cluster = self.check_cluster(cluster)?;
            return Ok(at);
        }
        let mut at = if hint.cluster != 0 && hint.index <= want {
            hint
        } else {
            Hint::start(self.check_cluster(alloc.first)?)
        };
        while at.index < want {
            let next = self.next_cluster(at.cluster).await?.ok_or(ErrorKind::Corrupt)?;
            if !at.advance(next) {
                return Err(ErrorKind::Corrupt.into());
            }
        }
        Ok(at)
    }

    /// Byte offset of byte `pos` of the bitmap.
    async fn bitmap_offset(&mut self, pos: u64) -> FsResult<u64, D::Error> {
        let extent = self.bitmap;
        let alloc = Alloc { first: extent.first, contiguous: extent.contiguous };
        let hint = self.locate_cluster(alloc, self.bitmap_hint, (pos >> self.geo.cluster_shift) as u32).await?;
        self.bitmap_hint = hint;
        Ok(self.cluster_at(hint.cluster)? + (pos & (self.geo.cluster_size() - 1)))
    }

    /// Reads bitmap bytes from `pos`, at most to the end of a cluster.
    pub(super) async fn bitmap_bytes(&mut self, pos: u64, out: &mut [u8]) -> FsResult<usize, D::Error> {
        let within = pos & (self.geo.cluster_size() - 1);
        let n = (out.len() as u64).min(self.geo.cluster_size() - within) as usize;
        let at = self.bitmap_offset(pos).await?;
        self.read(at, &mut out[..n]).await?;
        Ok(n)
    }

    async fn set_bit(&mut self, cluster: u32, used: bool) -> FsResult<(), D::Error> {
        let index = self.check_cluster(cluster)? - raw::FIRST_CLUSTER;
        let at = self.bitmap_offset(index as u64 / 8).await?;
        let mut byte = [0u8; 1];
        self.read(at, &mut byte).await?;
        let bit = 1 << (index % 8);
        let was = byte[0] & bit != 0;
        if was == used {
            return Ok(());
        }
        byte[0] ^= bit;
        if let Some(mirror) = self.mirror_bitmap {
            let alloc = Alloc { first: mirror.first, contiguous: mirror.contiguous };
            let cluster = self.locate_cluster(alloc, Hint::NONE, index >> (self.geo.cluster_shift + 3)).await?.cluster;
            let within = (index as u64 / 8) & (self.geo.cluster_size() - 1);
            let mut other = [0u8; 1];
            let other_at = self.cluster_at(cluster)? + within;
            self.read(other_at, &mut other).await?;
            other[0] = (other[0] & !bit) | (byte[0] & bit);
            self.write(other_at, &other).await?;
        }
        self.write(at, &byte).await?;
        self.allocation_changed = true;
        self.free_clusters = match (self.free_clusters, used) {
            (Some(free), true) => free.checked_sub(1),
            (Some(free), false) => Some((free + 1).min(self.geo.cluster_count)),
            (None, _) => None,
        };
        Ok(())
    }

    async fn free_count(&mut self) -> FsResult<u32, D::Error> {
        if let Some(free) = self.free_clusters {
            return Ok(free);
        }
        let count = self.geo.cluster_count;
        let mut used = 0u32;
        let mut pos = 0u64;
        let mut chunk = [0u8; 64];
        let bytes = (count as u64).div_ceil(8);
        while pos < bytes {
            let want = ((bytes - pos) as usize).min(chunk.len());
            let n = self.bitmap_bytes(pos, &mut chunk[..want]).await?;
            for (index, &byte) in chunk[..n].iter().enumerate() {
                let first = (pos + index as u64) * 8;
                let mask = if first + 8 > count as u64 { (1u16 << (count as u64 - first)) as u8 - 1 } else { 0xFF };
                used += (byte & mask).count_ones();
            }
            pos += n as u64;
        }
        let free = count - used.min(count);
        self.free_clusters = Some(free);
        Ok(free)
    }

    /// Takes a free cluster: its FAT entry ends a chain and its bitmap bit
    /// is set.
    async fn allocate(&mut self) -> FsResult<u32, D::Error> {
        let count = self.geo.cluster_count;
        let from = self.next_free.clamp(raw::FIRST_CLUSTER, self.geo.max_cluster()) - raw::FIRST_CLUSTER;
        let mut scanned = 0u32;
        let mut chunk = [0u8; 64];
        let mut index = from & !7;
        while scanned < count + 8 {
            let pos = (index / 8) as u64;
            let want = ((count as u64).div_ceil(8) - pos).min(chunk.len() as u64) as usize;
            let n = self.bitmap_bytes(pos, &mut chunk[..want]).await?;
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
                    if let Some(pending) = self.pending.as_mut() {
                        if pending.head == 0 {
                            pending.head = cluster;
                        } else {
                            pending.extra = cluster;
                        }
                    }
                    self.set_fat(cluster, raw::FAT_END).await?;
                    self.set_bit(cluster, true).await?;
                    self.next_free = if cluster == self.geo.max_cluster() { raw::FIRST_CLUSTER } else { cluster + 1 };
                    return Ok(cluster);
                }
            }
            scanned += n as u32 * 8;
            index += n as u32 * 8;
            if index >= count {
                index = 0;
            }
        }
        self.free_clusters = Some(0);
        Err(ErrorKind::NoSpace.into())
    }

    /// Allocates a chain of `count` clusters, zeroed when `zero` is set, and
    /// returns its first cluster. The chain stays pending, to be linked
    /// through `owner`; the caller clears `pending` once it is. Nothing is
    /// left allocated on failure.
    async fn allocate_chain(&mut self, count: u32, zero: bool, owner: Owner) -> FsResult<u32, D::Error> {
        self.pending = Some(Pending::chain(0, owner));
        if !zero && count > 1 {
            let allocated = self.allocate_run(count).await;
            if allocated.is_err() {
                let _ = self.recover().await;
            }
            return allocated;
        }
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
                    let _ = self.recover().await;
                    return Err(err);
                }
            }
        }
        Ok(first)
    }

    async fn append_cluster(&mut self, prev: u32, zero: bool) -> FsResult<u32, D::Error> {
        let cluster = self.allocate().await?;
        if zero {
            let at = self.cluster_at(cluster)?;
            self.put(at, None, self.geo.cluster_size()).await?;
        }
        if prev != 0 {
            self.set_fat(prev, cluster).await?;
            if let Some(pending) = self.pending.as_mut() {
                pending.extra = 0;
            }
        }
        Ok(cluster)
    }

    /// Frees the chain at `first`, which nothing links any more: every
    /// bitmap bit, following the FAT. Clears `pending`; on failure what is
    /// left of the chain stays pending. The bits of consecutive clusters of
    /// the chain that share a device block of the bitmap are cleared with
    /// one write.
    async fn free_chain(&mut self, first: u32) -> FsResult<(), D::Error> {
        let cluster = self.check_cluster(first)?;
        self.mark_chain(cluster, self.geo.cluster_count, false).await?;
        self.pending = None;
        Ok(())
    }

    /// Sets or clears the bitmap bits of the chain at `cluster`, at most
    /// `limit` clusters of it, and fails with [`ErrorKind::Corrupt`] when
    /// it is longer. Clusters whose bits share a device block of each
    /// bitmap take one write. While clearing, what is left of the chain is
    /// kept pending.
    async fn mark_chain(&mut self, mut cluster: u32, limit: u32, used: bool) -> FsResult<(), D::Error> {
        let mut steps = 0;
        loop {
            let start = cluster;
            let mut group = ClusterGroup::new(cluster);
            let place = self.bit_place(cluster).await?;
            let next = loop {
                if steps == limit {
                    break Err(ErrorKind::Corrupt.into());
                }
                steps += 1;
                let next = if used && steps == limit { Ok(None) } else { self.next_cluster(cluster).await };
                if next.is_ok() {
                    group.add(cluster);
                }
                match next {
                    Ok(Some(next)) if group.spans(next) && !group.has(next) && self.bit_place(next).await? == place => {
                        cluster = next;
                    }
                    other => break other,
                }
            };
            if group.count > 0 {
                if !used {
                    self.pending = Some(Pending {
                        head: next.as_ref().ok().copied().flatten().unwrap_or(0),
                        extra: start,
                        owner: Owner::None,
                    });
                }
                self.patch_bits(&group, place, used).await?;
            }
            match next? {
                Some(next) => cluster = next,
                None => return Ok(()),
            }
        }
    }

    /// Allocates a chain of `count` unzeroed clusters, the first `count`
    /// free ones from `next_free` on, and returns its first cluster. The
    /// FAT entries are written a device block at a time from the chain's
    /// end back to its start, then the bitmap bits a block at a time. The
    /// chain is recorded in the pending allocation before any bit is set.
    async fn allocate_run(&mut self, count: u32) -> FsResult<u32, D::Error> {
        let total = self.geo.cluster_count;
        let from = self.next_free.clamp(raw::FIRST_CLUSTER, self.geo.max_cluster()) - raw::FIRST_CLUSTER;
        let at = |step: u32| raw::FIRST_CLUSTER + (from + step) % total;
        let mut found = 0;
        let mut last = None;
        for step in 0..total {
            if !self.bit(at(step)).await? {
                found += 1;
                if found == count {
                    last = Some(step);
                    break;
                }
            }
        }
        let Some(last) = last else {
            self.free_clusters = Some(found);
            return Err(ErrorKind::NoSpace.into());
        };
        let mut head = raw::FAT_END;
        let mut top = last + 1;
        while top > 0 {
            let high = at(top - 1);
            let mut group = ClusterGroup::new(high.saturating_sub(ClusterGroup::SPAN - 1));
            while top > 0 {
                let cluster = at(top - 1);
                if cluster > high || !self.same_fat_block(cluster, high) {
                    break;
                }
                if !self.bit(cluster).await? {
                    group.add(cluster);
                }
                top -= 1;
            }
            if group.count > 0 {
                self.patch_fat(&group, head).await?;
                head = group.lowest();
            }
        }
        if let Some(pending) = self.pending.as_mut() {
            pending.head = head;
        }
        self.mark_chain(head, count, true).await?;
        let tail = at(last);
        self.next_free = if tail == self.geo.max_cluster() { raw::FIRST_CLUSTER } else { tail + 1 };
        Ok(head)
    }

    /// Whether the bitmap marks `cluster` used.
    async fn bit(&mut self, cluster: u32) -> FsResult<bool, D::Error> {
        let index = self.check_cluster(cluster)? - raw::FIRST_CLUSTER;
        let mut byte = [0u8; 1];
        self.bitmap_bytes(index as u64 / 8, &mut byte).await?;
        Ok(byte[0] & (1 << (index % 8)) != 0)
    }

    /// Where the bitmap bits around `cluster` are: for the bitmap and the
    /// mirror bitmap, the device block holding its bit and the offset
    /// bitmap byte 0 would have if the bitmap were contiguous up to it.
    /// Clusters with the same place have their bits in one device block.
    async fn bit_place(&mut self, cluster: u32) -> FsResult<[(u64, u64); 2], D::Error> {
        let size = self.block.size as u64;
        let byte = (self.check_cluster(cluster)? - raw::FIRST_CLUSTER) as u64 / 8;
        let at = self.bitmap_offset(byte).await?;
        let mut place = [(at / size, at - byte), (0, 0)];
        if let Some(mirror) = self.mirror_bitmap {
            let alloc = Alloc { first: mirror.first, contiguous: mirror.contiguous };
            let other = self.locate_cluster(alloc, Hint::NONE, (byte >> self.geo.cluster_shift) as u32).await?.cluster;
            let at = self.cluster_at(other)? + (byte & (self.geo.cluster_size() - 1));
            place[1] = (at / size, at - byte);
        }
        Ok(place)
    }

    /// Sets or clears the bitmap bits of `group`, which share `place`: the
    /// mirror bitmap first, as `set_bit` does.
    async fn patch_bits(&mut self, group: &ClusterGroup, place: [(u64, u64); 2], used: bool) -> FsResult<(), D::Error> {
        self.begin_write().await?;
        let size = self.block.size as u64;
        let mut flips = 0;
        let order = if self.mirror_bitmap.is_some() { &[1, 0][..] } else { &[0][..] };
        for &which in order {
            let (block, origin) = place[which];
            load(&mut self.dev, &mut self.block, block).await?;
            for cluster in group.descending() {
                let bit = cluster - raw::FIRST_CLUSTER;
                let byte = (origin + bit as u64 / 8 - block * size) as usize;
                let mask = 1 << (bit % 8);
                if which == 0 && (self.block.data[byte] & mask != 0) != used {
                    flips += 1;
                }
                if used {
                    self.block.data[byte] |= mask;
                } else {
                    self.block.data[byte] &= !mask;
                }
            }
            self.write_block(block).await?;
        }
        self.allocation_changed = true;
        self.free_clusters = match (self.free_clusters, used) {
            (Some(free), true) => free.checked_sub(flips),
            (Some(free), false) => Some((free + flips).min(self.geo.cluster_count)),
            (None, _) => None,
        };
        Ok(())
    }

    /// Whether the FAT entries of `a` and `b` lie in one device block of
    /// every FAT written.
    fn same_fat_block(&self, a: u32, b: u32) -> bool {
        let size = self.block.size as u64;
        [Some(self.geo.fat_start), self.geo.mirror_fat]
            .into_iter()
            .flatten()
            .all(|base| (base + a as u64 * 4) / size == (base + b as u64 * 4) / size)
    }

    /// Writes the FAT entries of `group`, which share a device block of
    /// every FAT, linking them in ascending order with the highest pointing
    /// to `next`: the mirror FAT first, as `set_fat` does.
    async fn patch_fat(&mut self, group: &ClusterGroup, next: u32) -> FsResult<(), D::Error> {
        self.begin_write().await?;
        let size = self.block.size as u64;
        for base in [self.geo.mirror_fat, Some(self.geo.fat_start)].into_iter().flatten() {
            let block = (base + group.lowest() as u64 * 4) / size;
            load(&mut self.dev, &mut self.block, block).await?;
            let mut value = next;
            for cluster in group.descending() {
                let at = (base + cluster as u64 * 4 - block * size) as usize;
                self.block.data[at..at + 4].copy_from_slice(&value.to_le_bytes());
                value = cluster;
            }
            self.write_block(block).await?;
        }
        Ok(())
    }

    /// Frees an allocation of `len` bytes.
    async fn free_alloc(&mut self, alloc: Alloc, len: u64) -> FsResult<(), D::Error> {
        if alloc.first == 0 {
            return Ok(());
        }
        if !alloc.contiguous {
            return self.free_chain(alloc.first).await;
        }
        let clusters = len.div_ceil(self.geo.cluster_size()) as u32;
        for step in 0..clusters {
            self.set_bit(alloc.first + step, false).await?;
        }
        Ok(())
    }

    /// Frees the allocations of the benign secondary entries of `set`,
    /// which is no longer on disk.
    async fn free_extras(&mut self, set: &Set) -> FsResult<(), D::Error> {
        for (alloc, len) in set.extra_allocs() {
            self.free_alloc(alloc, len).await?;
        }
        Ok(())
    }

        /// Extends a file's allocation to hold `end` bytes. The new clusters
    /// are linked but not zeroed, and stay pending until the caller has
    /// recorded the new size.
    async fn cover(&mut self, state: &Node, end: u64) -> FsResult<Growth, D::Error> {
        let cluster_size = self.geo.cluster_size();
        let have = state.len.div_ceil(cluster_size);
        let need = end.div_ceil(cluster_size);
        let none = Growth { alloc: state.alloc(), added: 0 };
        if need <= have && (state.first != 0 || need == 0) {
            return Ok(none);
        }
        let extra = u32::try_from(need - have).map_err(|_| ErrorKind::NoSpace)?;
        if extra > self.free_count().await? {
            return Err(ErrorKind::NoSpace.into());
        }
        let owner = if state.first == 0 { Owner::Entry(state.entry) } else { Owner::None };
        let added = self.allocate_chain(extra, false, owner).await?;
        if state.first == 0 {
            return Ok(Growth { alloc: Alloc { first: added, contiguous: false }, added });
        }
        match self.link(state, added).await {
            Ok(alloc) => {
                if let Some(pending) = self.pending.as_mut()
                    && let Owner::Cluster(tail) = pending.owner
                {
                    pending.owner = Owner::Tail(tail);
                }
                Ok(Growth { alloc, added })
            }
            Err(err) => {
                let _ = self.recover().await;
                Err(err)
            }
        }
    }

    /// Frees what [`cover`](Self::cover) added, which is still pending.
    /// Best effort.
    async fn undo_growth(&mut self, growth: Growth) {
        if growth.added != 0 {
            let _ = self.recover().await;
        }
    }

    /// Writes `len` bytes of `data`, or zeros, at byte `pos` of an
    /// allocation, and returns a hint for the last cluster written.
    async fn fill(&mut self, alloc: Alloc, hint: Hint, pos: u64, data: Option<&[u8]>, len: u64) -> FsResult<Hint, D::Error> {
        if len == 0 {
            return Ok(hint);
        }
        let cluster_size = self.geo.cluster_size();
        let mut hint = hint;
        let mut done = 0u64;
        while done < len {
            let at = pos + done;
            hint = self.locate_cluster(alloc, hint, (at / cluster_size) as u32).await?;
            let within = at % cluster_size;
            let n = (cluster_size - within).min(len - done);
            let offset = self.cluster_at(hint.cluster)? + within;
            let max = usize::try_from(len - done).unwrap_or(usize::MAX);
            let n = self.run(alloc, &mut hint, n as usize, max).await? as u64;
            let chunk = data.map(|data| &data[done as usize..(done + n) as usize]);
            self.put(offset, chunk, n).await?;
            done += n;
        }
        Ok(hint)
    }

    /// Extends `n` bytes that start in the cluster at `at` of an
    /// allocation over the clusters that follow it on disk and in the
    /// allocation, up to `max` bytes, so they take one device call. Returns
    /// the length and leaves `at` at the run's last cluster.
    async fn run(&mut self, alloc: Alloc, at: &mut Hint, mut n: usize, max: usize) -> FsResult<usize, D::Error> {
        let cluster_size = self.geo.cluster_size() as usize;
        while n < max {
            let next = if alloc.contiguous {
                Some(at.cluster + 1).filter(|&next| self.geo.is_cluster(next))
            } else {
                self.next_cluster(at.cluster).await?
            };
            match next {
                Some(next) if next == at.cluster + 1 => {
                    if alloc.contiguous {
                        at.cluster = next;
                        at.index += 1;
                    } else if !at.advance(next) {
                        return Err(ErrorKind::Corrupt.into());
                    }
                    n = n.saturating_add(cluster_size).min(max);
                }
                _ => break,
            }
        }
        Ok(n)
    }
}

}

/// What a tree search looks for.
#[derive(Clone, Copy)]
enum Target {
    /// The directory holding the entry set at this offset.
    ParentOf(u64),
    /// A directory with this first cluster, below the given directory.
    Below(DirStart, u32),
}

impl_exfat_driver!(impl[D: BlockDevice, T: NodeTable, C: Clock] ExFatFs<D, T, C>, error = D::Error; also = [parent, open_node, close_node, publish_node]);
