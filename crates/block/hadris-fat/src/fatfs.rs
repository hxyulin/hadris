use core::fmt;

use super::block_io::{BlockBuf, new_block, read_bytes, write_bytes};
use super::fsapi::FileSystem;
use super::rawio;
use super::storage::BlockDevice;
use hadris_fat_raw::io::{ChainPos, DirStart, DirWalk, Fat, Held};
use hadris_fat_raw::lfn::{self, Assembler};
use hadris_fat_raw::{
    self as raw, LongEntry, RootLocation, ShortEntry, Slot, date, name as names, short_name,
};
use hadris_fs::{
    Capabilities, CaseRule, Charset, Clock, CodePage, DateTime, DirCursor, DirEntry, ErrorKind,
    Extent, Field, FileType, FsResult, FsStats, Metadata, MountError, MountOptions, Name, NameBuf,
    NameError, NodeId, OpenMode, RenameMode, SetAttr, Stored,
};

use crate::names::{
    CANDIDATES, NewName, apply_attributes, is_exact, matches, permissions, read_only_bit,
    set_read_only, stamp,
};
use crate::table::Table;
use crate::{FatKind, Geometry, VolumeLabel, push_run};

/// `NodeId::new` for ids that are not 0 by construction.
const fn node_id(raw: u64) -> NodeId {
    match NodeId::new(raw) {
        Some(id) => id,
        None => RESERVED,
    }
}

const ROOT: NodeId = node_id(1);
/// A node id holds the slot of its short entry, the entry's byte offset
/// divided by 32, in its low `SLOT_BITS` bits. The bits above, the tier,
/// count up when that id is taken by a pinned node that has since moved
/// away from the slot, so a listing and a lookup of one entry agree on its
/// id. A FAT volume has at most `2^32` sectors of 4096 bytes, so slots stay
/// below `2^39`, and no id is 0.
const SLOT_BITS: u32 = 40;
const SLOT_MASK: u64 = (1 << SLOT_BITS) - 1;
/// Highest value of the bits above the slot, so ids stay below `2^63`.
const MAX_TIER: u64 = (1 << (63 - SLOT_BITS)) - 1;
/// The id of the table slot `create` reserves before it writes anything.
/// Never handed out.
const RESERVED: NodeId = match NodeId::new(1 << 63) {
    Some(id) => id,
    None => panic!("not 0"),
};
/// Size of one directory entry.
const ENTRY_SIZE: u64 = raw::ENTRY_SIZE as u64;
const MAX_FILE_SIZE: u64 = u32::MAX as u64;
/// The first name byte of a deleted entry.
const DELETED: u8 = raw::ENTRY_FREE;
/// The boot sector's label when the volume has none.
const NO_NAME: &[u8; 11] = b"NO NAME    ";

/// State of a pinned node.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// Byte offset of the node's short entry on the volume.
    entry: u64,
    first: u32,
    size: u32,
    dir: bool,
    /// A known position in the chain, so sequential reads do not walk it
    /// from the start.
    hint: ChainPos,
    /// The directory entry lacks the size and modification time. A dirty
    /// node holds one pin of the driver's own until it is written.
    dirty: bool,
    /// Opens not yet closed; `unlink` and a replacing `rename` refuse the
    /// node while there are any.
    opens: u32,
    /// The node was removed while pinned. Its entry is gone and every
    /// method but `forget` and `close` answers `NotFound`.
    unlinked: bool,
}

impl Node {
    fn new(entry: u64, short: &ShortEntry, kind: FatKind) -> Self {
        Self {
            entry,
            first: short.first_cluster(kind),
            size: short.size(),
            dir: short.is_dir(),
            hint: ChainPos::NONE,
            dirty: false,
            opens: 0,
            unlinked: false,
        }
    }
}

/// Byte offset of the serial in the boot sector; the label follows it.
const fn bpb_serial_offset(kind: FatKind) -> u64 {
    match kind {
        FatKind::Fat32 => 0x43,
        _ => 0x27,
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
    /// The short entry at this byte offset, of a node that stays.
    Entry(u64),
    /// The short entry at this byte offset, of a node being removed.
    Removed(u64),
}

/// Clusters an unfinished operation holds: allocated but not yet linked,
/// or unlinked but not yet freed. The next writing operation frees them
/// unless `owner` links `head` on disk.
#[derive(Debug, Clone, Copy)]
struct Pending {
    /// The chain, and one more chain, allocated or being freed, that the
    /// chain does not reach: a single cluster, or one that leads into it.
    held: Held,
    owner: Owner,
}

impl Pending {
    const fn chain(head: u32, owner: Owner) -> Self {
        Self {
            held: Held::new(head, 0),
            owner,
        }
    }
}

/// Long-name slots `first..short` of `dir` that belong to the short entry
/// in slot `short`. An operation that writes or clears them sets this
/// first; the next writing operation clears them when that short entry is
/// not there.
#[derive(Debug, Clone, Copy)]
struct Run {
    dir: DirStart,
    first: u32,
    short: u32,
}

impl Run {
    fn of(dir: DirStart, first: u32, short: u32) -> Option<Self> {
        (first < short).then_some(Self { dir, first, short })
    }
}

fn metadata(node: &Node, entry: &ShortEntry, zone: Option<i16>) -> Metadata {
    crate::names::metadata(entry, node.dir, node.size as u64, zone)
}

/// Writes the entry's name into `out`: the long name when it is a valid
/// name, else the short name.
fn write_name(
    out: &mut NameBuf,
    long: Option<&[u16]>,
    entry: &ShortEntry,
    code_page: &dyn CodePage,
) -> Result<usize, ErrorKind> {
    if let Some(units) = long
        && out
            .fill(|buf| names::utf16_to_utf8(units.iter().copied(), buf).ok_or(NameError::TooLong))
            .is_ok()
    {
        return Ok(out.len());
    }
    let mut short = [0u8; short_name::DISPLAY_MAX];
    let len = short_name::display(
        &entry.name(),
        entry.nt_case(),
        |byte| code_page.decode(byte),
        &mut short,
    );
    out.set_bytes(&short[..len]).map_err(|err| match err {
        NameError::TooLong => ErrorKind::LimitExceeded,
        _ => ErrorKind::Corrupt,
    })?;
    Ok(len)
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

/// A FAT12, FAT16 or FAT32 volume on a block device.
///
/// `FatFs` implements the `hadris_fs` `FileSystem` trait, whose methods
/// take `&mut self`, hold no lock and need no allocator. Share it through
/// `hadris_fs` `Volume`, or call the trait methods directly.
///
/// Mount with [`mount`](FatFs::mount) and [`MountOptions`]:
///
/// - Nodes are identified by the location of their directory entry.
///   `lookup`, `create` and `parent` pin the node they return in a table
///   on the heap, and `forget` unpins it. A pinned node keeps its id across
///   `rename`. The table grows without a limit unless
///   [`MountOptions::with_node_limit`] caps it; past the cap `lookup` and
///   `create` fail with [`ErrorKind::LimitExceeded`] before anything is
///   written. Ids from `readdir` are not pinned and stay valid until that
///   directory changes; until then, and unless a node is forgotten in
///   between, they are the ids a `lookup` of the same names pins.
/// - The [`Clock`] stamps created and modified entries. The default
///   `NoClock` writes 1980-01-01, so images are reproducible; `SystemClock`
///   with `std` writes the current time.
/// - FAT stores local time with no zone. Timestamps are read and written
///   as UTC unless [`MountOptions::with_utc_offset`] names the zone.
/// - The [`CodePage`] maps short-name bytes above `0x7F`. The default is
///   `Cp437`; `Ascii` reads a byte `b` above `0x7F` as the private-use
///   character `U+F700 + b`, so distinct short names stay distinct and can
///   be looked up by the name listed.
///
/// Long names are always read and written. Names compare
/// case-insensitively, by the long name or by the short name, with each
/// UTF-16 unit folded by `hadris_fat_raw::fold_unicode` as Windows does,
/// so characters outside the Basic Multilingual Plane match only
/// exactly. A new name that is a valid
/// 8.3 name, in one case per part, is stored as a short entry alone;
/// others get long-name entries and a generated short name with a `~N`
/// tail, as Windows and Linux do.
///
/// The driver keeps one device block, at most 4096 bytes, inline as its
/// buffer, so a `FatFs` is a little over 4 KiB plus the node table. Reads
/// and writes of whole blocks go straight between the device and the
/// caller's buffer, one device call for each run of clusters that follow
/// one another on disk. On FAT16 and FAT32, growing and freeing a chain
/// writes the FAT a device block at a time. Devices with blocks larger than 4096 bytes are rejected
/// with [`ErrorKind::Unsupported`]; the device block may be smaller or
/// larger than the FAT sector.
///
/// # Writing
///
/// Writes go to the device at once; the driver caches no data. What it
/// defers is the size and modification time in the directory entry of a
/// pinned file: `write` and growing `truncate` keep them in the node table,
/// so every handle on the node sees one size, and `close`, `fsync` or
/// `sync` writes them. `close` does not flush the device; `fsync` and
/// `sync` do. Until then the node stays in the table under a pin of the
/// driver's own, even after the last `forget`, and counts towards
/// [`open_nodes`](Self::open_nodes) and the table's capacity. Writes through
/// an unpinned id, shrinking `truncate`, and a file's first cluster are
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
///   freed only after nothing does, so an interruption never leaves a
///   cross-linked or dangling chain. The driver remembers the clusters an
///   unfinished operation holds, allocated but not yet linked or unlinked
///   but not yet freed, and the next writing operation or `sync` frees
///   them unless the interrupted write did link them. Lost clusters remain
///   only when the process stops, or the driver is dropped, in between.
/// - `write` and `truncate` link new clusters before they publish the new
///   size, so an interrupted write leaves a chain longer than the size;
///   the next writing operation cuts it back. A shrinking `truncate` writes
///   the new size before it frees clusters.
/// - `create` prepares a new directory's cluster, then writes the name
///   entries, the short entry last. `unlink` and `rmdir` clear the short entry first.
///   Long-name entries an interruption leaves without their short entry
///   are cleared by the next writing operation.
/// - `rename` writes the new entry, then the moved directory's `..`, then
///   clears the old entry, so an interruption can leave the node under both
///   names.
///
/// That clean-up runs at the start of the next `create`, `mkdir`,
/// `unlink`, `rmdir`, `rename`, `write`, `truncate`, `setattr` or `sync`. What it
/// cannot finish because the volume turns out to be corrupt is dropped,
/// and `check` reports it as lost clusters.
/// - FAT copies are written active copy first. An entry whose mirrors an
///   interruption left behind is copied to them by the next FAT write or
///   `sync`, so after `sync` the copies match. The FSInfo free count is a
///   hint and is written by `sync`.
///
/// Data written by an interrupted `write` may be partly on disk.
pub struct FatFs<D> {
    dev: D,
    fat: Fat,
    nodes: Table<Node>,
    block: BlockBuf,
    /// Clusters an interrupted operation left to free.
    pending: Option<Pending>,
    /// Long-name slots an interrupted operation may have left without
    /// their short entry.
    run: Option<Run>,
    read_only: bool,
    /// FAT entry 1's clean bit was clear at mount.
    was_dirty: bool,
    clock: &'static dyn Clock,
    code_page: &'static dyn CodePage,
    /// The UTC offset of the volume's timestamps, `None` for UTC.
    zone: Option<i16>,
    /// Some pinned node is not at the slot its id names, because it was
    /// renamed or removed. Until the table empties, `pinned_at` searches it.
    moved: bool,
    /// A known `(index, cluster)` of a FAT32 root directory's chain.
    root_hint: ChainPos,
}

impl<D> fmt::Debug for FatFs<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FatFs")
            .field("kind", &self.fat.geometry().kind())
            .field("cluster_size", &self.fat.geometry().cluster_size())
            .field("open_nodes", &(self.nodes.len() + 1))
            .field("free_clusters", &self.fat.free_clusters())
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

impl<D: BlockDevice> FatFs<D> {
    /// Mounts the volume on `dev`. `options` set read-only, the clock, the
    /// UTC offset of the volume's timestamps, the code page of short names
    /// and the node cap. With [`MountOptions::backup_boot`] a FAT32 volume
    /// is mounted read-only from its backup boot sector (sector 6), which
    /// fails with [`ErrorKind::Corrupt`] when that is not valid; FAT12 and
    /// FAT16 have no backup and mount from the boot sector as usual.
    ///
    /// Fails with [`ErrorKind::NotRecognized`] when the first sector has no
    /// FAT BIOS parameter block (its sector and cluster sizes are not ones
    /// FAT allows), with [`ErrorKind::Corrupt`] when the boot sector is not a
    /// valid FAT12, FAT16 or FAT32 boot sector or describes a volume larger
    /// than the device, and with [`ErrorKind::Unsupported`] when the device's
    /// blocks are larger than 4096 bytes. The [`MountError`] gives `dev`
    /// back.
    pub async fn mount(mut dev: D, options: MountOptions) -> Result<Self, MountError<D, D::Error>> {
        let read_only = options.is_read_only() || !dev.writable();
        let mut block = match new_block(dev.block_size().get() as usize) {
            Ok(block) => block,
            Err(error) => return Err(MountError::new(error.into(), dev)),
        };
        let backup = options.is_backup_boot();
        let fat = match Self::read_volume(&mut dev, &mut block, backup).await {
            Ok(fat) => fat,
            Err(error) => return Err(MountError::new(error, dev)),
        };
        let read_only = read_only || (backup && fat.geometry().kind() == FatKind::Fat32);
        let geo = *fat.geometry();
        let kind = geo.kind();
        let mut entry = [0u8; 4];
        let at = geo.fat_copy(geo.active_fat()) + kind.entry_offset(1);
        let was_dirty = match kind.clean_bit() {
            0 => false,
            clean => match read_bytes(&mut dev, &mut block, at, &mut entry[..kind.entry_len()]).await {
                Ok(()) => kind.decode(1, &entry) & clean == 0,
                Err(error) => return Err(MountError::new(error, dev)),
            },
        };
        Ok(Self {
            dev,
            fat,
            nodes: Table::new(options.node_limit()),
            block,
            pending: None,
            run: None,
            read_only,
            was_dirty,
            clock: options.clock(),
            code_page: options.code_page(),
            zone: options.utc_offset(),
            moved: false,
            root_hint: ChainPos::NONE,
        })
    }

    /// Syncs the volume, as `FileSystem::sync` does, and gives the device
    /// back. When the sync fails the [`MountError`] holds its error and the
    /// device.
    pub async fn unmount(mut self) -> Result<D, MountError<D, D::Error>> {
        let synced = if self.read_only { Ok(()) } else { FileSystem::sync(&mut self).await };
        match synced {
            Ok(()) => Ok(self.dev),
            Err(error) => Err(MountError::new(error, self.dev)),
        }
    }

    /// The volume's state from its boot sector, or with `backup` from the
    /// FAT32 backup boot sector. A volume whose boot sector reads as FAT12
    /// or FAT16 has no backup and is read from its boot sector.
    async fn read_volume(dev: &mut D, block: &mut BlockBuf, backup: bool) -> FsResult<Fat, D::Error> {
        let geo = match backup {
            true => match rawio::read_geometry(dev, block).await {
                Ok(geo) if geo.kind() != FatKind::Fat32 => geo,
                Err(err) if err.kind() == ErrorKind::Io => return Err(err),
                primary => match rawio::read_backup_geometry(dev, block).await {
                    Ok(geo) => geo,
                    Err(err) => match primary {
                        Err(first) if first.kind() == ErrorKind::NotRecognized && err.kind() != ErrorKind::Io => {
                            return Err(first);
                        }
                        _ => return Err(err),
                    },
                },
            },
            false => rawio::read_geometry(dev, block).await?,
        };
        rawio::read_fat(dev, block, geo).await
    }

    /// Returns the device without syncing; [`unmount`](Self::unmount)
    /// syncs first.
    pub fn into_inner(self) -> D {
        self.dev
    }

    /// Whether the volume was mounted with
    /// [`MountOptions::read_only`] or on a device that is not
    /// [`writable`](BlockDevice::writable), or the device has refused a
    /// write since.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// The clock that stamps new and modified entries.
    pub fn clock(&self) -> &'static dyn Clock {
        self.clock
    }

    /// The code page of short names.
    pub fn code_page(&self) -> &'static dyn CodePage {
        self.code_page
    }

    /// The UTC offset of the volume's timestamps in minutes, `None` for
    /// UTC.
    pub fn utc_offset(&self) -> Option<i16> {
        self.zone
    }

    /// Number of nodes in the node table, plus one for the root, which is
    /// always pinned. Nodes whose size is not yet written count until
    /// `close`, `fsync` or `sync`.
    pub fn open_nodes(&self) -> usize {
        self.nodes.len() + 1
    }

    /// The volume's geometry from its boot sector, with its FAT variant
    /// and serial.
    pub fn info(&self) -> &Geometry {
        self.fat.geometry()
    }

    /// Whether the volume was not cleanly unmounted: the clean bit of FAT
    /// entry 1 was clear at mount. Always false on FAT12, which has no
    /// such bit.
    pub fn was_dirty(&self) -> bool {
        self.was_dirty
    }

    /// Maps `node` to the device, FIEMAP style: fills `out` with the runs
    /// of consecutive clusters that hold its bytes from file offset `from`
    /// on, and returns how many it filled. A run is whole clusters, except
    /// that a file's last run ends with the file. Call again from the end
    /// of the last run for more; 0 means there are none. An empty file has
    /// none, and the FAT12/16 root directory is one run, its fixed region.
    /// Fails with [`ErrorKind::Corrupt`] when the chain leaves the data
    /// clusters or is longer than the volume.
    pub async fn extents(&mut self, node: NodeId, from: u64, out: &mut [Extent]) -> FsResult<usize, D::Error> {
        let (first, len) = if node == ROOT {
            match self.fat.geometry().root() {
                RootLocation::Fixed { start, size } => {
                    return Ok(match out.first_mut() {
                        Some(slot) if from < size => {
                            *slot = Extent::new(start, size);
                            1
                        }
                        _ => 0,
                    });
                }
                RootLocation::Cluster(cluster) => (cluster, u64::MAX),
            }
        } else {
            let state = self.node(node).await?;
            (state.first, if state.dir { u64::MAX } else { state.size as u64 })
        };
        if first == 0 || out.is_empty() {
            return Ok(0);
        }
        let cluster_size = self.fat.geometry().cluster_size() as u64;
        let mut cluster = self.check_cluster(first)?;
        let mut count = 0usize;
        let mut run: Option<(u64, u64, u64)> = None;
        let mut index = 0u64;
        while index * cluster_size < len {
            let at = self.cluster_at(cluster)?;
            run = match run {
                Some((file, disk, bytes)) if disk + bytes == at => Some((file, disk, bytes + cluster_size)),
                Some(done) => {
                    if push_run(out, &mut count, done, from, len, len) {
                        return Ok(count);
                    }
                    Some((index * cluster_size, at, cluster_size))
                }
                None => Some((index * cluster_size, at, cluster_size)),
            };
            index += 1;
            if index >= self.fat.geometry().max_cluster() as u64 {
                return Err(ErrorKind::Corrupt.into());
            }
            match rawio::next(&mut self.dev, &mut self.block, &self.fat, cluster).await? {
                Some(next) => cluster = self.check_cluster(next)?,
                None => break,
            }
        }
        if let Some(done) = run {
            push_run(out, &mut count, done, from, len, len);
        }
        Ok(count)
    }

    /// Locates `node`'s on-disk records: its 32-byte short directory
    /// entry. Long-name entries are not included, and the root, which has
    /// no entry, has none. Returns how many it filled; fails with
    /// [`ErrorKind::LimitExceeded`] when `out` is empty and there is one.
    pub async fn records(&mut self, node: NodeId, out: &mut [Extent]) -> FsResult<usize, D::Error> {
        if node == ROOT {
            return Ok(0);
        }
        let state = self.node(node).await?;
        let slot = out.first_mut().ok_or(ErrorKind::LimitExceeded)?;
        *slot = Extent::new(state.entry, ENTRY_SIZE);
        Ok(1)
    }

    /// Reads `buf.len()` bytes of the device at byte `offset`, through the
    /// driver's block buffer.
    pub async fn read_raw(&mut self, offset: u64, buf: &mut [u8]) -> FsResult<(), D::Error> {
        read_bytes(&mut self.dev, &mut self.block, offset, buf).await
    }

    /// Sets the volume label, or removes it with `None`: the label entry
    /// of the root directory is rewritten, created in a free slot (the
    /// FAT32 root grows when it has none) or deleted, then the copy in the
    /// boot sector, and on FAT32 in the backup boot sector, is set, to
    /// `NO NAME` when removed. A boot sector without an extended boot
    /// signature has no copy.
    pub async fn set_label(&mut self, label: Option<VolumeLabel>) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let found = self.find_label().await?;
        match (found, label) {
            (Some((offset, _)), None) => self.put_bytes(offset, &[DELETED]).await?,
            (Some((offset, mut entry)), Some(label)) => {
                entry.set_name(*label.as_bytes());
                self.stamp_label(&mut entry);
                self.put_bytes(offset, &entry.encode()).await?;
            }
            (None, Some(label)) => {
                let mut entry = ShortEntry::new(*label.as_bytes(), raw::ATTR_VOLUME_ID);
                self.stamp_label(&mut entry);
                let start = self.dir_start(ROOT).await?;
                let new = NewName::new("LABEL", self.code_page, raw::fold_unicode)?;
                let plan = self.plan(start, "LABEL", false, &new, Skip::default()).await?;
                let grown = self.grow(&plan).await?;
                self.insert_entry(start, &new, &plan, &entry, grown).await?;
            }
            (None, None) => {}
        }
        if self.fat.geometry().volume_serial().is_some() {
            let bytes = label.map_or(*NO_NAME, |label| *label.as_bytes());
            self.put_boot(bpb_serial_offset(self.fat.geometry().kind()) + 4, &bytes).await?;
        }
        Ok(())
    }

    /// Writes `serial` to the boot sector, and on FAT32 to the backup boot
    /// sector. Fails with [`ErrorKind::Unsupported`] when the boot sector
    /// has no extended boot signature, and so no serial field.
    pub async fn set_volume_serial(&mut self, serial: u32) -> FsResult<(), D::Error> {
        self.prepare().await?;
        if self.fat.geometry().volume_serial().is_none() {
            return Err(ErrorKind::Unsupported.into());
        }
        self.put_boot(bpb_serial_offset(self.fat.geometry().kind()), &serial.to_le_bytes()).await?;
        self.fat.set_volume_serial(serial);
        Ok(())
    }

    /// Writes `bytes` at `offset` in the boot sector, and first in the
    /// FAT32 backup boot sector when that is valid.
    async fn put_boot(&mut self, offset: u64, bytes: &[u8]) -> FsResult<(), D::Error> {
        if self.fat.geometry().kind() == FatKind::Fat32
            && rawio::read_backup_geometry(&mut self.dev, &mut self.block).await.is_ok()
        {
            let backup = raw::layout::BACKUP_BOOT_SECTOR as u64 * self.fat.geometry().sector_size() as u64;
            self.put_bytes(backup + offset, bytes).await?;
        }
        self.put_bytes(offset, bytes).await
    }

    fn stamp_label(&self, entry: &mut ShortEntry) {
        let (date, time, tenths) = date::encode(self.now(), self.zone);
        entry.set_created(date, time, tenths);
        entry.set_modified(date, time);
        entry.set_accessed_date(date);
    }

    /// Where the last listing of `dir` left its chain, [`ChainPos::NONE`] when
    /// unknown.
    fn dir_hint(&self, dir: NodeId) -> ChainPos {
        if dir == ROOT {
            return self.root_hint;
        }
        self.nodes.get(dir).map_or(ChainPos::NONE, |node| node.hint)
    }

    /// Records a position in the chain of `dir` for the next
    /// `readdir` to start from. Shrinking a directory chain resets
    /// the positions.
    fn set_dir_hint(&mut self, dir: NodeId, hint: ChainPos) {
        if dir == ROOT {
            self.root_hint = hint;
        } else if let Some(node) = self.nodes.get_mut(dir)
            && node.dir
        {
            node.hint = hint;
        }
    }

    /// The root directory's label entry and its offset.
    async fn find_label(&mut self) -> FsResult<Option<(u64, ShortEntry)>, D::Error> {
        let mut walk = DirWalk::new(self.fat.root());
        let mut slot = 0;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await? {
            match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                Slot::End => break,
                Slot::Short(entry) if entry.is_label() => return Ok(Some((offset, entry))),
                _ => {}
            }
            slot += 1;
        }
        Ok(None)
    }

    /// Writes the node's pending size and modification time to its
    /// directory entry, without flushing the device.
    async fn publish_node(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        if node != ROOT {
            let (id, _) = self.any_node(node).await?;
            if let Some(id) = id {
                self.flush_node(id).await?;
            }
        }
        Ok(())
    }

    /// Creates `name` in `dir` as a file or directory and pins it.
    async fn create_node(
        &mut self,
        dir: NodeId,
        name: &Name,
        is_dir: bool,
        attrs: &SetAttr,
    ) -> FsResult<NodeId, D::Error> {
        self.prepare().await?;
        name.check()?;
        let start = self.dir_start(dir).await?;
        let text = entry_name(name, ErrorKind::InvalidInput)?;
        let new = NewName::new(text, self.code_page, raw::fold_unicode)?;
        let plan = self.plan(start, text, true, &new, Skip::default()).await?;
        let reserved = RESERVED;
        let placeholder = Node {
            entry: u64::MAX,
            first: 0,
            size: 0,
            dir: is_dir,
            hint: ChainPos::NONE,
            dirty: false,
            opens: 0,
            unlinked: false,
        };
        self.nodes
            .insert(reserved, placeholder)
            .map_err(|_| ErrorKind::LimitExceeded)?;
        let node = match self.create_entry(start, &new, &plan, is_dir, attrs).await {
            Ok(node) => node,
            Err(err) => {
                self.nodes.remove(reserved);
                return Err(err);
            }
        };
        self.nodes.remove(reserved);
        let id = self.free_id(node.entry).ok_or(ErrorKind::LimitExceeded)?;
        self.nodes
            .insert(id, node)
            .map_err(|_| ErrorKind::LimitExceeded)?;
        Ok(id)
    }

    /// Removes the file (or, with `want_dir`, the empty directory) `name`
    /// from `dir` and frees its clusters.
    async fn remove_entry(&mut self, dir: NodeId, name: &Name, want_dir: bool) -> FsResult<(), D::Error> {
        self.prepare().await?;
        name.check()?;
        let start = self.dir_start(dir).await?;
        let query = entry_name(name, ErrorKind::NotFound)?;
        let found = self.find_entry(start, query).await?.ok_or(ErrorKind::NotFound)?;
        match (want_dir, found.entry.is_dir()) {
            (false, true) => return Err(ErrorKind::IsADirectory.into()),
            (true, false) => return Err(ErrorKind::NotADirectory.into()),
            _ => {}
        }
        let pinned = self.pinned_at(found.offset);
        if pinned.is_some_and(|id| self.is_open(id)) {
            return Err(ErrorKind::Busy.into());
        }
        let first = match pinned.and_then(|id| self.nodes.get(id)) {
            Some(node) => node.first,
            None => found.entry.first_cluster(self.fat.geometry().kind()),
        };
        if found.entry.is_dir() && !self.dir_is_empty(self.check_cluster(first)?).await? {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        if first != 0 {
            self.pending = Some(Pending::chain(first, Owner::Removed(found.offset)));
        }
        self.run = Run::of(start, found.first, found.slot);
        self.put_bytes(found.offset, &[raw::ENTRY_FREE]).await?;
        if let Some(id) = pinned {
            self.mark_unlinked(id);
        }
        self.clear_slots(start, found.first, found.slot).await?;
        self.run = None;
        if first != 0 {
            self.free_chain(first).await?;
        }
        Ok(())
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

    /// Copies FAT entries an interrupted write did not mirror, clears
    /// long-name slots it left without their short entry, and frees the
    /// clusters it held unless it linked them. What cannot be finished
    /// because the volume is corrupt is dropped.
    async fn recover(&mut self) -> FsResult<(), D::Error> {
        match self.finish_interrupted().await {
            Err(err) if err.kind() == ErrorKind::Corrupt => {
                self.fat.forget_unmirrored();
                self.run = None;
                self.pending = None;
                Ok(())
            }
            other => other,
        }
    }

    async fn finish_interrupted(&mut self) -> FsResult<(), D::Error> {
        if self.fat.unmirrored().is_some() {
            let mirrored = rawio::mirror(&mut self.dev, &mut self.block, &mut self.fat).await;
            self.note(mirrored)?;
        }
        if let Some(run) = self.run {
            let mut walk = DirWalk::new(run.dir);
            let short = match rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, run.short).await? {
                Some(at) => matches!(rawio::read_slot(&mut self.dev, &mut self.block, at).await?, Slot::Short(entry) if entry.is_visible()),
                None => true,
            };
            if !short {
                let cleared =
                    rawio::clear_slots(&mut self.dev, &mut self.block, &self.fat, run.dir, run.first, run.short)
                        .await;
                self.note(cleared)?;
            }
            self.run = None;
        }
        let Some(pending) = self.pending else {
            return Ok(());
        };
        let (head, extra) = (pending.held.head(), pending.held.extra());
        let mut owned = head != 0 && self.links(pending.owner, head).await?;
        if let Owner::Tail(tail) = pending.owner
            && owned
        {
            self.set_fat(tail, self.fat.geometry().kind().end_of_chain()).await?;
            owned = false;
        }
        match pending.owner {
            Owner::Entry(offset) => self.reconcile(offset).await?,
            Owner::Removed(offset) if !owned => {
                if let Some(id) = self.pinned_at(offset)
                    && self.nodes.get(id).is_some_and(|node| node.first == head)
                {
                    self.mark_unlinked(id);
                }
            }
            _ => {}
        }
        if !owned {
            self.pending = Some(Pending {
                owner: Owner::None,
                ..pending
            });
            if extra != 0 {
                self.reclaim(extra, head).await?;
            }
            if head != 0 {
                self.reclaim(head, 0).await?;
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
                self.check_cluster(prev).is_ok()
                    && rawio::get(&mut self.dev, &mut self.block, &self.fat, prev).await? & self.fat.geometry().kind().mask() == head
            }
            Owner::Entry(offset) | Owner::Removed(offset) => matches!(
                rawio::read_slot(&mut self.dev, &mut self.block, offset).await?,
                Slot::Short(entry) if entry.is_visible() && entry.first_cluster(self.fat.geometry().kind()) == head
            ),
        })
    }

    /// Brings a pinned node at `offset` in line with its entry after an
    /// interrupted write to that entry.
    async fn reconcile(&mut self, offset: u64) -> FsResult<(), D::Error> {
        let Some(id) = self.pinned_at(offset) else {
            return Ok(());
        };
        let Slot::Short(entry) = rawio::read_slot(&mut self.dev, &mut self.block, offset).await? else {
            return Ok(());
        };
        let first = entry.first_cluster(self.fat.geometry().kind());
        let stale = self.nodes.get(id).is_some_and(|node| node.first != first);
        if stale {
            self.clean(id);
            if let Some(node) = self.nodes.get_mut(id) {
                node.first = first;
                node.size = entry.size();
                node.hint = ChainPos::NONE;
            }
        }
        Ok(())
    }

    /// Frees the clusters of a chain from `head` for as long as they are
    /// allocated, stopping at a free or bad cluster, the end, or a link out
    /// of range. `keep` stays pending once the chain ends.
    async fn reclaim(&mut self, head: u32, keep: u32) -> FsResult<(), D::Error> {
        let kind = self.fat.geometry().kind();
        let mut cluster = head;
        for _ in 0..self.fat.geometry().max_cluster() {
            if self.check_cluster(cluster).is_err() {
                break;
            }
            let stored = rawio::get(&mut self.dev, &mut self.block, &self.fat, cluster).await? & kind.mask();
            if stored == 0 || kind.is_bad(stored) {
                break;
            }
            let next = kind.next(stored, self.fat.geometry().max_cluster()).ok().flatten();
            self.pending = Some(Pending {
                held: Held::new(next.unwrap_or(keep), cluster),
                owner: Owner::None,
            });
            self.set_fat(cluster, 0).await?;
            match next {
                Some(next) => cluster = next,
                None => break,
            }
        }
        Ok(())
    }

    /// Pins held by callers, not counting the driver's own pin on a dirty
    /// node.
    fn user_pins(&self, id: NodeId) -> u32 {
        let dirty = self.nodes.get(id).is_some_and(|node| node.dirty);
        self.nodes.pins(id).saturating_sub(dirty as u32)
    }

    fn is_open(&self, id: NodeId) -> bool {
        self.nodes.get(id).is_some_and(|node| node.opens > 0)
    }

    /// Drops the table state of a node whose entry was just removed. A node
    /// callers still pin stays, marked unlinked, so its id answers
    /// `NotFound` until the last `forget`.
    fn mark_unlinked(&mut self, id: NodeId) {
        self.clean(id);
        if self.nodes.pins(id) == 0 {
            self.nodes.remove(id);
        } else if let Some(node) = self.nodes.get_mut(id) {
            self.moved = true;
            node.unlinked = true;
            node.entry = u64::MAX;
            node.first = 0;
            node.size = 0;
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

    /// The parent cluster a `..` entry records for a child of `dir`.
    fn parent_cluster(&self, dir: DirStart) -> u32 {
        match dir {
            DirStart::Chain(cluster) if self.fat.geometry().root() != RootLocation::Cluster(cluster) => cluster,
            _ => 0,
        }
    }

    async fn put(&mut self, offset: u64, data: Option<&[u8]>, len: usize) -> FsResult<(), D::Error> {
        let result = write_bytes(&mut self.dev, &mut self.block, offset, data, len).await;
        self.note(result)
    }

    /// Marks the volume read-only when the device refused a write.
    fn note<R>(&mut self, result: FsResult<R, D::Error>) -> FsResult<R, D::Error> {
        if let Err(err) = &result
            && err.kind() == ErrorKind::ReadOnly
        {
            self.read_only = true;
        }
        result
    }

    async fn put_bytes(&mut self, offset: u64, data: &[u8]) -> FsResult<(), D::Error> {
        self.put(offset, Some(data), data.len()).await
    }

    async fn flush_device(&mut self) -> FsResult<(), D::Error> {
        if self.read_only {
            return Ok(());
        }
        let result = self.dev.flush().await;
        if let Err(err) = &result
            && err.kind() == ErrorKind::ReadOnly
        {
            self.read_only = true;
        }
        result
    }

    async fn read_short(&mut self, offset: u64) -> FsResult<ShortEntry, D::Error> {
        match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
            Slot::Short(entry) => Ok(entry),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    /// Records the node's size and first cluster in `entry` and marks it
    /// modified now.
    fn touch(&self, entry: &mut ShortEntry, node: &Node) {
        if !node.dir {
            entry.set_size(node.size);
        }
        entry.set_first_cluster(self.fat.geometry().kind(), node.first);
        let (date, time, _) = date::encode(self.now(), self.zone);
        entry.set_modified(date, time);
        entry.set_accessed_date(date);
        entry.set_attributes(entry.attributes() | raw::ATTR_ARCHIVE);
    }

    /// Writes a file's first cluster, size and modification time to its
    /// entry and updates its table state.
    async fn store(
        &mut self,
        id: Option<NodeId>,
        state: &Node,
        first: u32,
        size: u32,
        hint: ChainPos,
    ) -> FsResult<(), D::Error> {
        let next = Node { first, size, hint, ..*state };
        let mut entry = self.read_short(state.entry).await?;
        self.touch(&mut entry, &next);
        self.put_bytes(state.entry, &entry.encode()).await?;
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
        hint: ChainPos,
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
            if node.unlinked {
                return Err(ErrorKind::NotFound.into());
            }
            return Ok((Some(id), *node));
        }
        let (offset, entry) = self.unpinned(id).await?;
        match self.pinned_at(offset) {
            Some(pinned) => Ok((Some(pinned), *self.nodes.get(pinned).ok_or(ErrorKind::Corrupt)?)),
            None => Ok((None, Node::new(offset, &entry, self.fat.geometry().kind()))),
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
        let mut walk = DirWalk::new(start);
        let mut slot = 0;
        let mut long = Assembler::new();
        while let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? {
            let units = long.finish(found.entry.lfn_checksum());
            let named = units.is_some();
            let units = units.filter(|units| !units.is_empty());
            if matches(query, units, &found.entry, self.code_page, raw::fold_unicode) {
                return Ok(Some(Located {
                    first: if named { found.long_start } else { found.slot },
                    slot: found.slot,
                    offset: found.offset,
                    exact: is_exact(query, units, &found.entry, self.code_page),
                    entry: found.entry,
                }));
            }
        }
        Ok(None)
    }

    async fn dir_is_empty(&mut self, first: u32) -> FsResult<bool, D::Error> {
        let mut walk = DirWalk::new(DirStart::Chain(first));
        let mut slot = 0;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await? {
            match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                Slot::End => break,
                Slot::Short(entry) if entry.is_visible() => return Ok(false),
                _ => {}
            }
            slot += 1;
        }
        Ok(true)
    }

    /// Whether `dir` is the directory starting at `ancestor` or below it.
    async fn is_within(&mut self, dir: DirStart, ancestor: u32) -> FsResult<bool, D::Error> {
        let DirStart::Chain(mut cluster) = dir else {
            return Ok(false);
        };
        for _ in 0..=self.fat.geometry().max_cluster() {
            if cluster == ancestor {
                return Ok(true);
            }
            if self.fat.geometry().root() == RootLocation::Cluster(cluster) {
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
        new: &NewName<'_>,
        skip: Skip,
    ) -> FsResult<Plan, D::Error> {
        let needed = new.slots();
        let mut walk = DirWalk::new(dir);
        let mut long = Assembler::new();
        let mut slot = 0;
        let mut end = false;
        let mut taken = 0u8;
        if new.candidates[CANDIDATES - 1][0] == 0 {
            taken |= 1 << (CANDIDATES - 1);
        }
        let (mut run_start, mut run_len) = (0, 0);
        let mut found = None;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await? {
            let reused = skip.run.is_some_and(|(first, last)| (first..=last).contains(&slot));
            if reused {
                long.reset();
            }
            let free = end
                || reused
                || match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                    Slot::End => {
                        end = true;
                        true
                    }
                    Slot::Free => {
                        long.reset();
                        true
                    }
                    Slot::Long(part) => {
                        long.push(&part);
                        false
                    }
                    Slot::Short(entry) => {
                        let units = long
                            .finish(entry.lfn_checksum())
                            .filter(|units| !units.is_empty());
                        if !skip.covers(slot, offset) {
                            if check_exists && entry.is_visible() && matches(text, units, &entry, self.code_page, raw::fold_unicode) {
                                return Err(ErrorKind::AlreadyExists.into());
                            }
                            for (bit, candidate) in new.candidates.iter().enumerate() {
                                if entry.name() == *candidate {
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
                    || start as u64 + needed as u64 > raw::MAX_DIR_ENTRIES as u64
                {
                    return Err(ErrorKind::NoSpace.into());
                }
                let per_cluster = self.fat.geometry().cluster_size() / ENTRY_SIZE as u32;
                (start, (needed - run_len).div_ceil(per_cluster), walk.pos().cluster())
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
        for suffix in CANDIDATES as u8..=u8::MAX {
            let Some(mut candidate) = crate::names::short_name(text, suffix, self.code_page, raw::fold_unicode) else {
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
        let mut walk = DirWalk::new(dir);
        let mut slot = 0;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await? {
            match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                Slot::End => break,
                Slot::Short(entry) if entry.name() == *name && !skip.covers(slot, offset) => {
                    return Ok(true);
                }
                _ => {}
            }
            slot += 1;
        }
        Ok(false)
    }

    /// Appends the zeroed clusters `plan` needs to its directory. Returns
    /// the first, 0 when none were needed.
    async fn grow(&mut self, plan: &Plan) -> FsResult<u32, D::Error> {
        if plan.grow == 0 {
            return Ok(0);
        }
        let grown = self.allocate_chain(plan.grow, true, Owner::Cluster(plan.tail)).await?;
        if let Err(err) = self.set_fat(plan.tail, grown).await {
            let _ = self.recover().await;
            return Err(err);
        }
        self.pending = None;
        Ok(grown)
    }

    /// Takes back the clusters [`grow`](Self::grow) appended. Best effort.
    /// Another pending chain is kept.
    async fn ungrow(&mut self, plan: &Plan, grown: u32) {
        if grown == 0 {
            return;
        }
        let held = self.pending.replace(Pending::chain(grown, Owner::Cluster(plan.tail)));
        let _ = self.set_fat(plan.tail, self.fat.geometry().kind().end_of_chain()).await;
        self.root_hint = ChainPos::NONE;
        while let Some(id) = self.nodes.find(|_, node| node.dir && node.hint.is_known()) {
            if let Some(node) = self.nodes.get_mut(id) {
                node.hint = ChainPos::NONE;
            }
        }
        if self.recover().await.is_ok() {
            self.pending = held;
        }
    }

    /// Writes `entry` and the long name of `new` into the run `plan` found,
    /// in a directory [`grow`](Self::grow) has grown by `grown`. Returns
    /// the short entry's offset. A pending allocation is owned by the short
    /// entry from its write on.
    async fn insert_entry(
        &mut self,
        dir: DirStart,
        new: &NewName<'_>,
        plan: &Plan,
        entry: &ShortEntry,
        grown: u32,
    ) -> FsResult<u64, D::Error> {
        self.run = Run::of(dir, plan.start, plan.start + plan.slots - 1);
        let checksum = entry.lfn_checksum();
        let mut walk = DirWalk::new(dir);
        let mut written = 0;
        let result = match rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, plan.start + plan.slots - 1).await {
            Ok(Some(offset)) => {
                if let Some(pending) = self.pending.as_mut()
                    && pending.owner == Owner::None
                {
                    pending.owner = Owner::Entry(offset);
                }
                let result = rawio::write_slots(
                    &mut self.dev,
                    &mut self.block,
                    &self.fat,
                    &mut walk,
                    plan.start,
                    plan.slots,
                    |index| {
                        if index + 1 == plan.slots {
                            entry.encode()
                        } else {
                            let (sequence, units) = new.encoded.entry(index as usize);
                            LongEntry::new(sequence, checksum, &units).encode()
                        }
                    },
                    &mut written,
                )
                .await;
                self.note(result).map(|()| offset)
            }
            Ok(None) => Err(ErrorKind::Corrupt.into()),
            Err(err) => Err(err),
        };
        let err = match result {
            Ok(offset) => {
                self.run = None;
                return Ok(offset);
            }
            Err(err) => err,
        };
        if self.clear_slots(dir, plan.start, plan.start + written).await.is_ok() {
            self.run = None;
        }
        self.ungrow(plan, grown).await;
        Err(err)
    }

    /// Marks slots `from..to` of `dir` deleted.
    async fn clear_slots(&mut self, dir: DirStart, from: u32, to: u32) -> FsResult<(), D::Error> {
        let result = rawio::clear_slots(&mut self.dev, &mut self.block, &self.fat, dir, from, to).await;
        self.note(result)
    }

    /// Writes a new node's entries: a directory's cluster with `.` and `..`
    /// first, then the name.
    async fn create_entry(
        &mut self,
        dir: DirStart,
        new: &NewName<'_>,
        plan: &Plan,
        is_dir: bool,
        attrs: &SetAttr,
    ) -> FsResult<Node, D::Error> {
        let kind = self.fat.geometry().kind();
        let now = self.now();
        let attr = if is_dir { raw::ATTR_DIRECTORY } else { raw::ATTR_ARCHIVE };
        let mut entry = ShortEntry::new(plan.short, attr);
        entry.set_nt_case(plan.nt_case);
        if let Some(attributes) = attrs.attributes() {
            apply_attributes(&mut entry, attributes);
        }
        if attrs.permissions().is_some_and(|mode| mode.bits() & 0o222 == 0) {
            set_read_only(&mut entry, true);
        }
        stamp(
            &mut entry,
            attrs.created().unwrap_or(now),
            attrs.modified().unwrap_or(now),
            attrs.accessed().unwrap_or(now),
            self.zone,
        );
        let grown = self.grow(plan).await?;
        let mut first = 0;
        if is_dir {
            first = match self.allocate_chain(1, true, Owner::None).await {
                Ok(first) => first,
                Err(err) => {
                    self.ungrow(plan, grown).await;
                    return Err(err);
                }
            };
            entry.set_first_cluster(kind, first);
            let mut dot = entry;
            dot.set_name(*b".          ");
            dot.set_nt_case(0);
            dot.set_attributes(raw::ATTR_DIRECTORY);
            let mut dot_dot = dot;
            dot_dot.set_name(*b"..         ");
            dot_dot.set_first_cluster(kind, self.parent_cluster(dir));
            let mut raw = [0u8; 2 * ENTRY_SIZE as usize];
            raw[..32].copy_from_slice(&dot.encode());
            raw[32..].copy_from_slice(&dot_dot.encode());
            let written = match self.cluster_at(first) {
                Ok(at) => self.put_bytes(at, &raw).await,
                Err(kind) => Err(kind.into()),
            };
            if let Err(err) = written {
                let _ = self.recover().await;
                self.ungrow(plan, grown).await;
                return Err(err);
            }
        }
        let offset = match self.insert_entry(dir, new, plan, &entry, grown).await {
            Ok(offset) => offset,
            Err(err) => {
                let _ = self.recover().await;
                return Err(err);
            }
        };
        self.pending = None;
        Ok(Node {
            entry: offset,
            first,
            size: 0,
            dir: is_dir,
            hint: ChainPos::NONE,
            dirty: false,
            opens: 0,
            unlinked: false,
        })
    }

    async fn set_dot_dot(&mut self, dir: u32, parent: u32) -> FsResult<(), D::Error> {
        let at = self.cluster_at(dir)? + ENTRY_SIZE;
        let mut entry = self.read_short(at).await?;
        if !entry.is_dot_dot() {
            return Err(ErrorKind::Corrupt.into());
        }
        entry.set_first_cluster(self.fat.geometry().kind(), parent);
        self.put_bytes(at, &entry.encode()).await
    }

    /// Writes `moved` under the name `new` where `plan` found room in `to`,
    /// which [`grow`](Self::grow) has grown by `grown`, then frees `src`'s
    /// short entry. On failure the new entry is cleared
    /// and `saved` written back. `src`'s long-name slots are left to the
    /// caller.
    #[allow(clippy::too_many_arguments)]
    async fn move_entry(
        &mut self,
        from: DirStart,
        src: &Located,
        to: DirStart,
        new: &NewName<'_>,
        plan: &Plan,
        grown: u32,
        moved: ShortEntry,
        dot_dot: Option<(u32, u32, u32)>,
        src_id: Option<NodeId>,
        saved: Option<&SavedRun>,
    ) -> FsResult<(), D::Error> {
        let mut entry = moved;
        entry.set_name(plan.short);
        entry.set_nt_case(plan.nt_case);
        let offset = match self.insert_entry(to, new, plan, &entry, grown).await {
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
        self.run = Run::of(from, src.first, src.slot);
        if let Err(err) = self.put_bytes(src.offset, &[raw::ENTRY_FREE]).await {
            self.run = None;
            if let Some((dir, old, _)) = dot_dot {
                let _ = self.set_dot_dot(dir, old).await;
            }
            let _ = self.clear_slots(to, plan.start, end).await;
            self.restore_run(to, saved).await;
            return Err(err);
        }
        if let Some(node) = src_id.and_then(|id| self.nodes.get_mut(id)) {
            node.entry = offset;
            self.moved = true;
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
        new: &NewName<'_>,
        moved: ShortEntry,
        dot_dot: Option<(u32, u32, u32)>,
        src_id: Option<NodeId>,
    ) -> FsResult<(), D::Error> {
        let target_id = self.pinned_at(target.offset);
        if target_id.is_some_and(|id| self.is_open(id)) {
            return Err(ErrorKind::Busy.into());
        }
        match (moved.is_dir(), target.entry.is_dir()) {
            (true, false) => return Err(ErrorKind::NotADirectory.into()),
            (false, true) => return Err(ErrorKind::IsADirectory.into()),
            _ => {}
        }
        let target_first = match target_id.and_then(|id| self.nodes.get(id)) {
            Some(node) => node.first,
            None => target.entry.first_cluster(self.fat.geometry().kind()),
        };
        if target.entry.is_dir() && !self.dir_is_empty(self.check_cluster(target_first)?).await? {
            return Err(ErrorKind::DirectoryNotEmpty.into());
        }
        let skip = Skip {
            entry: (from == to).then_some(src.offset),
            run: Some((target.first, target.slot)),
        };
        let plan = self.plan(to, text, false, new, skip).await?;
        let mut saved = SavedRun {
            first: target.first,
            len: 0,
            raw: [[0; ENTRY_SIZE as usize]; lfn::MAX_ENTRIES + 1],
        };
        self.save_run(to, target.slot, &mut saved).await?;
        let grown = self.grow(&plan).await?;
        let held = (target_first != 0).then(|| Pending::chain(target_first, Owner::Removed(target.offset)));
        self.pending = held;
        self.run = Run::of(to, target.first, target.slot);
        let cleared = match self.put_bytes(target.offset, &[raw::ENTRY_FREE]).await {
            Ok(()) => self.clear_slots(to, target.first, target.slot).await,
            Err(err) => Err(err),
        };
        if let Err(err) = cleared {
            self.restore_run(to, Some(&saved)).await;
            self.run = None;
            let _ = self.recover().await;
            return Err(err);
        }
        if let Err(err) = self
            .move_entry(from, src, to, new, &plan, grown, moved, dot_dot, src_id, Some(&saved))
            .await
        {
            self.pending = held;
            let _ = self.recover().await;
            return Err(err);
        }
        self.pending = held;
        if let Some(id) = target_id {
            self.mark_unlinked(id);
        }
        self.clear_slots(from, src.first, src.slot).await?;
        self.run = None;
        if target_first != 0 {
            self.free_chain(target_first).await?;
        }
        Ok(())
    }

    /// Reads slots `saved.first..=last` of `dir`, at most one long name and
    /// its short entry, into `saved`.
    async fn save_run(&mut self, dir: DirStart, last: u32, saved: &mut SavedRun) -> FsResult<(), D::Error> {
        let len = last - saved.first + 1;
        if len as usize > saved.raw.len() {
            return Err(ErrorKind::Corrupt.into());
        }
        let mut walk = DirWalk::new(dir);
        for (slot, raw) in (saved.first..=last).zip(saved.raw.iter_mut()) {
            let at = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await?.ok_or(ErrorKind::Corrupt)?;
            read_bytes(&mut self.dev, &mut self.block, at, raw).await?;
        }
        saved.len = len;
        Ok(())
    }

    /// Writes back the slots `save_run` read, as far as the device allows.
    async fn restore_run(&mut self, dir: DirStart, saved: Option<&SavedRun>) {
        let Some(saved) = saved else {
            return;
        };
        let mut walk = DirWalk::new(dir);
        for (slot, raw) in (saved.first..).zip(&saved.raw[..saved.len as usize]) {
            if let Ok(Some(at)) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, &mut walk, slot).await {
                let _ = self.put_bytes(at, raw).await;
            }
        }
    }

    /// Stores `value` as the FAT entry of `cluster`, in the active copy
    /// first, then in the mirrors.
    async fn set_fat(&mut self, cluster: u32, value: u32) -> FsResult<(), D::Error> {
        let result = rawio::set(&mut self.dev, &mut self.block, &mut self.fat, cluster, value).await;
        self.note(result)
    }

    /// Allocates a chain of `count` clusters, zeroed when `zero` is set, and
    /// returns its first cluster. The chain stays pending, to be linked
    /// through `owner`; the caller clears `pending` once it is. Nothing is
    /// left allocated on failure.
    async fn allocate_chain(&mut self, count: u32, zero: bool, owner: Owner) -> FsResult<u32, D::Error> {
        let pending = self.pending.insert(Pending::chain(0, owner));
        let allocated = if zero {
            self.allocate_zeroed(count).await
        } else {
            let result = rawio::allocate_run(
                &mut self.dev,
                &mut self.block,
                &mut self.fat,
                Some(&mut pending.held),
                count,
            )
            .await;
            self.note(result)
        };
        if allocated.is_err() {
            let _ = self.recover().await;
        }
        allocated
    }

    /// Allocates a chain of `count` zeroed clusters one by one, recording
    /// them in the pending allocation.
    async fn allocate_zeroed(&mut self, count: u32) -> FsResult<u32, D::Error> {
        let (mut first, mut last) = (0, 0);
        for _ in 0..count {
            let held = self.pending.as_mut().map(|pending| &mut pending.held);
            let result = rawio::allocate(&mut self.dev, &mut self.block, &mut self.fat, held).await;
            let cluster = self.note(result)?;
            let at = self.fat.cluster_at(cluster)?;
            self.put(at, None, self.fat.geometry().cluster_size() as usize).await?;
            if last != 0 {
                self.set_fat(last, cluster).await?;
                if let Some(pending) = self.pending.as_mut() {
                    pending.held.set_extra(0);
                }
            }
            if first == 0 {
                first = cluster;
            }
            last = cluster;
        }
        Ok(first)
    }

    /// Frees the chain starting at `first`, which nothing links any more,
    /// and clears `pending`. What is left of the chain stays pending when
    /// that fails.
    async fn free_chain(&mut self, first: u32) -> FsResult<(), D::Error> {
        let held = self.pending.map_or(Held::NONE, |pending| pending.held);
        let pending = self.pending.insert(Pending { held, owner: Owner::None });
        let result =
            rawio::free_chain(&mut self.dev, &mut self.block, &mut self.fat, &mut pending.held, first).await;
        self.note(result)?;
        self.pending = None;
        Ok(())
    }

    /// Extends a file's chain to hold `end` bytes. The new clusters are
    /// linked but not zeroed, and stay pending until the caller has
    /// recorded the new size.
    async fn cover(&mut self, state: &Node, end: u64) -> FsResult<Growth, D::Error> {
        let need = end.div_ceil(self.fat.geometry().cluster_size() as u64) as u32;
        let none = Growth { first: state.first, added: 0 };
        if need == 0 {
            return Ok(none);
        }
        if state.first == 0 {
            let first = self.allocate_chain(need, false, Owner::Entry(state.entry)).await?;
            return Ok(Growth { first, added: first });
        }
        let reached = rawio::walk(&mut self.dev, &mut self.block, &self.fat, state.first, state.hint, need - 1).await?;
        let (index, tail) = (reached.index(), reached.cluster());
        if index + 1 >= need {
            return Ok(none);
        }
        let added = self.allocate_chain(need - 1 - index, false, Owner::Tail(tail)).await?;
        if let Err(err) = self.set_fat(tail, added).await {
            let _ = self.recover().await;
            return Err(err);
        }
        Ok(Growth { first: state.first, added })
    }

    /// Frees what [`cover`](Self::cover) added, which is still pending.
    /// Best effort: on failure the clusters stay allocated.
    async fn undo_growth(&mut self, growth: Growth) {
        if growth.added != 0 {
            let _ = self.recover().await;
        }
    }

    /// Writes `len` bytes of `data`, or zeros, at byte `pos` of the chain at
    /// `first`, and returns a hint for the last cluster written.
    async fn fill(
        &mut self,
        first: u32,
        hint: ChainPos,
        pos: u64,
        data: Option<&[u8]>,
        len: usize,
    ) -> FsResult<ChainPos, D::Error> {
        if len == 0 {
            return Ok(hint);
        }
        let cluster_size = self.fat.geometry().cluster_size() as u64;
        let mut hint = hint;
        let mut done = 0;
        while done < len {
            let at = pos + done as u64;
            let want = (at / cluster_size) as u32;
            hint = rawio::walk(&mut self.dev, &mut self.block, &self.fat, first, hint, want).await?;
            if hint.index() < want {
                return Err(ErrorKind::Corrupt.into());
            }
            let within = at % cluster_size;
            let offset = self.cluster_at(hint.cluster())? + within;
            let n = ((cluster_size - within) as usize).min(len - done);
            let n = rawio::run(&mut self.dev, &mut self.block, &self.fat, &mut hint, n, len - done).await?;
            self.put(offset, data.map(|data| &data[done..done + n]), n).await?;
            done += n;
        }
        Ok(hint)
    }


    fn check_cluster(&self, cluster: u32) -> Result<u32, ErrorKind> {
        self.fat.check_cluster(cluster)
    }

    fn cluster_at(&self, cluster: u32) -> Result<u64, ErrorKind> {
        self.fat.cluster_at(cluster)
    }

    /// The pinned node whose short entry is at `offset`. Unless a pinned
    /// node has moved, that is the tier-0 id of the slot, found by key.
    fn pinned_at(&self, offset: u64) -> Option<NodeId> {
        if !self.moved {
            let id = node_id(offset / ENTRY_SIZE);
            return self.nodes.get(id).is_some_and(|node| node.entry == offset).then_some(id);
        }
        self.nodes.find(|_, node| node.entry == offset)
    }

    /// The first id for the entry at `offset` that no node in the table
    /// holds. `None` only when every generation of the slot is taken.
    fn free_id(&self, offset: u64) -> Option<NodeId> {
        let slot = offset / ENTRY_SIZE;
        (0..=MAX_TIER)
            .map(|tier| node_id(tier << SLOT_BITS | slot))
            .find(|&id| self.nodes.get(id).is_none())
    }

    /// The id of the entry at `offset` without pinning it: its pinned id,
    /// else the id a lookup would pin.
    fn id_at(&self, offset: u64) -> NodeId {
        self.pinned_at(offset)
            .or_else(|| self.free_id(offset))
            .unwrap_or(node_id(offset / ENTRY_SIZE))
    }

    /// Pins the node whose short entry is at `offset`.
    fn intern(&mut self, offset: u64, entry: &ShortEntry) -> Result<NodeId, ErrorKind> {
        if let Some(id) = self.pinned_at(offset) {
            self.nodes.pin(id);
            return Ok(id);
        }
        let id = self.free_id(offset).ok_or(ErrorKind::LimitExceeded)?;
        self.nodes
            .insert(id, Node::new(offset, entry, self.fat.geometry().kind()))
            .map_err(|_| ErrorKind::LimitExceeded)?;
        Ok(id)
    }


    /// The entry of an id that is not pinned, decoded from its location.
    async fn unpinned(&mut self, id: NodeId) -> FsResult<(u64, ShortEntry), D::Error> {
        let raw = id.get();
        if raw == ROOT.get() || raw >= RESERVED.get() {
            return Err(ErrorKind::InvalidHandle.into());
        }
        let offset = (raw & SLOT_MASK) * ENTRY_SIZE;
        let in_root = matches!(
            self.fat.geometry().root(),
            RootLocation::Fixed { start, size } if (start..start + size).contains(&offset)
        );
        if !in_root && !(self.fat.geometry().data_start()..self.fat.geometry().data_end()).contains(&offset) {
            return Err(ErrorKind::InvalidHandle.into());
        }
        match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
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
        match rawio::read_slot(&mut self.dev, &mut self.block, node.entry).await? {
            Slot::Short(entry) => Ok((node, entry)),
            _ => Err(ErrorKind::Corrupt.into()),
        }
    }

    async fn dir_start(&mut self, dir: NodeId) -> FsResult<DirStart, D::Error> {
        if dir == ROOT {
            return Ok(self.fat.root());
        }
        let node = self.node(dir).await?;
        if !node.dir {
            return Err(ErrorKind::NotADirectory.into());
        }
        Ok(DirStart::Chain(self.check_cluster(node.first)?))
    }




    /// Scans from `slot` to the next visible short entry, feeding long-name
    /// fragments to `long`, and leaves `slot` after it.
    async fn next_visible(
        &mut self,
        walk: &mut DirWalk,
        slot: &mut u32,
        long: &mut Assembler,
    ) -> FsResult<Option<Found>, D::Error> {
        let mut long_start = *slot;
        while let Some(offset) = rawio::slot_offset(&mut self.dev, &mut self.block, &self.fat, walk, *slot).await? {
            let at = *slot;
            match rawio::read_slot(&mut self.dev, &mut self.block, offset).await? {
                Slot::End => break,
                Slot::Free => long.reset(),
                Slot::Long(part) => {
                    if part.sequence() & raw::LFN_LAST_ENTRY != 0 {
                        long_start = at;
                    }
                    long.push(&part)
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
        let Slot::Short(entry) = rawio::read_slot(&mut self.dev, &mut self.block, offset).await? else {
            return Err(ErrorKind::Corrupt.into());
        };
        if !entry.is_dot_dot() {
            return Err(ErrorKind::Corrupt.into());
        }
        let cluster = entry.first_cluster(self.fat.geometry().kind());
        if cluster == 0 || self.fat.geometry().root() == RootLocation::Cluster(cluster) {
            return Ok(None);
        }
        Ok(Some(self.check_cluster(cluster)?))
    }
}

impl<D: BlockDevice> FileSystem for FatFs<D> {
    type DeviceError = D::Error;

    /// What this volume supports: case-insensitive, case-preserving UTF-16
    /// names of up to 255 code units (765 bytes of UTF-8), two-second
    /// modification times, creation times, access dates, the DOS attributes
    /// and, through the read-only attribute, part of the permissions.
    /// Writable unless [`is_read_only`](FatFs::is_read_only).
    fn capabilities(&self) -> Capabilities {
        let caps = Capabilities::new(CaseRule::InsensitivePreserving, Charset::Unicode, lfn::MAX_UNITS * 3)
            .with_stored(Field::Created, Stored::Yes)
            .with_stored(Field::Modified, Stored::Yes)
            .with_stored(Field::Accessed, Stored::Partial)
            .with_stored(Field::Permissions, Stored::Partial)
            .with_stored(Field::Attributes, Stored::Yes)
            .with_timestamp_resolution_ns(2_000_000_000);
        if self.read_only { caps } else { caps.with_writable() }
    }

    /// The root directory. Always pinned.
    fn root(&self) -> NodeId {
        ROOT
    }

    /// Space usage in clusters. Free space comes from the FAT32 FSInfo
    /// sector when it is valid, and otherwise from one scan of the FAT,
    /// which is then kept. The FSInfo count is a hint: allocation scans the
    /// FAT regardless, and one that finds it wrong drops it.
    async fn statfs(&mut self) -> FsResult<FsStats, D::Error> {
        let free = match self.fat.free_clusters() {
            Some(free) => free,
            None => rawio::count_free(&mut self.dev, &mut self.block, &mut self.fat).await?,
        };
        let total = self.fat.geometry().max_cluster() - 1;
        Ok(FsStats::new(total as u64, free as u64, self.fat.geometry().cluster_size()))
    }

    /// The volume label from the label entry of the root directory, or
    /// `None` when it has none. The copy in the boot sector is not read.
    /// Bytes above `0x7F` are decoded through the mount's code page, as in
    /// short names.
    async fn label<'b>(&mut self, buf: &'b mut [u8]) -> FsResult<Option<&'b str>, D::Error> {
        let Some((_, entry)) = self.find_label().await? else {
            return Ok(None);
        };
        let label = entry.name();
        let mut text = [0u8; short_name::DISPLAY_MAX];
        let len = short_name::display_label(&label, |byte| self.code_page.decode(byte), &mut text);
        let out = buf.get_mut(..len).ok_or(ErrorKind::LimitExceeded)?;
        out.copy_from_slice(&text[..len]);
        Ok(Some(core::str::from_utf8(out).map_err(|_| ErrorKind::Corrupt)?))
    }

    /// Finds `name` in `dir` and pins the result. Case is ignored, and the
    /// short name of an entry with a long name matches too.
    async fn lookup(&mut self, dir: NodeId, name: &Name) -> FsResult<NodeId, D::Error> {
        name.check()?;
        let start = self.dir_start(dir).await?;
        let Ok(query) = name.to_str() else {
            return Err(ErrorKind::NotFound.into());
        };
        let found = self.find_entry(start, query).await?.ok_or(ErrorKind::NotFound)?;
        Ok(self.intern(found.offset, &found.entry)?)
    }

    /// Drops `count` pins of a node. Unknown ids and the root are ignored.
    /// A node whose size is not yet written stays in the table until
    /// `close`, `fsync` or `sync`; a removed node leaves the table with its
    /// last pin.
    fn forget(&mut self, node: NodeId, count: u64) {
        if node == ROOT {
            return;
        }
        for _ in 0..count.min(u64::from(self.user_pins(node))) {
            if self.nodes.unpin(node) == Some(0)
                && self.nodes.get(node).is_some_and(|n| n.unlinked)
            {
                self.nodes.remove(node);
                break;
            }
        }
        if self.nodes.is_empty() {
            self.moved = false;
        }
    }

    /// The directory containing `dir`, pinned. The root is its own parent.
    ///
    /// Found through `..` entries: the parent's first cluster, then the
    /// grandparent, which is scanned for the parent's entry.
    async fn parent(&mut self, dir: NodeId) -> FsResult<NodeId, D::Error> {
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
            None => self.fat.root(),
        };
        let mut walk = DirWalk::new(grand);
        let mut slot = 0;
        let mut long = Assembler::new();
        while let Some(found) = self.next_visible(&mut walk, &mut slot, &mut long).await? {
            if found.entry.is_dir() && found.entry.first_cluster(self.fat.geometry().kind()) == up {
                return Ok(self.intern(found.offset, &found.entry)?);
            }
        }
        Err(ErrorKind::Corrupt.into())
    }

    /// Metadata of a pinned node or of an id just returned by
    /// `readdir`. Directories have length 0.
    async fn stat(&mut self, node: NodeId) -> FsResult<Metadata, D::Error> {
        if node == ROOT {
            return Ok(Metadata::new(FileType::Dir, permissions(true, false)));
        }
        let (state, entry) = self.node_entry(node).await?;
        Ok(metadata(&state, &entry, self.zone))
    }

    /// The entry at or after `from`, or `None` at the end. `.`, `..`, the
    /// volume label and deleted entries are skipped. The raw cursor is the
    /// index of a directory slot. A directory whose cluster chain loops
    /// fails with [`ErrorKind::Corrupt`] once the walk comes back around.
    async fn readdir(&mut self, dir: NodeId, from: DirCursor) -> FsResult<Option<DirEntry>, D::Error> {
        let start = self.dir_start(dir).await?;
        let Ok(mut slot) = u32::try_from(from.into_raw()) else {
            return Ok(None);
        };
        let mut walk = DirWalk::resume(start, self.dir_hint(dir));
        let mut long = Assembler::new();
        let found = self.next_visible(&mut walk, &mut slot, &mut long).await?;
        if walk.pos().is_known() {
            self.set_dir_hint(dir, walk.pos());
        }
        let Some(found) = found else {
            return Ok(None);
        };
        let units = long.finish(found.entry.lfn_checksum());
        let mut name = NameBuf::new();
        write_name(&mut name, units.filter(|units| !units.is_empty()), &found.entry, self.code_page)?;
        let node = self.id_at(found.offset);
        let state = match self.pinned_at(found.offset).and_then(|id| self.nodes.get(id)) {
            Some(state) => *state,
            None => Node::new(found.offset, &found.entry, self.fat.geometry().kind()),
        };
        let name = name.as_name().ok_or(ErrorKind::Corrupt)?;
        let next = DirCursor::from_raw(found.slot as u64 + 1);
        let entry = DirEntry::new(name, node, metadata(&state, &found.entry, self.zone), next)
            .map_err(|_| ErrorKind::Corrupt)?;
        Ok(Some(entry))
    }

    /// FAT has no symlinks: fails with [`ErrorKind::InvalidInput`] for any
    /// node that exists.
    async fn readlink<'b>(&mut self, node: NodeId, buf: &'b mut [u8]) -> FsResult<&'b [u8], D::Error> {
        let _ = buf;
        if node != ROOT {
            self.node(node).await?;
        }
        Err(ErrorKind::InvalidInput.into())
    }

    /// Opens a pinned file: until the matching `close`, `unlink` and a
    /// replacing `rename` of it fail with [`ErrorKind::Busy`]. Fails with
    /// [`ErrorKind::IsADirectory`] for a directory, [`ErrorKind::ReadOnly`]
    /// for [`OpenMode::Write`] on a read-only volume,
    /// [`ErrorKind::InvalidHandle`] for an id that is not pinned, and
    /// [`ErrorKind::NotFound`] for a removed node.
    async fn open(&mut self, node: NodeId, mode: OpenMode) -> FsResult<(), D::Error> {
        if node == ROOT {
            return Err(ErrorKind::IsADirectory.into());
        }
        let read_only = self.read_only;
        match self.nodes.get_mut(node) {
            Some(state) if state.unlinked => Err(ErrorKind::NotFound.into()),
            Some(state) if state.dir => Err(ErrorKind::IsADirectory.into()),
            Some(_) if mode == OpenMode::Write && read_only => Err(ErrorKind::ReadOnly.into()),
            Some(state) => {
                state.opens = state.opens.saturating_add(1);
                Ok(())
            }
            None => Err(ErrorKind::InvalidHandle.into()),
        }
    }

    /// Ends one `open` and writes the node's pending size and modification
    /// time to its directory entry, without flushing the device.
    async fn close(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        if let Some(state) = self.nodes.get_mut(node) {
            state.opens = state.opens.saturating_sub(1);
        }
        self.publish_node(node).await
    }

    /// Reads from a file at `offset`. Returns 0 at or past the end. A chain
    /// that ends before the file's size or loops fails with
    /// [`ErrorKind::Corrupt`].
    async fn read(&mut self, node: NodeId, offset: u64, buf: &mut [u8]) -> FsResult<usize, D::Error> {
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
        let cluster_size = self.fat.geometry().cluster_size() as u64;
        let mut hint = state.hint;
        let mut done = 0;
        while done < count {
            let pos = offset + done as u64;
            let want = (pos / cluster_size) as u32;
            hint = rawio::walk(&mut self.dev, &mut self.block, &self.fat, state.first, hint, want).await?;
            if hint.index() < want {
                return Err(ErrorKind::Corrupt.into());
            }
            let within = pos % cluster_size;
            let at = self.cluster_at(hint.cluster())? + within;
            let n = ((cluster_size - within) as usize).min(count - done);
            let n = rawio::run(&mut self.dev, &mut self.block, &self.fat, &mut hint, n, count - done).await?;
            read_bytes(&mut self.dev, &mut self.block, at, &mut buf[done..done + n]).await?;
            done += n;
        }
        if let Some(state) = self.nodes.get_mut(node) {
            state.hint = hint;
        }
        Ok(count)
    }

    /// Changes attributes, times and permissions. Permissions are stored
    /// only as the read-only attribute, so only the values FAT reports (see
    /// `stat`) are accepted; others and an owner fail with
    /// [`ErrorKind::Unsupported`], as does any change to the root, which has
    /// no entry.
    /// A pending size is written too.
    async fn setattr(&mut self, node: NodeId, changes: &SetAttr) -> FsResult<(), D::Error> {
        self.prepare().await?;
        if changes.owner().is_some() {
            return Err(ErrorKind::Unsupported.into());
        }
        if node == ROOT {
            return match changes.is_empty() {
                true => Ok(()),
                false => Err(ErrorKind::Unsupported.into()),
            };
        }
        let (id, state) = self.any_node(node).await?;
        let read_only = match changes.permissions() {
            Some(wanted) => Some(read_only_bit(state.dir, wanted).ok_or(ErrorKind::Unsupported)?),
            None => None,
        };
        if !state.dirty && changes.is_empty() {
            return Ok(());
        }
        let mut entry = self.read_short(state.entry).await?;
        if state.dirty {
            self.touch(&mut entry, &state);
        }
        if let Some(attributes) = changes.attributes() {
            apply_attributes(&mut entry, attributes);
        }
        if let Some(read_only) = read_only {
            set_read_only(&mut entry, read_only);
        }
        if let Some(time) = changes.created() {
            let (date, time, tenths) = date::encode(time, self.zone);
            entry.set_created(date, time, tenths);
        }
        if let Some(time) = changes.modified() {
            let (date, time, _) = date::encode(time, self.zone);
            entry.set_modified(date, time);
        }
        if let Some(time) = changes.accessed() {
            entry.set_accessed_date(date::encode(time, self.zone).0);
        }
        self.put_bytes(state.entry, &entry.encode()).await?;
        if let Some(id) = id {
            self.clean(id);
        }
        Ok(())
    }

    /// Writes to a file at `offset`, growing it and zero-filling any gap
    /// past the old end. Returns the bytes written, fewer than `buf.len()`
    /// only at the 4 GiB - 1 FAT size limit; an `offset` at or past it fails
    /// with [`ErrorKind::FileTooLarge`], and a volume without room for the
    /// new clusters fails with [`ErrorKind::NoSpace`] and changes nothing.
    ///
    /// The new size of a pinned file is written by `close`,
    /// `fsync` or `sync`, with the modification time and the archive
    /// attribute.
    async fn write(&mut self, node: NodeId, offset: u64, buf: &[u8]) -> FsResult<usize, D::Error> {
        self.prepare().await?;
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
        let hint = if growth.first == state.first { state.hint } else { ChainPos::NONE };
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
        self.pending = None;
        Ok(count)
    }

    /// Truncates or extends a file. Growth reads as zeros. Shrinking writes
    /// the new size to the directory entry at once and frees the clusters
    /// past it. A changed size sets the archive attribute; the same size
    /// changes nothing. A length past 4 GiB - 1 fails with
    /// [`ErrorKind::FileTooLarge`].
    async fn truncate(&mut self, node: NodeId, len: u64) -> FsResult<(), D::Error> {
        self.prepare().await?;
        let (id, state) = self.file_node(node).await?;
        if len > MAX_FILE_SIZE {
            return Err(ErrorKind::FileTooLarge.into());
        }
        let old = state.size as u64;
        if len > old {
            let growth = self.cover(&state, len).await?;
            let hint = if growth.first == state.first { state.hint } else { ChainPos::NONE };
            let published = match self.fill(growth.first, hint, old, None, (len - old) as usize).await {
                Ok(hint) => self.publish(id, &state, growth.first, len as u32, hint).await,
                Err(err) => Err(err),
            };
            if let Err(err) = published {
                self.undo_growth(growth).await;
                return Err(err);
            }
            self.pending = None;
            return Ok(());
        }
        if len == old {
            return Ok(());
        }
        let keep = len.div_ceil(self.fat.geometry().cluster_size() as u64) as u32;
        let first = if keep == 0 { 0 } else { state.first };
        if keep == 0 && state.first != 0 {
            self.pending = Some(Pending::chain(state.first, Owner::Entry(state.entry)));
        }
        self.store(id, &state, first, len as u32, ChainPos::NONE).await?;
        if state.first == 0 {
            return Ok(());
        }
        if keep == 0 {
            return self.free_chain(state.first).await;
        }
        let reached = rawio::walk(&mut self.dev, &mut self.block, &self.fat, state.first, ChainPos::NONE, keep - 1).await?;
        let last = reached.cluster();
        if reached.index() == keep - 1
            && let Some(next) = rawio::next(&mut self.dev, &mut self.block, &self.fat, last).await?
        {
            self.pending = Some(Pending::chain(next, Owner::Cluster(last)));
            self.set_fat(last, self.fat.geometry().kind().end_of_chain()).await?;
            self.free_chain(next).await?;
        }
        Ok(())
    }

    /// Writes the node's pending size and modification time, then flushes
    /// the device.
    async fn fsync(&mut self, node: NodeId) -> FsResult<(), D::Error> {
        self.publish_node(node).await?;
        self.flush_device().await
    }

    /// Creates the empty file `name` in `dir` and pins it.
    ///
    /// `attrs` sets the attributes, which default to archive, the creation,
    /// modification and access times, which default to now, and, when it
    /// has no write bits, the read-only attribute; owners and other
    /// permissions are ignored. Fails with [`ErrorKind::AlreadyExists`]
    /// when a long or short name matches, [`ErrorKind::InvalidInput`] for a
    /// name FAT cannot hold (control characters, `"*/:<>?\|`, or a trailing
    /// dot or space, refused rather than stripped so a created name is the
    /// name listed), and [`ErrorKind::NoSpace`] when a FAT12/16 root
    /// directory is full or a directory would pass 65536 entries.
    async fn create(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, D::Error> {
        self.create_node(dir, name, false, attrs).await
    }

    /// Creates the empty directory `name` in `dir` and pins it, as `create`
    /// does for a file.
    async fn mkdir(&mut self, dir: NodeId, name: &Name, attrs: &SetAttr) -> FsResult<NodeId, D::Error> {
        self.create_node(dir, name, true, attrs).await
    }

    /// Removes the file `name` from `dir` and frees its clusters. Fails with
    /// [`ErrorKind::IsADirectory`] for a directory and with
    /// [`ErrorKind::Busy`] while the node is open. A node that is pinned but
    /// not open is removed; its id then answers [`ErrorKind::NotFound`]
    /// until its last `forget`.
    async fn unlink(&mut self, dir: NodeId, name: &Name) -> FsResult<(), D::Error> {
        self.remove_entry(dir, name, false).await
    }

    /// Removes the empty directory `name` from `dir`, as `unlink` does a
    /// file. Fails with [`ErrorKind::NotADirectory`] for anything else and
    /// [`ErrorKind::DirectoryNotEmpty`] for a directory with entries.
    async fn rmdir(&mut self, dir: NodeId, name: &Name) -> FsResult<(), D::Error> {
        self.remove_entry(dir, name, true).await
    }

    /// Moves `from` in `from_dir` to `to` in `to_dir`. The moved node keeps
    /// its `NodeId` when it is pinned. A renamed file gets the archive
    /// attribute, as the FAT specification and Windows do; a directory
    /// keeps its attributes.
    ///
    /// An existing `to` is replaced unless `mode` is
    /// [`RenameMode::NoReplace`] ([`ErrorKind::AlreadyExists`]): a file by
    /// a file, an empty directory by a directory. The result is named `to`
    /// as given, even when `to` matches the target in another case or by its
    /// short alias, and the new name takes the target's slots when it fits
    /// them, so a full FAT12/16 root directory still allows the replace. A
    /// pinned target fails with [`ErrorKind::Busy`].
    /// Moving a directory into itself or below, or a `to` that `create`
    /// would refuse, fails with [`ErrorKind::InvalidInput`].
    async fn rename(
        &mut self,
        from_dir: NodeId,
        from: &Name,
        to_dir: NodeId,
        to: &Name,
        mode: RenameMode,
    ) -> FsResult<(), D::Error> {
        self.prepare().await?;
        from.check()?;
        to.check()?;
        let from_start = self.dir_start(from_dir).await?;
        let to_start = self.dir_start(to_dir).await?;
        let from_text = entry_name(from, ErrorKind::NotFound)?;
        let to_text = entry_name(to, ErrorKind::InvalidInput)?;
        let new = NewName::new(to_text, self.code_page, raw::fold_unicode)?;
        let src = self
            .find_entry(from_start, from_text)
            .await?
            .ok_or(ErrorKind::NotFound)?;
        let src_id = self.pinned_at(src.offset);
        let src_node = match src_id.and_then(|id| self.nodes.get(id)) {
            Some(node) => *node,
            None => Node::new(src.offset, &src.entry, self.fat.geometry().kind()),
        };
        if src_node.dir {
            self.check_cluster(src_node.first)?;
        }
        if src_node.dir && self.is_within(to_start, src_node.first).await? {
            return Err(ErrorKind::InvalidInput.into());
        }
        let mut moved = src.entry;
        moved.set_first_cluster(self.fat.geometry().kind(), src_node.first);
        if !src_node.dir {
            moved.set_size(src_node.size);
            moved.set_attributes(moved.attributes() | raw::ATTR_ARCHIVE);
        }
        let dot_dot = (src_node.dir && self.parent_cluster(from_start) != self.parent_cluster(to_start))
            .then(|| (src_node.first, self.parent_cluster(from_start), self.parent_cluster(to_start)));
        match self.find_entry(to_start, to_text).await? {
            Some(target) if target.offset == src.offset && target.exact => return Ok(()),
            Some(target) if target.offset != src.offset => {
                if mode == RenameMode::NoReplace {
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
                let grown = self.grow(&plan).await?;
                self.move_entry(from_start, &src, to_start, &new, &plan, grown, moved, dot_dot, src_id, None)
                    .await?;
            }
        }
        self.clear_slots(from_start, src.first, src.slot).await?;
        self.run = None;
        Ok(())
    }

    /// Writes every pending size and modification time and the FAT32
    /// FSInfo free count, then flushes the device.
    ///
    /// A node whose short entry cannot be read any more does not stop the
    /// others: its pending size is dropped, the rest is written, and `sync`
    /// then fails with [`ErrorKind::Corrupt`].
    async fn sync(&mut self) -> FsResult<(), D::Error> {
        if !self.read_only {
            self.recover().await?;
        }
        let mut corrupt = None;
        while let Some(id) = self.nodes.find(|_, node| node.dirty) {
            match self.flush_node(id).await {
                Err(err) if err.kind() == ErrorKind::Corrupt => {
                    self.clean(id);
                    corrupt = Some(err);
                }
                other => other?,
            }
        }
        let written = rawio::write_fs_info(&mut self.dev, &mut self.block, &mut self.fat).await;
        self.note(written)?;
        self.flush_device().await?;
        corrupt.map_or(Ok(()), Err)
    }
}

}
