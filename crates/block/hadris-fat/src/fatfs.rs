use core::fmt;

use hadris_common::types::endian::Endian;
use hadris_fs::{
    Attributes, Capabilities, CaseSensitivity, DirCursor, DirEntry, Error, ErrorKind, FileTimes,
    FileType, FixedTable, FsResult, FsStats, Metadata, Name, NameBuf, NameCharset, NameError,
    NodeId, NodeTable,
};
use hadris_storage::BlockIndex;

use super::storage::BlockDevice;
use crate::FatKind;
use crate::codec::boot::{self, BootError, Geometry, RootDir};
use crate::codec::dirent::{self, ENTRY_SIZE, ShortEntry, Slot};
use crate::codec::entry::FIRST_DATA_CLUSTER;
use crate::codec::lfn::{self, Assembler};
use crate::codec::{date, name as names, short_name};
use crate::raw::{RawBpb, RawBpbExt16, RawBpbExt32, RawFsInfo};

const ROOT: NodeId = NodeId::new(1);
/// Ids from here up are handed out when a node's natural id is taken.
const FALLBACK_IDS: u64 = 1 << 63;
/// The largest device block [`FatFs`] can buffer.
const MAX_BLOCK_SIZE: usize = 4096;
const BOOT_SECTOR_LEN: usize = 512;
const BPB_LEN: usize = size_of::<RawBpb>();
const FAT32_MIRRORING_DISABLED: u16 = 0x80;
const FAT32_ACTIVE_FAT: u16 = 0x0F;

/// Decodes a short-name byte above `0x7F`. The code page becomes a type
/// parameter of `FatFs` later; until then such bytes read as U+FFFD, as with
/// the default `LossyAsciiOemCpConverter` of `FatVolume`.
fn oem_char(_: u8) -> char {
    char::REPLACEMENT_CHARACTER
}

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
}

impl Node {
    fn new(entry: u64, short: &ShortEntry, kind: FatKind) -> Self {
        Self {
            entry,
            first: short.first_cluster(kind),
            size: short.size,
            dir: short.is_dir(),
            hint: (0, 0),
        }
    }
}

/// Where a directory's entries live.
#[derive(Debug, Clone, Copy)]
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
}

/// One device block, the driver's only buffer.
struct BlockBuf {
    data: [u8; MAX_BLOCK_SIZE],
    size: usize,
    cached: Option<u64>,
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
    for (bit, flag) in [
        (dirent::ATTR_READ_ONLY, Attributes::READ_ONLY),
        (dirent::ATTR_HIDDEN, Attributes::HIDDEN),
        (dirent::ATTR_SYSTEM, Attributes::SYSTEM),
        (dirent::ATTR_ARCHIVE, Attributes::ARCHIVE),
    ] {
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
fn matches(query: &str, long: Option<&[u16]>, entry: &ShortEntry) -> bool {
    if long.is_some_and(|units| names::eq_ignore_case(query.chars(), names::utf16_chars(units))) {
        return true;
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(&entry.name, entry.nt_case, oem_char, &mut short);
    core::str::from_utf8(&short[..len])
        .is_ok_and(|short| names::eq_ignore_case(query.chars(), short.chars()))
}

/// Writes the entry's name into `out`: the long name when it is a valid
/// name, else the short name.
fn write_name(
    out: &mut NameBuf,
    long: Option<&[u16]>,
    entry: &ShortEntry,
) -> Result<usize, ErrorKind> {
    if let Some(units) = long
        && out
            .fill(|buf| names::utf16_to_utf8(units, buf).ok_or(NameError::TooLong))
            .is_ok()
    {
        return Ok(out.len());
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(&entry.name, entry.nt_case, oem_char, &mut short);
    out.set_bytes(&short[..len]).map_err(|err| match err {
        NameError::TooLong => ErrorKind::LimitExceeded,
        _ => ErrorKind::Corrupt,
    })?;
    Ok(len)
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

/// A FAT12, FAT16 or FAT32 volume on a block device.
///
/// `FatFs` is the V3 driver: every node method takes `&mut self`, holds no
/// lock, and needs no allocator. Share it through `hadris_fs` `Volume`, or
/// call the node methods directly; they have the same names and signatures
/// as the `FsDriver` methods, which forward to them.
///
/// Nodes are identified by the location of their directory entry. `lookup`
/// and `parent` pin the node they return in the node table `T`, and
/// `forget` unpins it. A full table makes `lookup` fail with
/// [`ErrorKind::LimitExceeded`]; name `HeapTable` or a larger
/// `FixedTable<N>` for more open nodes. Ids from `read_dir_entry` are not
/// pinned and stay valid until that directory changes.
///
/// Long names are always read. Names compare case-insensitively, by the
/// long name or by the short name, and short-name bytes above `0x7F` read
/// as U+FFFD.
///
/// The driver keeps one device block, at most 4096 bytes, inline as its
/// buffer, so a `FatFs` is a little over 4 KiB plus the node table. Reads of
/// whole blocks go straight into the caller's buffer. Devices with blocks
/// larger than 4096 bytes are rejected with [`ErrorKind::Unsupported`]; the
/// device block may be smaller or larger than the FAT sector.
///
/// This release reads only: the write methods of `FsDriver` answer
/// [`ErrorKind::ReadOnly`] and [`capabilities`](Self::capabilities) reports
/// the volume as not writable.
pub struct FatFs<D, T: NodeTable = FixedTable<64>> {
    dev: D,
    geo: Geometry,
    active_fat: u8,
    data_end: u64,
    nodes: T::With<Node>,
    next_id: u64,
    block: BlockBuf,
    free_clusters: Option<u32>,
    read_only: bool,
}

impl<D, T: NodeTable> fmt::Debug for FatFs<D, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FatFs")
            .field("kind", &self.geo.kind)
            .field("cluster_size", &self.geo.cluster_size)
            .field("open_nodes", &(self.nodes.len() + 1))
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl<D: BlockDevice> FatFs<D> {
    /// Mounts the volume on `dev` with the default node table.
    ///
    /// Fails with [`ErrorKind::Corrupt`] when the boot sector is not a valid
    /// FAT12, FAT16 or FAT32 boot sector or describes a volume larger than
    /// the device, and with [`ErrorKind::Unsupported`] when the device's
    /// blocks are larger than 4096 bytes.
    pub async fn open(dev: D) -> FsResult<Self, D::Error> {
        Self::open_with_table(dev, FixedTable::new()).await
    }

    /// Mounts the volume on `dev` for reading only, with the default node
    /// table. The driver never calls `write_blocks`.
    pub async fn open_read_only(dev: D) -> FsResult<Self, D::Error> {
        Self::open_read_only_with_table(dev, FixedTable::new()).await
    }
}

impl<D: BlockDevice, T: NodeTable> FatFs<D, T> {
    /// Mounts the volume on `dev` with a node table of the kind of `table`.
    pub async fn open_with_table(dev: D, table: T) -> FsResult<Self, D::Error> {
        Self::mount(dev, &table, false).await
    }

    /// Mounts the volume on `dev` for reading only, with a node table of the
    /// kind of `table`.
    pub async fn open_read_only_with_table(dev: D, table: T) -> FsResult<Self, D::Error> {
        Self::mount(dev, &table, true).await
    }

    async fn mount(mut dev: D, table: &T, read_only: bool) -> FsResult<Self, D::Error> {
        let size = dev.block_size().get() as usize;
        if size > MAX_BLOCK_SIZE {
            return Err(ErrorKind::Unsupported.into());
        }
        let mut block = BlockBuf { data: [0; MAX_BLOCK_SIZE], size, cached: None };
        let mut sector = [0u8; BOOT_SECTOR_LEN];
        read_bytes(&mut dev, &mut block, 0, &mut sector).await?;
        let bpb: RawBpb = bytemuck::pod_read_unaligned(&sector[..BPB_LEN]);
        boot::check_bpb(&bpb).map_err(corrupt)?;
        let (geo, active_fat, fs_info) = if boot::is_fat32(&bpb) {
            let ext: RawBpbExt32 =
                bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt32>()]);
            boot::check_ext32(&bpb, &ext).map_err(corrupt)?;
            let geo = boot::geometry32(&bpb, &ext).map_err(corrupt)?;
            let flags = u16::from_le_bytes(ext.ext_flags);
            let active = if flags & FAT32_MIRRORING_DISABLED != 0 {
                (flags & FAT32_ACTIVE_FAT) as u8
            } else {
                0
            };
            (geo, active, Some(ext.fs_info_sector.get()))
        } else {
            let ext: RawBpbExt16 =
                bytemuck::pod_read_unaligned(&sector[BPB_LEN..BPB_LEN + size_of::<RawBpbExt16>()]);
            boot::check_ext16(&bpb, &ext).map_err(corrupt)?;
            (boot::geometry16(&bpb).map_err(corrupt)?, 0, None)
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
        let free_clusters = match fs_info {
            Some(at) if at != 0 && at < bpb.reserved_sector_count.get() => {
                let mut sector = [0u8; BOOT_SECTOR_LEN];
                let offset = at as u64 * geo.sector_size as u64;
                read_bytes(&mut dev, &mut block, offset, &mut sector).await?;
                let info: RawFsInfo = bytemuck::pod_read_unaligned(&sector);
                let free = info.free_count.get();
                (boot::check_fs_info(&info).is_ok() && free < geo.max_cluster).then_some(free)
            }
            _ => None,
        };
        Ok(Self {
            dev,
            geo,
            active_fat,
            data_end,
            nodes: table.empty(),
            next_id: FALLBACK_IDS,
            block,
            free_clusters,
            read_only,
        })
    }

    /// Returns the device.
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// Whether the volume was mounted with
    /// [`open_read_only`](FatFs::open_read_only).
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The FAT variant of the volume.
    pub fn kind(&self) -> FatKind {
        self.geo.kind
    }

    /// Number of nodes in the node table, plus one for the root, which is
    /// always pinned.
    pub fn open_nodes(&self) -> usize {
        self.nodes.len() + 1
    }

    /// What this volume supports: case-insensitive, case-preserving UTF-16
    /// names of up to 255 code units (765 bytes of UTF-8) and two-second
    /// modification times.
    pub fn capabilities(&self) -> Capabilities {
        Capabilities::new()
            .with_case_sensitivity(CaseSensitivity::InsensitivePreserving)
            .with_name_charset(NameCharset::Utf16)
            .with_max_name_len(lfn::MAX_UNITS * 3)
            .with_timestamp_resolution_ns(2_000_000_000)
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
        let mut walk = Walk::new(start);
        let mut slot = 0;
        let mut long = Assembler::new();
        while let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? {
            let units = long.finish(lfn::checksum(&found.entry.name));
            if matches(query, units.filter(|units| !units.is_empty()), &found.entry) {
                return Ok(self.intern(found.offset, &found.entry)?);
            }
        }
        Err(ErrorKind::NotFound.into())
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
        let len = write_name(name, units.filter(|units| !units.is_empty()), &found.entry)?;
        let file_type = if found.entry.is_dir() { FileType::Dir } else { FileType::File };
        let node = self.pinned_at(found.offset).unwrap_or(NodeId::new(found.offset / ENTRY_SIZE));
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
            let at = self.geo.cluster_offset(cluster) + within;
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
    /// which is then kept.
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

    /// Unpins a node. Unknown ids and the root are ignored.
    pub fn forget(&mut self, node: NodeId) {
        if node != ROOT {
            self.nodes.unpin(node);
        }
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

    fn pinned_at(&self, offset: u64) -> Option<NodeId> {
        self.nodes.find(&mut |_, node| node.entry == offset)
    }

    /// Pins the node whose short entry is at `offset`.
    fn intern(&mut self, offset: u64, entry: &ShortEntry) -> Result<NodeId, ErrorKind> {
        if let Some(id) = self.pinned_at(offset) {
            self.nodes.pin(id);
            return Ok(id);
        }
        let natural = NodeId::new(offset / ENTRY_SIZE);
        let fallback = self.nodes.get(natural).is_some();
        let id = if fallback { NodeId::new(self.next_id) } else { natural };
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
        let raw = id.get();
        if raw == ROOT.get() || raw >= FALLBACK_IDS {
            return Err(ErrorKind::InvalidHandle.into());
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
        if let Some(node) = self.nodes.get(id) {
            return Ok(*node);
        }
        let (offset, entry) = self.unpinned(id).await?;
        match self.pinned_at(offset).and_then(|id| self.nodes.get(id)) {
            Some(node) => Ok(*node),
            None => Ok(Node::new(offset, &entry, self.geo.kind)),
        }
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
                Ok(Some(self.geo.cluster_offset(walk.cluster) + within))
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
        while let Some(offset) = self.slot_offset(walk, *slot).await? {
            let at = *slot;
            match self.read_slot(offset).await? {
                Slot::End => break,
                Slot::Free => long.reset(),
                Slot::Long(part) => {
                    long.push(part.sequence, part.checksum, &part.name1, &part.name2, &part.name3)
                }
                Slot::Short(entry) if entry.is_visible() => {
                    *slot = at + 1;
                    return Ok(Some(Found { slot: at, offset, entry }));
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
        let offset = self.geo.cluster_offset(first) + ENTRY_SIZE;
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

impl_fat_driver!(impl[D: BlockDevice, T: NodeTable] FatFs<D, T>, error = D::Error, read_only; also = [parent]);
