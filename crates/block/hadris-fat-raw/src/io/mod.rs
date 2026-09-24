//! Device primitives: the reads and writes a FAT driver is made of,
//! generated for each mode from one source.
//!
//! The functions in `sync`, `r#async` and `async_send`, one module for
//! each mode feature, are generic over the device only. They borrow the
//! caller's [`BlockBuf`], which caches one device block, and a [`Fat`],
//! which holds what the driver tracks about the allocation tables between
//! calls. Sequences whose order matters when they are interrupted live
//! here once: `set` writes the active FAT copy before the others and
//! records the entry until every copy has it, and `allocate_run` and
//! `free_chain` write the FAT a device block at a time in an order that
//! never leaves an entry pointing to a free cluster, recording their
//! progress in a [`Held`].
//!
//! The mode-independent types are here: [`BlockBuf`], [`Fat`], [`Held`],
//! [`ChainPos`], [`DirStart`] and [`DirWalk`].

use hadris_fs::ErrorKind;

use crate::boot::{Geometry, RootLocation};
use crate::chain::ChainGuard;
use crate::dirent::ENTRY_SIZE;
use crate::entry::FIRST_DATA_CLUSTER;

/// One device block of buffering, borrowed by every primitive.
///
/// `B` is the storage: an array such as `[u8; 512]` or `[u8; 4096]` owned
/// by the caller. The primitives take `&mut BlockBuf`, the unsized form,
/// which any `&mut BlockBuf<[u8; N]>` converts to. The buffer remembers
/// which device block it holds, so reads within one block cost one device
/// call.
#[derive(Debug, Clone)]
pub struct BlockBuf<B: ?Sized = [u8]> {
    size: usize,
    cached: Option<u64>,
    data: B,
}

impl<const N: usize> BlockBuf<[u8; N]> {
    /// A buffer for device blocks of `block_size` bytes. `None` when the
    /// size is 0 or larger than `N`.
    pub const fn new(block_size: usize) -> Option<Self> {
        if block_size == 0 || block_size > N {
            return None;
        }
        Some(Self {
            size: block_size,
            cached: None,
            data: [0; N],
        })
    }
}

impl<B: ?Sized + AsRef<[u8]> + AsMut<[u8]>> BlockBuf<B> {
    /// The device block size the buffer was made for.
    pub fn block_size(&self) -> usize {
        self.size
    }

    /// The device block the buffer holds, if any.
    pub fn cached(&self) -> Option<u64> {
        self.cached
    }

    /// Forgets the block the buffer holds, so the next read goes to the
    /// device.
    pub fn invalidate(&mut self) {
        self.cached = None;
    }

    /// The bytes of the block the buffer holds.
    pub fn contents(&self) -> &[u8] {
        &self.data.as_ref()[..self.size]
    }

    /// The bytes of the block, to change before a `store`. The buffer no
    /// longer counts as holding a device block.
    pub fn contents_mut(&mut self) -> &mut [u8] {
        self.cached = None;
        &mut self.data.as_mut()[..self.size]
    }
}

impl BlockBuf {
    fn scratch(&mut self, len: usize) -> &mut [u8] {
        self.cached = None;
        &mut self.data[..len]
    }

    fn capacity(&self) -> usize {
        self.data.len()
    }
}

/// What a driver tracks about a volume's FATs between calls: the
/// [`Geometry`], the free cluster count and allocation hint from FSInfo or
/// counting, and the FAT entries whose copies an interrupted write may have
/// left different.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fat {
    geo: Geometry,
    free: Option<u32>,
    next_free: u32,
    fs_info: Option<u64>,
    info_dirty: bool,
    unmirrored: Option<(u32, u32)>,
}

impl Fat {
    /// The state of a volume with geometry `geo`: free count unknown and
    /// allocation starting at cluster 2.
    pub const fn new(geo: Geometry) -> Self {
        Self {
            geo,
            free: None,
            next_free: FIRST_DATA_CLUSTER,
            fs_info: None,
            info_dirty: false,
            unmirrored: None,
        }
    }

    /// The volume's geometry.
    pub const fn geometry(&self) -> &Geometry {
        &self.geo
    }

    /// The number of free clusters, when known.
    pub const fn free_clusters(&self) -> Option<u32> {
        self.free
    }

    /// Where the search for a free cluster starts.
    pub const fn next_free(&self) -> u32 {
        self.next_free
    }

    /// Byte offset of a valid FAT32 FSInfo sector.
    pub const fn fs_info(&self) -> Option<u64> {
        self.fs_info
    }

    /// Whether the free count or allocation hint changed since FSInfo was
    /// last written.
    pub const fn fs_info_dirty(&self) -> bool {
        self.info_dirty
    }

    /// The clusters, first and last, whose FAT entries may differ between
    /// the copies because a write to them was interrupted.
    pub const fn unmirrored(&self) -> Option<(u32, u32)> {
        self.unmirrored
    }

    /// Stops tracking entries an interrupted write left unmirrored, when
    /// the volume turned out too damaged to copy them.
    pub fn forget_unmirrored(&mut self) {
        self.unmirrored = None;
    }

    /// Whether `cluster` is a data cluster.
    pub const fn is_cluster(&self, cluster: u32) -> bool {
        cluster >= FIRST_DATA_CLUSTER && cluster <= self.geo.max_cluster()
    }

    /// `cluster`, or [`ErrorKind::Corrupt`] when it is not a data cluster.
    pub const fn check_cluster(&self, cluster: u32) -> Result<u32, ErrorKind> {
        if self.is_cluster(cluster) {
            Ok(cluster)
        } else {
            Err(ErrorKind::Corrupt)
        }
    }

    /// Byte offset of a data cluster; [`ErrorKind::Corrupt`] for any other.
    pub const fn cluster_at(&self, cluster: u32) -> Result<u64, ErrorKind> {
        match self.geo.cluster_offset(cluster) {
            Some(at) => Ok(at),
            None => Err(ErrorKind::Corrupt),
        }
    }

    /// Where the root directory's slots are.
    pub const fn root(&self) -> DirStart {
        DirStart::root(&self.geo)
    }

    fn adjust_free(&mut self, freed: u32, taken: u32) {
        let total = self.geo.max_cluster() - 1;
        self.free = self
            .free
            .and_then(|free| (free + freed).checked_sub(taken))
            .map(|free| free.min(total));
        self.info_dirty = true;
    }
}

/// The clusters an unfinished allocation or free holds, so a driver can
/// release them after an interruption: `head` starts a chain still to be
/// linked or freed, and `extra` is one more cluster `head` does not reach.
/// 0 means none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Held {
    head: u32,
    extra: u32,
}

impl Held {
    /// Nothing held.
    pub const NONE: Self = Self { head: 0, extra: 0 };

    /// The chain at `head` and the extra cluster `extra`.
    pub const fn new(head: u32, extra: u32) -> Self {
        Self { head, extra }
    }

    /// The chain still to be linked or freed.
    pub const fn head(&self) -> u32 {
        self.head
    }

    /// One more cluster the chain at `head` does not reach.
    pub const fn extra(&self) -> u32 {
        self.extra
    }

    /// Replaces the chain.
    pub fn set_head(&mut self, head: u32) {
        self.head = head;
    }

    /// Replaces the extra cluster.
    pub fn set_extra(&mut self, extra: u32) {
        self.extra = extra;
    }
}

/// A position in a cluster chain: the cluster at index `index`, reached by
/// a walk from the chain's first cluster, with the [`ChainGuard`] of that
/// walk, so a later walk can resume from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainPos {
    index: u32,
    cluster: u32,
    guard: ChainGuard,
}

impl ChainPos {
    /// No known position.
    pub const NONE: Self = Self {
        index: 0,
        cluster: 0,
        guard: ChainGuard::new(0),
    };

    /// The cluster `cluster` at index `index` of a chain. A walk from it
    /// detects loops from there on.
    pub const fn new(index: u32, cluster: u32) -> Self {
        Self {
            index,
            cluster,
            guard: ChainGuard::new(cluster),
        }
    }

    /// The first cluster of a chain.
    pub const fn start(first: u32) -> Self {
        Self {
            index: 0,
            cluster: first,
            guard: ChainGuard::new(first),
        }
    }

    /// The index of the cluster in its chain.
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// The cluster; 0 when no position is known.
    pub const fn cluster(&self) -> u32 {
        self.cluster
    }

    /// Whether a position is known.
    pub const fn is_known(&self) -> bool {
        self.cluster != 0
    }

    /// Moves to `next`, the cluster after this one. `false` when the chain
    /// has looped.
    pub fn advance(&mut self, next: u32) -> bool {
        if !self.guard.step(next) {
            return false;
        }
        self.cluster = next;
        self.index += 1;
        true
    }
}

/// Where a directory's slots are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirStart {
    /// The FAT12/16 root: `slots` slots from byte `start`.
    Fixed {
        /// Byte offset of the first slot.
        start: u64,
        /// The number of slots.
        slots: u32,
    },
    /// A cluster chain starting at the given cluster.
    Chain(u32),
}

impl DirStart {
    /// The root directory of a volume with geometry `geo`.
    pub const fn root(geo: &Geometry) -> Self {
        match geo.root() {
            RootLocation::Fixed { start, size } => Self::Fixed {
                start,
                slots: (size / ENTRY_SIZE as u64) as u32,
            },
            RootLocation::Cluster(cluster) => Self::Chain(cluster),
        }
    }
}

/// A directory being read slot by slot, with the position its chain was
/// last walked to, so consecutive slots cost no FAT reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirWalk {
    start: DirStart,
    at: ChainPos,
}

impl DirWalk {
    /// A walk of the directory at `start` from its first slot.
    pub const fn new(start: DirStart) -> Self {
        Self {
            start,
            at: ChainPos::NONE,
        }
    }

    /// A walk that resumes from `at`, a position in the directory's chain.
    pub const fn resume(start: DirStart, at: ChainPos) -> Self {
        Self { start, at }
    }

    /// The directory.
    pub const fn start(&self) -> DirStart {
        self.start
    }

    /// The position its chain was last walked to.
    pub const fn pos(&self) -> ChainPos {
        self.at
    }
}

/// Clusters whose entries share a device block, as bits from `base`.
pub(crate) struct ClusterGroup {
    base: u32,
    bits: [u64; ClusterGroup::SPAN as usize / 64],
    pub(crate) count: u32,
}

impl ClusterGroup {
    /// Most clusters a group holds: the FAT16 entries of a 4096-byte block.
    pub(crate) const SPAN: u32 = 2048;

    pub(crate) fn new(base: u32) -> Self {
        Self {
            base,
            bits: [0; Self::SPAN as usize / 64],
            count: 0,
        }
    }

    pub(crate) fn spans(&self, cluster: u32) -> bool {
        cluster.wrapping_sub(self.base) < Self::SPAN
    }

    pub(crate) fn has(&self, cluster: u32) -> bool {
        let bit = cluster.wrapping_sub(self.base);
        bit < Self::SPAN && self.bits[bit as usize / 64] & (1 << (bit % 64)) != 0
    }

    pub(crate) fn add(&mut self, cluster: u32) {
        let bit = cluster.wrapping_sub(self.base);
        if bit < Self::SPAN && !self.has(cluster) {
            self.bits[bit as usize / 64] |= 1 << (bit % 64);
            self.count += 1;
        }
    }

    pub(crate) fn descending(&self) -> impl Iterator<Item = u32> + '_ {
        (0..Self::SPAN)
            .rev()
            .filter(|&bit| self.bits[bit as usize / 64] & (1 << (bit % 64)) != 0)
            .map(|bit| self.base + bit)
    }

    pub(crate) fn lowest(&self) -> u32 {
        self.descending().last().unwrap_or(self.base)
    }
}

#[cfg(feature = "sync")]
#[path = ""]
pub mod sync {
    //! The synchronous primitives.

    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::strip_async!{ $($item)* } };
    }

    use hadris_storage::sync as storage;

    #[path = "block.rs"]
    mod block;
    #[path = "check.rs"]
    mod check;
    #[path = "fat.rs"]
    mod fat;

    pub use block::{load, read_bytes, store, write_bytes, write_zeros};
    pub use check::check;
    pub use fat::{
        allocate, allocate_run, clear_slots, count_free, free_chain, get, get_copy, mirror, mkfs,
        next, read_fat, read_geometry, read_slot, run, set, slot_offset, walk, write_fs_info,
        write_slots,
    };
}

#[cfg(feature = "async")]
#[path = ""]
pub mod r#async {
    //! The asynchronous primitives.

    macro_rules! io_transform {
        ($($item:tt)*) => { $($item)* };
    }

    use hadris_storage::r#async as storage;

    #[path = "block.rs"]
    mod block;
    #[path = "check.rs"]
    mod check;
    #[path = "fat.rs"]
    mod fat;

    pub use block::{load, read_bytes, store, write_bytes, write_zeros};
    pub use check::check;
    pub use fat::{
        allocate, allocate_run, clear_slots, count_free, free_chain, get, get_copy, mirror, mkfs,
        next, read_fat, read_geometry, read_slot, run, set, slot_offset, walk, write_fs_info,
        write_slots,
    };
}

#[cfg(feature = "async-send")]
#[path = ""]
pub mod async_send {
    //! The asynchronous primitives with `Send` futures, for devices whose
    //! futures are `Send`.

    #[allow(unused_macros)]
    macro_rules! io_transform {
        ($($item:tt)*) => { hadris_macros::send_async! { $($item)* } };
    }

    use hadris_storage::async_send as storage;

    #[path = "block.rs"]
    mod block;
    #[path = "check.rs"]
    mod check;
    #[path = "fat.rs"]
    mod fat;

    pub use block::{load, read_bytes, store, write_bytes, write_zeros};
    pub use check::check;
    pub use fat::{
        allocate, allocate_run, clear_slots, count_free, free_chain, get, get_copy, mirror, mkfs,
        next, read_fat, read_geometry, read_slot, run, set, slot_offset, walk, write_fs_info,
        write_slots,
    };
}
